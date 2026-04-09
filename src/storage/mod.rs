pub mod positions;
pub mod tick_reader;
pub mod writer;

use anyhow::{Context, Result};
use sqlx_core::executor::Executor;
use sqlx_core::raw_sql::raw_sql;
use sqlx_postgres::{PgPool, PgPoolOptions};

pub const SCHEMA_SQL: &str = include_str!("schema.sql");

/// Connect to Postgres using the given URL.
pub async fn connect(db_url: &str) -> Result<PgPool> {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(db_url)
        .await
        .with_context(|| format!("failed to connect to database at {}", db_url))
}

/// Run the embedded schema against the given pool.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    pool.execute(raw_sql(SCHEMA_SQL))
        .await
        .context("failed to execute schema.sql")?;
    Ok(())
}
