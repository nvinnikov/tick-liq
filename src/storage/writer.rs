// Async writer for pool_ticks snapshots received from WebSocket.
// Wired into the watch loop in plan 01-03 — ready for shadow mode integration.

#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_postgres::PgPool;
use tokio::task::JoinHandle;
use tracing::warn;

/// A single pool-state snapshot captured from a WebSocket tick event.
#[derive(Debug, Clone)]
pub struct PoolTick {
    pub pool_address: String,
    pub slot: i64,
    pub tick_current: i32,
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub fee_growth_global_a: u128,
    pub fee_growth_global_b: u128,
    pub observed_at: DateTime<Utc>,
}

/// Insert one `PoolTick` snapshot into `pool_ticks`.
///
/// If a row with the same `(pool_address, slot)` already exists the insert is
/// silently skipped — the database UNIQUE constraint handles idempotency so
/// reconnects / duplicate deliveries are safe.
///
/// Uses the non-macro `query` path so the crate compiles without a live
/// `DATABASE_URL` at build time (matching the pattern in `positions.rs`).
pub async fn write_pool_tick(pool: &PgPool, tick: &PoolTick) -> Result<()> {
    // u128 values are serialised as decimal strings; Postgres casts them via
    // the explicit `::numeric` in the SQL, matching NUMERIC(80,0) columns.
    let sp = tick.sqrt_price.to_string();
    let liq = tick.liquidity.to_string();
    let fga = tick.fee_growth_global_a.to_string();
    let fgb = tick.fee_growth_global_b.to_string();

    pool.execute(
        query(
            "INSERT INTO pool_ticks \
             (time, pool_address, slot, tick_current, sqrt_price, liquidity, \
              fee_growth_global_a, fee_growth_global_b) \
             VALUES ($1, $2, $3, $4, $5::numeric, $6::numeric, $7::numeric, $8::numeric) \
             ON CONFLICT (pool_address, slot) DO NOTHING",
        )
        .bind(tick.observed_at)
        .bind(&tick.pool_address)
        .bind(tick.slot)
        .bind(tick.tick_current)
        .bind(sp)
        .bind(liq)
        .bind(fga)
        .bind(fgb),
    )
    .await
    .context("write_pool_tick failed")?;

    Ok(())
}

/// A P&L snapshot captured after each tick event, recording fee income,
/// impermanent loss, net profit, and current position value.
#[derive(Debug, Clone)]
pub struct PnlSnapshot {
    pub mint: String,
    pub pool_address: String,
    pub fees_earned: f64,
    pub il_usd: f64,
    pub net_pnl: f64,
    pub position_value: f64,
    pub price: f64,
    pub observed_at: DateTime<Utc>,
}

/// Insert one `PnlSnapshot` row into `pnl_history`.
///
/// Uses the non-macro `query` path so the crate compiles without a live
/// `DATABASE_URL` at build time. All values are bound as parameters — no
/// string interpolation — satisfying T-01-07 (SQL injection via mint/pool_address).
pub async fn write_pnl_snapshot(pool: &PgPool, snap: &PnlSnapshot) -> Result<()> {
    pool.execute(
        query(
            "INSERT INTO pnl_history \
             (time, mint, pool_address, fees_earned, il_usd, net_pnl, position_value, price) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(snap.observed_at)
        .bind(&snap.mint)
        .bind(&snap.pool_address)
        .bind(snap.fees_earned)
        .bind(snap.il_usd)
        .bind(snap.net_pnl)
        .bind(snap.position_value)
        .bind(snap.price),
    )
    .await
    .context("write_pnl_snapshot failed")?;
    Ok(())
}

/// Fire-and-forget variant of `write_pnl_snapshot`.
///
/// Spawns a Tokio task and returns its `JoinHandle` immediately — the caller
/// is never blocked by DB I/O (PERSIST-03). Failures are logged via
/// `tracing::warn!` with the position mint for traceability (T-01-06).
pub fn spawn_pnl_write(pool: PgPool, snap: PnlSnapshot) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = write_pnl_snapshot(&pool, &snap).await {
            warn!(error = %e, mint = %snap.mint, "pnl write failed");
        }
    })
}

/// A shadow rebalance decision row written on every rebalance trigger (SHADOW-02).
///
/// `error_flag = true` captures any `anyhow::Error` from the decision path so the
/// Plan 03 gate query can detect bad runs (T-02-05).
#[derive(Debug, Clone)]
pub struct ShadowRebalanceRow {
    pub pool_address: String,
    /// 'out_of_range' | 'near_lower_edge' | 'near_upper_edge' | 'il_threshold' | 'manual' | 'error'
    pub trigger_reason: String,
    pub price: f64,
    pub simulated_range_width: Option<f64>,
    pub simulated_fees_earned: Option<f64>,
    pub simulated_il_usd: Option<f64>,
    pub simulated_net_pnl: Option<f64>,
    pub error_flag: bool,
    pub error_message: Option<String>,
}

