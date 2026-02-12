// Async reader for pool_ticks rows, filtered by pool address and UTC date range.
// Used by the DB-mode backtest replay (BACKTEST-01).

use anyhow::{Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use sqlx_core::{executor::Executor, query::query, row::Row};
use sqlx_postgres::PgPool;

/// A single pool-state snapshot read back from the `pool_ticks` table.
/// Mirrors `storage::writer::PoolTick`, using the `time` column name
/// rather than `observed_at` (which is the DB column name).
///
/// Not all fields are consumed by every caller; `#[allow(dead_code)]` is
/// intentional — the struct is the full row contract, and future callers
/// (e.g. analytics, reporting) will use the remaining fields.
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub struct PoolTickRow {
    pub time: DateTime<Utc>,
    pub pool_address: String,
    pub slot: i64,
    pub tick_current: i32,
    /// Parsed from NUMERIC(80,0) stored as a decimal string.
    pub sqrt_price: u128,
    /// Parsed from NUMERIC(80,0) stored as a decimal string.
    pub liquidity: u128,
    /// Parsed from NUMERIC(80,0) stored as a decimal string.
    pub fee_growth_global_a: u128,
    /// Parsed from NUMERIC(80,0) stored as a decimal string.
    pub fee_growth_global_b: u128,
}

/// Read all `pool_ticks` rows for `pool_address` where `time >= from` (start of
/// UTC day) and `time < to` (start of UTC day), returned in chronological order.
///
/// # Security
/// `pool_address` is passed as a parameterised bind — no string interpolation
/// into SQL (T-03-01: SQL injection mitigation).
///
/// # Error handling
/// NUMERIC(80,0) columns are cast to `TEXT` in SQL and then parsed as `u128`.
/// If any row contains a value outside [0, u128::MAX] the function returns
/// `Err` with context identifying which column was malformed (T-03-04).
///
/// # Memory
/// All matching rows are fetched into memory at once. For typical operator
/// date ranges (days to weeks of data at one event/slot) this is acceptable;
/// the operator controls the range (T-03-02, accepted risk).
pub async fn read_ticks(
    pool: &PgPool,
    pool_address: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<PoolTickRow>> {
    // Construct UTC midnight timestamps for the inclusive start / exclusive end.
    let from_ts: DateTime<Utc> = from
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid")
        .and_utc();
    let to_ts: DateTime<Utc> = to
        .and_hms_opt(0, 0, 0)
        .expect("midnight is always valid")
        .and_utc();

    // NUMERIC(80,0) columns are cast to TEXT so we can parse them as u128
    // in Rust. sqlx-postgres has no native u128 codec, and this matches the
    // write path (writer.rs serialises u128 → decimal string → ::numeric).
    //
    // NOTE: anyhow::Context wraps DB errors with safe labels — raw sqlx
    // errors are not surfaced beyond local CLI logs (T-03-03).
    let rows = pool
        .fetch_all(
            query(
                "SELECT time, pool_address, slot, tick_current, \
                 sqrt_price::text AS sqrt_price, \
                 liquidity::text AS liquidity, \
                 fee_growth_global_a::text AS fee_growth_global_a, \
                 fee_growth_global_b::text AS fee_growth_global_b \
                 FROM pool_ticks \
                 WHERE pool_address = $1 AND time >= $2 AND time < $3 \
                 ORDER BY time ASC, slot ASC",
            )
            .bind(pool_address)
            .bind(from_ts)
            .bind(to_ts),
        )
        .await
        .context("query pool_ticks")?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
        let sqrt_price_str: String = r.try_get("sqrt_price")?;
        let liquidity_str: String = r.try_get("liquidity")?;
        let fga_str: String = r.try_get("fee_growth_global_a")?;
        let fgb_str: String = r.try_get("fee_growth_global_b")?;

        out.push(PoolTickRow {
            time: r.try_get("time")?,
            pool_address: r.try_get("pool_address")?,
            slot: r.try_get("slot")?,
            tick_current: r.try_get("tick_current")?,
            sqrt_price: sqrt_price_str.parse().context("parse sqrt_price as u128")?,
            liquidity: liquidity_str.parse().context("parse liquidity as u128")?,
            fee_growth_global_a: fga_str
                .parse()
                .context("parse fee_growth_global_a as u128")?,
            fee_growth_global_b: fgb_str
                .parse()
                .context("parse fee_growth_global_b as u128")?,
        });
    }

    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    // ---------------------------------------------------------------------------
    // Unit tests: pure Rust logic — no DB required
    // ---------------------------------------------------------------------------

    /// NUMERIC(80,0) values arrive from the DB as decimal strings.
    /// Verify that u128 parsing handles the full u128 range, including 2^64
    /// which is the expected sqrt_price boundary value used in the behaviour spec.
    #[test]
    fn parse_u128_from_decimal_string_two_pow_64() {
        let s = "18446744073709551616"; // 2^64
        let parsed: u128 = s.parse().expect("should parse");
        assert_eq!(parsed, 18446744073709551616u128);
    }

    #[test]
    fn parse_u128_from_decimal_string_max() {
        let s = u128::MAX.to_string();
        let parsed: u128 = s.parse().expect("should parse u128::MAX");
        assert_eq!(parsed, u128::MAX);
    }

    #[test]
    fn parse_u128_from_decimal_string_zero() {
        let s = "0";
        let parsed: u128 = s.parse().expect("should parse 0");
        assert_eq!(parsed, 0u128);
    }

    #[test]
    fn parse_u128_rejects_negative() {
        let s = "-1";
        assert!(
            s.parse::<u128>().is_err(),
            "negative should not parse as u128"
        );
    }

    /// Verify the UTC from/to timestamp derivation logic:
    /// - `from` = 2026-01-01 → from_ts = 2026-01-01T00:00:00Z
    /// - `to`   = 2026-01-02 → to_ts   = 2026-01-02T00:00:00Z
    #[test]
    fn from_to_dates_produce_utc_midnight_timestamps() {
        let from = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        let to = NaiveDate::from_ymd_opt(2026, 1, 2).unwrap();

        let from_ts: DateTime<Utc> = from.and_hms_opt(0, 0, 0).expect("midnight valid").and_utc();
        let to_ts: DateTime<Utc> = to.and_hms_opt(0, 0, 0).expect("midnight valid").and_utc();

        assert_eq!(from_ts.to_rfc3339(), "2026-01-01T00:00:00+00:00");
        assert_eq!(to_ts.to_rfc3339(), "2026-01-02T00:00:00+00:00");
        // to_ts > from_ts (exclusive upper bound is after inclusive lower bound)
        assert!(to_ts > from_ts);
    }

    // ---------------------------------------------------------------------------
    // Integration tests — require a live TimescaleDB instance.
    // Run with: DATABASE_URL=postgres://... cargo test -- --ignored
    // ---------------------------------------------------------------------------

    /// Full roundtrip: write a row via writer, read it back via read_ticks,
    /// verify field values including u128 round-trip for all NUMERIC columns.
    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn read_ticks_roundtrip() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }

    /// When no rows exist for the requested pool + date range, read_ticks
    /// returns Ok(vec![]) without error.
    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn read_ticks_empty_result_is_ok() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }

    /// Rows are returned in (time ASC, slot ASC) order regardless of
    /// insertion order.
    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn read_ticks_chronological_order() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }
}
