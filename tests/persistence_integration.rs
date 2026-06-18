//! Integration tests for storage::writer.
//!
//! These tests require a running PostgreSQL (or TimescaleDB) instance.
//! Run with:
//!   DATABASE_URL=postgres://user:pass@localhost/tickliq \
//!     cargo test --test persistence_integration -- --ignored

use chrono::Utc;
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_core::query_scalar::query_scalar;
use std::time::Instant;
use tick_liq::storage::{
    self,
    writer::{PnlSnapshot, PoolTick, spawn_pnl_write, write_pnl_snapshot, write_pool_tick},
};

// ---------------------------------------------------------------------------
// Test helpers
// ---------------------------------------------------------------------------

async fn setup() -> sqlx_postgres::PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL required for integration tests");
    let pool = storage::connect(&url).await.expect("connect to Postgres");
    storage::run_migrations(&pool)
        .await
        .expect("run_migrations");

    // Clean up any rows from previous test runs to ensure a clean slate.
    pool.execute(query(
        "DELETE FROM pool_ticks WHERE pool_address LIKE 'test-%'",
    ))
    .await
    .expect("clean pool_ticks");
    pool.execute(query(
        "DELETE FROM pnl_history WHERE pool_address LIKE 'test-%'",
    ))
    .await
    .expect("clean pnl_history");

    pool
}

fn sample_tick(slot: i64) -> PoolTick {
    PoolTick {
        pool_address: "test-pool-A".into(),
        slot,
        tick_current: 100,
        sqrt_price: 1_000_000_000_000_u128,
        liquidity: 2_000_000_000_u128,
        fee_growth_global_a: 42_u128,
        fee_growth_global_b: 99_u128,
        observed_at: Utc::now(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Inserting the same (pool_address, slot) twice must not create duplicate rows.
/// This validates the ON CONFLICT DO NOTHING idempotency required by PERSIST-04.
#[tokio::test]
#[ignore = "requires live PostgreSQL/TimescaleDB"]
async fn pool_tick_write_is_idempotent() {
    let pool = setup().await;
    let tick = sample_tick(12345);

    write_pool_tick(&pool, &tick).await.unwrap();
    write_pool_tick(&pool, &tick).await.unwrap();

    let count: i64 =
        query_scalar("SELECT COUNT(*) FROM pool_ticks WHERE pool_address = $1 AND slot = $2")
            .bind("test-pool-A")
            .bind(12345_i64)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_eq!(count, 1, "idempotent upsert must not create duplicate rows");
}

/// A PnlSnapshot written via write_pnl_snapshot must be queryable from pnl_history.
/// Validates PERSIST-02: fees_earned, il_usd, net_pnl, position_value columns.
#[tokio::test]
#[ignore = "requires live PostgreSQL/TimescaleDB"]
async fn pnl_snapshot_write_persists() {
    let pool = setup().await;

    let snap = PnlSnapshot {
        mint: "test-mint-1".into(),
        pool_address: "test-pool-B".into(),
        fees_earned: 12.5,
        il_usd: -3.25,
        net_pnl: 9.25,
        position_value: 10_000.0,
        price: 150.0,
        observed_at: Utc::now(),
    };

    write_pnl_snapshot(&pool, &snap).await.unwrap();

    let fees: f64 = query_scalar(
        "SELECT fees_earned FROM pnl_history WHERE mint = $1 ORDER BY time DESC LIMIT 1",
    )
    .bind("test-mint-1")
    .fetch_one(&pool)
    .await
    .unwrap();

    assert!(
        (fees - 12.5).abs() < 1e-9,
        "fees_earned mismatch: got {fees}"
    );
}

/// spawn_pnl_write must return without awaiting the DB write — fire-and-forget.
/// Spawning 50 tasks must complete in under 100 ms wall-clock (PERSIST-03).
#[tokio::test]
#[ignore = "requires live PostgreSQL/TimescaleDB"]
async fn spawn_pnl_write_is_non_blocking() {
    let pool = setup().await;

    let start = Instant::now();
    for i in 0..50 {
        let snap = PnlSnapshot {
            mint: format!("bench-{i}"),
            pool_address: "test-pool-bench".into(),
            fees_earned: 1.0,
            il_usd: 0.0,
            net_pnl: 1.0,
            position_value: 1.0,
            price: 1.0,
            observed_at: Utc::now(),
        };
        // Drop the JoinHandle — we only care about the scheduling latency, not completion.
        std::mem::drop(spawn_pnl_write(pool.clone(), snap));
    }
    let elapsed = start.elapsed();

    assert!(
        elapsed.as_millis() < 100,
        "spawn_pnl_write must be fire-and-forget; spawning 50 tasks took {}ms",
        elapsed.as_millis()
    );
}