/// Insert one `ShadowRebalanceRow` into `shadow_rebalances`.
///
/// All values are bound as parameters — no string interpolation — satisfying SQL
/// injection requirements. The `created_at` timestamp is generated server-side via
/// `DEFAULT NOW()` (T-02-04: no client-supplied id or timestamp).
pub async fn write_shadow_rebalance(pool: &PgPool, row: &ShadowRebalanceRow) -> Result<()> {
    pool.execute(
        query(
            "INSERT INTO shadow_rebalances \
             (pool_address, trigger_reason, price, \
              simulated_range_width, simulated_fees_earned, \
              simulated_il_usd, simulated_net_pnl, \
              error_flag, error_message) \
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9)",
        )
        .bind(&row.pool_address)
        .bind(&row.trigger_reason)
        .bind(row.price)
        .bind(row.simulated_range_width)
        .bind(row.simulated_fees_earned)
        .bind(row.simulated_il_usd)
        .bind(row.simulated_net_pnl)
        .bind(row.error_flag)
        .bind(row.error_message.as_deref()),
    )
    .await
    .context("write_shadow_rebalance failed")?;
    Ok(())
}

/// Log a rebalance skip due to approval timeout or rejection (TG-02).
///
/// Writes a row to `shadow_rebalances` with trigger_reason prefixed by
/// `approval_` (e.g., `approval_timeout`, `approval_rejected`) and
/// `error_flag = false` — a skip is expected behaviour, not an error.
/// All values are bound as parameters (T-07-06: no SQL injection via message content).
pub async fn write_approval_skip(
    pool: &PgPool,
    pool_address: &str,
    reason: &str,
    price: f64,
) -> Result<()> {
    pool.execute(
        query(
            "INSERT INTO shadow_rebalances \
             (pool_address, trigger_reason, price, error_flag, error_message) \
             VALUES ($1, $2, $3, FALSE, $4)",
        )
        .bind(pool_address)
        .bind(format!("approval_{}", reason)) // 'approval_timeout' or 'approval_rejected'
        .bind(price)
        .bind(format!("Rebalance skipped: {}", reason)),
    )
    .await
    .context("write_approval_skip failed")?;
    Ok(())
}

/// Fire-and-forget variant of `write_shadow_rebalance`, mirroring `spawn_pnl_write`.
///
/// Spawns a Tokio task immediately — caller is never blocked by DB I/O.
/// Errors are logged via `tracing::error!` with pool address for traceability.
pub fn spawn_shadow_write(pool: PgPool, row: ShadowRebalanceRow) {
    tokio::spawn(async move {
        if let Err(e) = write_shadow_rebalance(&pool, &row).await {
            tracing::error!(
                error = %e,
                pool = %row.pool_address,
                "failed to write shadow_rebalances row"
            );
        }
    });
}

/// Number of days of shadow data required before --live is allowed.
pub const SHADOW_GATE_REQUIRED_DAYS: i64 = 14;

/// Result of the DB gate check that guards `--live` mode startup.
#[derive(Debug, Clone, PartialEq)]
pub enum GateStatus {
    /// All conditions met — safe to enter live mode.
    Pass,
    /// No shadow_rebalances rows exist for this pool.
    NoData { pool_address: String },
    /// Earliest row is too recent; need at least `required_age_days` days.
    TooRecent { earliest: DateTime<Utc>, required_age_days: i64 },
    /// One or more rows have `error_flag = true`.
    ErrorsPresent { count: i64 },
}

impl GateStatus {
    pub fn is_pass(&self) -> bool {
        matches!(self, GateStatus::Pass)
    }

    pub fn describe(&self) -> String {
        match self {
            GateStatus::Pass => "shadow gate PASSED".to_string(),
            GateStatus::NoData { pool_address } => format!(
                "shadow gate FAILED: no shadow_rebalances rows for pool {}",
                pool_address
            ),
            GateStatus::TooRecent { earliest, required_age_days } => format!(
                "shadow gate FAILED: earliest shadow row is {}, requires {} days of runtime before --live",
                earliest.to_rfc3339(),
                required_age_days
            ),
            GateStatus::ErrorsPresent { count } => format!(
                "shadow gate FAILED: {} error-flagged shadow_rebalances rows must be zero",
                count
            ),
        }
    }
}

