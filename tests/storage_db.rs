//! Integration tests for the storage layer against a real Postgres+TimescaleDB.
//!
//! Run with:
//!   docker run -d --name tick-liq-db -e POSTGRES_PASSWORD=tickliq \
//!     -e POSTGRES_DB=tickliq -p 5432:5432 timescale/timescaledb:latest-pg16
//!   export DATABASE_URL=postgres://postgres:tickliq@localhost:5432/tickliq
//!   cargo test --features db-tests --test storage_db
//!
//! Skipped entirely unless the `db-tests` feature is enabled.

#![cfg(feature = "db-tests")]

use bigdecimal::BigDecimal;
use chrono::Utc;
use std::str::FromStr;
use tick_liq::storage::{self, events, pnl, positions, ticks};

async fn pool() -> sqlx::PgPool {
    let url = std::env::var("DATABASE_URL").expect("DATABASE_URL must be set for db-tests");
    let pool = storage::connect(&url, 4).await.expect("connect");
    storage::run_migrations(&pool).await.expect("migrations");
    pool
}

#[tokio::test]
async fn position_roundtrip() {
    let pool = pool().await;
    let mint = format!("mint-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let id = positions::insert(
        &pool,
        positions::NewPosition {
            mint: &mint,
            pool: "POOL_ADDR",
            owner: "OWNER",
            lower_tick: -100,
            upper_tick: 100,
        },
    )
    .await
    .expect("insert position");

    let fetched = positions::get_by_mint(&pool, &mint)
        .await
        .expect("get_by_mint")
        .expect("position exists");
    assert_eq!(fetched.id, id);
    assert_eq!(fetched.lower_tick, -100);
    assert!(fetched.closed_at.is_none());

    positions::mark_closed(&pool, id, Utc::now())
        .await
        .expect("mark_closed");
    let closed = positions::get_by_mint(&pool, &mint).await.unwrap().unwrap();
    assert!(closed.closed_at.is_some());
}

#[tokio::test]
async fn tick_and_pnl_and_event_roundtrip() {
    let pool = pool().await;
    let mint = format!("mint-{}", Utc::now().timestamp_nanos_opt().unwrap_or(0));
    let pos_id = positions::insert(
        &pool,
        positions::NewPosition {
            mint: &mint,
            pool: "POOL_X",
            owner: "OWNER",
            lower_tick: 0,
            upper_tick: 200,
        },
    )
    .await
    .unwrap();

    let now = Utc::now();
    ticks::insert(
        &pool,
        &ticks::PoolTick {
            pool: "POOL_X".into(),
            ts: now,
            sqrt_price: BigDecimal::from_str("12345678901234567890").unwrap(),
            tick: 42,
            liquidity: BigDecimal::from_str("999999999999").unwrap(),
        },
    )
    .await
    .unwrap();
    let latest = ticks::latest(&pool, "POOL_X").await.unwrap().unwrap();
    assert_eq!(latest.tick, 42);

    pnl::insert(
        &pool,
        &pnl::PnlSample {
            position_id: pos_id,
            ts: now,
            fees_x: BigDecimal::from(100),
            fees_y: BigDecimal::from(200),
            il_usd: -1.5,
            net_usd: 3.25,
        },
    )
    .await
    .unwrap();
    let latest_pnl = pnl::latest(&pool, pos_id).await.unwrap().unwrap();
    assert_eq!(latest_pnl.net_usd, 3.25);

    let ev_id = events::insert(
        &pool,
        events::NewRebalanceEvent {
            position_id: pos_id,
            old_range: (0, 200),
            new_range: (50, 250),
            reason: "out_of_range",
            tx_sig: "SIG_ABC",
        },
    )
    .await
    .unwrap();
    let by_sig = events::get_by_tx_sig(&pool, "SIG_ABC")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(by_sig.id, ev_id);
    assert_eq!(by_sig.reason, "out_of_range");
}
