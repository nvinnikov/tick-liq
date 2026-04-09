// Async writer for pool_ticks snapshots received from WebSocket.
// Not yet wired to the watch loop — ready for shadow mode integration.

#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_postgres::PgPool;

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

#[cfg(test)]
mod tests {
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
}