/// Check the DB gate conditions for allowing `--live` mode.
///
/// Checks in order:
/// 1. NoData — if no rows exist for the pool
/// 2. TooRecent — if the earliest row is less than SHADOW_GATE_REQUIRED_DAYS old
/// 3. ErrorsPresent — if any row has error_flag = true
/// 4. Pass — all conditions satisfied
///
/// Uses parameterised queries (T-02-08: no override path).
pub async fn check_shadow_gate(
    pool: &PgPool,
    pool_address: &str,
) -> anyhow::Result<GateStatus> {
    use sqlx_core::query_scalar::query_scalar;

    // Step 1: check for any rows at all
    let earliest: Option<DateTime<Utc>> = query_scalar(
        "SELECT MIN(created_at) FROM shadow_rebalances WHERE pool_address = $1",
    )
    .bind(pool_address)
    .fetch_one(pool)
    .await
    .context("check_shadow_gate: MIN(created_at) query failed")?;

    let earliest = match earliest {
        None => {
            return Ok(GateStatus::NoData {
                pool_address: pool_address.to_string(),
            })
        }
        Some(ts) => ts,
    };

    // Step 2: check age requirement
    let required = chrono::Duration::days(SHADOW_GATE_REQUIRED_DAYS);
    if Utc::now() - earliest < required {
        return Ok(GateStatus::TooRecent {
            earliest,
            required_age_days: SHADOW_GATE_REQUIRED_DAYS,
        });
    }

    // Step 3: check for error rows
    let error_count: i64 = query_scalar(
        "SELECT COUNT(*) FROM shadow_rebalances WHERE pool_address = $1 AND error_flag = true",
    )
    .bind(pool_address)
    .fetch_one(pool)
    .await
    .context("check_shadow_gate: error_flag count query failed")?;

    if error_count > 0 {
        return Ok(GateStatus::ErrorsPresent { count: error_count });
    }

    Ok(GateStatus::Pass)
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    #[test]
    fn describe_failures_are_actionable() {
        let s = GateStatus::NoData {
            pool_address: "abc".into(),
        };
        assert!(
            s.describe().contains("no shadow_rebalances rows"),
            "NoData describe should mention 'no shadow_rebalances rows'"
        );

        let s = GateStatus::ErrorsPresent { count: 3 };
        assert!(
            s.describe().contains("3 error-flagged"),
            "ErrorsPresent describe should mention count"
        );
    }

    #[test]
    fn gate_status_is_pass_predicate() {
        assert!(GateStatus::Pass.is_pass());
        assert!(!GateStatus::NoData { pool_address: "x".into() }.is_pass());
        assert!(
            !GateStatus::TooRecent {
                earliest: Utc::now(),
                required_age_days: 14,
            }
            .is_pass()
        );
        assert!(!GateStatus::ErrorsPresent { count: 1 }.is_pass());
    }

    #[test]
    fn too_recent_describe_contains_rfc3339_and_days() {
        let ts = chrono::DateTime::parse_from_rfc3339("2026-03-01T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        let s = GateStatus::TooRecent {
            earliest: ts,
            required_age_days: 14,
        };
        let desc = s.describe();
        assert!(desc.contains("14"), "should mention required days");
        assert!(desc.contains("2026-03-01"), "should contain date");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    /// Integration tests require a live TimescaleDB instance.
    /// Run manually with: DATABASE_URL=postgres://... cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn write_pool_tick_roundtrip() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }

    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn write_pool_tick_idempotent() {
        // Verify second insert with same (pool_address, slot) does not error
        // or duplicate the row (ON CONFLICT DO NOTHING).
    }

    /// Verify PnlSnapshot can be constructed and cloned without panicking.
    #[test]
    fn pnl_snapshot_fields_accessible() {
        let snap = PnlSnapshot {
            mint: "mint123".to_string(),
            pool_address: "pool456".to_string(),
            fees_earned: 1.5,
            il_usd: -0.3,
            net_pnl: 1.2,
            position_value: 1000.0,
            price: 42.0,
            observed_at: Utc::now(),
        };
        let cloned = snap.clone();
        assert_eq!(cloned.mint, "mint123");
        assert_eq!(cloned.pool_address, "pool456");
        assert!((cloned.fees_earned - 1.5).abs() < f64::EPSILON);
        assert!((cloned.net_pnl - 1.2).abs() < f64::EPSILON);
        assert!((cloned.position_value - 1000.0).abs() < f64::EPSILON);
    }

    /// Verify spawn_pnl_write returns a JoinHandle without blocking.
    /// The task will fail (no DB), but the spawn itself must not panic.
    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn spawn_pnl_write_roundtrip() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }
}
