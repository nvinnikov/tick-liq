//! Integration tests for Phase 2 shadow gate.
//!
//! Requires TEST_DATABASE_URL env var pointing to a Postgres instance
//! where the tick-liq schema has been applied (run_migrations on startup).
//! Each test creates an isolated pool_address so tests are parallel-safe.
//!
//! Run with:
//!   TEST_DATABASE_URL=postgres://user:pass@localhost/tickliq \
//!     cargo test --test shadow_gate_integration -- --nocapture

use chrono::{Duration, Utc};
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_postgres::PgPool;
use tick_liq::storage::writer::{check_shadow_gate, GateStatus};
use uuid::Uuid;

async fn setup_pool() -> PgPool {
    let url = std::env::var("TEST_DATABASE_URL")
        .expect("TEST_DATABASE_URL must be set for integration tests");
    let pool = tick_liq::storage::connect(&url)
        .await
        .expect("connect test db");
    tick_liq::storage::run_migrations(&pool)
        .await
        .expect("run_migrations");
    pool
}

fn unique_pool() -> String {
    format!("test_pool_{}", Uuid::new_v4())
}

async fn insert_row(pool: &PgPool, pool_address: &str, age_days: i64, error_flag: bool) {
    let created_at = Utc::now() - Duration::days(age_days);
    pool.execute(
        query(
            r#"INSERT INTO shadow_rebalances
               (created_at, pool_address, trigger_reason, price, error_flag)
               VALUES ($1, $2, 'manual', 100.0, $3)"#,
        )
        .bind(created_at)
        .bind(pool_address)
        .bind(error_flag),
    )
    .await
    .expect("insert fixture row");
}

#[tokio::test]
async fn gate_no_data() {
    let pool = setup_pool().await;
    let addr = unique_pool();
    let status = check_shadow_gate(&pool, &addr).await.unwrap();
    assert!(matches!(status, GateStatus::NoData { .. }));
}

#[tokio::test]
async fn gate_too_recent() {
    let pool = setup_pool().await;
    let addr = unique_pool();
    insert_row(&pool, &addr, 1, false).await;
    let status = check_shadow_gate(&pool, &addr).await.unwrap();
    assert!(matches!(
        status,
        GateStatus::TooRecent {
            required_age_days: 14,
            ..
        }
    ));
}

#[tokio::test]
async fn gate_errors_present() {
    let pool = setup_pool().await;
    let addr = unique_pool();
    insert_row(&pool, &addr, 20, true).await;
    let status = check_shadow_gate(&pool, &addr).await.unwrap();
    assert!(matches!(status, GateStatus::ErrorsPresent { count: 1 }));
}

#[tokio::test]
async fn gate_pass() {
    let pool = setup_pool().await;
    let addr = unique_pool();
    insert_row(&pool, &addr, 20, false).await;
    insert_row(&pool, &addr, 5, false).await;
    let status = check_shadow_gate(&pool, &addr).await.unwrap();
    assert_eq!(status, GateStatus::Pass);
}

#[tokio::test]
async fn gate_per_pool_isolation() {
    let pool = setup_pool().await;
    let addr_a = unique_pool();
    let addr_b = unique_pool();
    insert_row(&pool, &addr_a, 20, false).await;
    assert_eq!(
        check_shadow_gate(&pool, &addr_a).await.unwrap(),
        GateStatus::Pass
    );
    assert!(matches!(
        check_shadow_gate(&pool, &addr_b).await.unwrap(),
        GateStatus::NoData { .. }
    ));
}
