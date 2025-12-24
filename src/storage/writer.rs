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
