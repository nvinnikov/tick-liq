//! P&L history repository — append/query per-position P&L samples from
//! the `pnl_history` TimescaleDB hypertable.

use bigdecimal::BigDecimal;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, FromRow)]
pub struct PnlSample {
    pub position_id: i64,
    pub ts: DateTime<Utc>,
    pub fees_x: BigDecimal,
    pub fees_y: BigDecimal,
    pub il_usd: f64,
    pub net_usd: f64,
}

/// Append a single P&L sample. Idempotent on (position_id, ts).
pub async fn insert(pool: &PgPool, s: &PnlSample) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO pnl_history (position_id, ts, fees_x, fees_y, il_usd, net_usd)
        VALUES ($1, $2, $3, $4, $5, $6)
        ON CONFLICT (position_id, ts) DO NOTHING
        "#,
    )
    .bind(s.position_id)
    .bind(s.ts)
    .bind(&s.fees_x)
    .bind(&s.fees_y)
    .bind(s.il_usd)
    .bind(s.net_usd)
    .execute(pool)
    .await?;
    Ok(())
}

/// Latest sample for a position, if any.
pub async fn latest(pool: &PgPool, position_id: i64) -> Result<Option<PnlSample>, sqlx::Error> {
    sqlx::query_as::<_, PnlSample>(
        r#"
        SELECT position_id, ts, fees_x, fees_y, il_usd, net_usd
        FROM pnl_history
        WHERE position_id = $1
        ORDER BY ts DESC
        LIMIT 1
        "#,
    )
    .bind(position_id)
    .fetch_optional(pool)
    .await
}

/// All samples for a position in `[from, to]`, ascending by `ts`.
pub async fn range(
    pool: &PgPool,
    position_id: i64,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<Vec<PnlSample>, sqlx::Error> {
    sqlx::query_as::<_, PnlSample>(
        r#"
        SELECT position_id, ts, fees_x, fees_y, il_usd, net_usd
        FROM pnl_history
        WHERE position_id = $1 AND ts BETWEEN $2 AND $3
        ORDER BY ts ASC
        "#,
    )
    .bind(position_id)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
}
