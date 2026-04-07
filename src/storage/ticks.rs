//! Pool tick repository — append/query pool snapshots from the
//! `pool_ticks` TimescaleDB hypertable.

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, FromRow)]
pub struct PoolTick {
    pub pool: String,
    pub ts: DateTime<Utc>,
    pub sqrt_price: BigDecimal,
    pub tick: i32,
    pub liquidity: BigDecimal,
}

/// Insert a single pool snapshot. Idempotent on (pool, ts) primary key:
/// callers should treat duplicates as a no-op via `ON CONFLICT DO NOTHING`.
pub async fn insert(pool: &PgPool, t: &PoolTick) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO pool_ticks (pool, ts, sqrt_price, tick, liquidity)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (pool, ts) DO NOTHING
        "#,
    )
    .bind(&t.pool)
    .bind(t.ts)
    .bind(&t.sqrt_price)
    .bind(t.tick)
    .bind(&t.liquidity)
    .execute(pool)
    .await?;
    Ok(())
}

/// Latest snapshot for a pool, if any.
pub async fn latest(pool: &PgPool, pool_addr: &str) -> Result<Option<PoolTick>, sqlx::Error> {
    sqlx::query_as::<_, PoolTick>(
        r#"
        SELECT pool, ts, sqrt_price, tick, liquidity
        FROM pool_ticks
        WHERE pool = $1
        ORDER BY ts DESC
        LIMIT 1
        "#,
    )
    .bind(pool_addr)
    .fetch_optional(pool)
    .await
}

/// All snapshots for a pool in `[from, to]`, ascending by `ts`.
pub async fn range(
    pool: &PgPool,
    pool_addr: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<PoolTick>, sqlx::Error> {
    sqlx::query_as::<_, PoolTick>(
        r#"
        SELECT pool, ts, sqrt_price, tick, liquidity
        FROM pool_ticks
        WHERE pool = $1 AND ts BETWEEN $2 AND $3
        ORDER BY ts ASC
        "#,
    )
    .bind(pool_addr)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
}
