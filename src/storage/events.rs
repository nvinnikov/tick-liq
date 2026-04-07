//! Rebalance event repository — append/query the audit log of
//! rebalance actions taken on each position.

use chrono::{DateTime, Utc};
use sqlx::postgres::types::PgRange;
use sqlx::{FromRow, PgPool};
use std::ops::Bound;

#[derive(Debug, Clone, FromRow)]
pub struct RebalanceEvent {
    pub id: i64,
    pub position_id: i64,
    pub ts: DateTime<Utc>,
    pub old_range: PgRange<i32>,
    pub new_range: PgRange<i32>,
    pub reason: String,
    pub tx_sig: String,
}

#[derive(Debug, Clone)]
pub struct NewRebalanceEvent<'a> {
    pub position_id: i64,
    pub old_range: (i32, i32),
    pub new_range: (i32, i32),
    pub reason: &'a str,
    pub tx_sig: &'a str,
}

/// Append a rebalance event, returning its assigned id.
pub async fn insert(pool: &PgPool, ev: NewRebalanceEvent<'_>) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO rebalance_events (position_id, old_range, new_range, reason, tx_sig)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(ev.position_id)
    .bind(to_pg_range(ev.old_range))
    .bind(to_pg_range(ev.new_range))
    .bind(ev.reason)
    .bind(ev.tx_sig)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// All rebalance events for a position, most recent first.
pub async fn list_for_position(
    pool: &PgPool,
    position_id: i64,
) -> Result<Vec<RebalanceEvent>, sqlx::Error> {
    sqlx::query_as::<_, RebalanceEvent>(
        r#"
        SELECT id, position_id, ts, old_range, new_range, reason, tx_sig
        FROM rebalance_events
        WHERE position_id = $1
        ORDER BY ts DESC
        "#,
    )
    .bind(position_id)
    .fetch_all(pool)
    .await
}

/// Look up a rebalance event by its on-chain transaction signature.
pub async fn get_by_tx_sig(
    pool: &PgPool,
    tx_sig: &str,
) -> Result<Option<RebalanceEvent>, sqlx::Error> {
    sqlx::query_as::<_, RebalanceEvent>(
        r#"
        SELECT id, position_id, ts, old_range, new_range, reason, tx_sig
        FROM rebalance_events
        WHERE tx_sig = $1
        "#,
    )
    .bind(tx_sig)
    .fetch_optional(pool)
    .await
}

fn to_pg_range((lo, hi): (i32, i32)) -> PgRange<i32> {
    PgRange {
        start: Bound::Included(lo),
        end: Bound::Excluded(hi),
    }
}
