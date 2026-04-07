//! Position repository — CRUD for tracked LP positions.

use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};

#[derive(Debug, Clone, FromRow)]
pub struct Position {
    pub id: i64,
    pub mint: String,
    pub pool: String,
    pub owner: String,
    pub lower_tick: i32,
    pub upper_tick: i32,
    pub opened_at: DateTime<Utc>,
    pub closed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewPosition<'a> {
    pub mint: &'a str,
    pub pool: &'a str,
    pub owner: &'a str,
    pub lower_tick: i32,
    pub upper_tick: i32,
}

/// Insert a new position, returning its assigned id.
///
/// Fails if a position with the same `mint` already exists.
pub async fn insert(pool: &PgPool, p: NewPosition<'_>) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO positions (mint, pool, owner, lower_tick, upper_tick)
        VALUES ($1, $2, $3, $4, $5)
        RETURNING id
        "#,
    )
    .bind(p.mint)
    .bind(p.pool)
    .bind(p.owner)
    .bind(p.lower_tick)
    .bind(p.upper_tick)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

/// Look up a position by its NFT mint.
pub async fn get_by_mint(pool: &PgPool, mint: &str) -> Result<Option<Position>, sqlx::Error> {
    sqlx::query_as::<_, Position>(
        r#"
        SELECT id, mint, pool, owner, lower_tick, upper_tick, opened_at, closed_at
        FROM positions
        WHERE mint = $1
        "#,
    )
    .bind(mint)
    .fetch_optional(pool)
    .await
}

/// All currently-open positions for a given owner.
pub async fn list_open_by_owner(pool: &PgPool, owner: &str) -> Result<Vec<Position>, sqlx::Error> {
    sqlx::query_as::<_, Position>(
        r#"
        SELECT id, mint, pool, owner, lower_tick, upper_tick, opened_at, closed_at
        FROM positions
        WHERE owner = $1 AND closed_at IS NULL
        ORDER BY opened_at DESC
        "#,
    )
    .bind(owner)
    .fetch_all(pool)
    .await
}

/// Mark a position as closed at `closed_at`.
pub async fn mark_closed(
    pool: &PgPool,
    id: i64,
    closed_at: DateTime<Utc>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        UPDATE positions
        SET closed_at = $2
        WHERE id = $1 AND closed_at IS NULL
        "#,
    )
    .bind(id)
    .bind(closed_at)
    .execute(pool)
    .await?;
    Ok(())
}
