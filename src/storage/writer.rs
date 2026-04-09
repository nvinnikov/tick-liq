// storage::writer — async pool_ticks and pnl_history writers.
//
// Uses the non-macro sqlx path (sqlx_core::query::query) so the crate builds
// without a DATABASE_URL at compile time.

#![allow(dead_code)]

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_postgres::PgPool;
use tokio::task::JoinHandle;
use tracing::warn;

// ---------------------------------------------------------------------------
// pool_ticks
// ---------------------------------------------------------------------------

/// One tick-level snapshot of a CLMM pool state.
///
/// `sqrt_price`, `liquidity`, `fee_growth_global_a/b` are stored as `u128`
/// in Rust and serialised to NUMERIC(80,0) strings for Postgres (Solana values
/// fit in 128 bits; NUMERIC avoids lossy f64 conversion).
#[derive(Debug, Clone)]
pub struct PoolTick {
    pub pool_address: String,
    /// Solana slot at which this snapshot was observed.
    pub slot: i64,
    pub tick_current: i32,
    pub sqrt_price: u128,
    pub liquidity: u128,
    pub fee_growth_global_a: u128,
    pub fee_growth_global_b: u128,
    pub observed_at: DateTime<Utc>,
}

/// Insert a pool tick snapshot into `pool_ticks`.
///
/// If a row with the same `(pool_address, slot)` already exists the insert is
/// silently ignored (`ON CONFLICT DO NOTHING`) — this makes the function safe
/// to call on WebSocket reconnects that replay the same slot.
pub async fn write_pool_tick(pool: &PgPool, tick: &PoolTick) -> Result<()> {
    // u128 → decimal string; NUMERIC(80,0) accepts a text cast in Postgres.
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

// ---------------------------------------------------------------------------
// pnl_history
// ---------------------------------------------------------------------------

/// One P&L snapshot for an active position.
///
/// Phase-2 will populate the strategy-derived fields. During shadow mode the
/// fee/IL/pnl fields are placeholder zeros — see TODO comments in the watch loop.
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

/// Insert a P&L snapshot into `pnl_history`.
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

/// Fire-and-forget variant: spawns a tokio task and returns immediately.
///
/// The watch tick loop uses this so P&L I/O never blocks tick processing
/// (PERSIST-03). Errors are logged via `tracing::warn!` and not propagated.
pub fn spawn_pnl_write(pool: PgPool, snap: PnlSnapshot) -> JoinHandle<()> {
    tokio::spawn(async move {
        if let Err(e) = write_pnl_snapshot(&pool, &snap).await {
            warn!(error = %e, mint = %snap.mint, "pnl_history write failed");
        }
    })
}
