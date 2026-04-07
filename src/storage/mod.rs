//! Storage layer: PostgreSQL + TimescaleDB persistence for positions,
//! tick history, P&L time series, and rebalance audit events.
//!
//! All repositories operate on a [`sqlx::PgPool`] passed in by the caller —
//! pool construction and migration application live in [`connect`] and
//! [`run_migrations`] respectively.
//!
//! # Note on `sqlx::query!` vs runtime-checked queries
//!
//! Task #18 originally specified `sqlx::query!` macros in offline mode, which
//! requires running `cargo sqlx prepare` against a live database and
//! committing the resulting `.sqlx/` metadata. No Postgres was available in
//! the dev environment when this layer was first written, so it uses
//! runtime-checked `sqlx::query_as` and `sqlx::query` instead. A follow-up
//! can swap them in once a DB is reachable.
//!
//! # Note on the sqlx version pin
//!
//! sqlx is pinned to 0.6 (not 0.7+). 0.7+ unconditionally depends on
//! `sqlx-mysql`, whose `rsa → zeroize >=1.5` chain conflicts with
//! `solana-client 1.18`'s `curve25519-dalek` (which needs `zeroize <1.4`).
//! 0.6 keeps mysql behind a feature flag, so `default-features = false`
//! drops it cleanly.

use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Error as SqlxError;

pub mod events;
pub mod pnl;
pub mod positions;
pub mod ticks;

/// Open a connection pool against `database_url` (e.g.
/// `postgres://user:pass@host:5432/db`).
pub async fn connect(database_url: &str, max_connections: u32) -> Result<PgPool, SqlxError> {
    PgPoolOptions::new()
        .max_connections(max_connections)
        .connect(database_url)
        .await
}

/// Apply all pending sqlx migrations from the `migrations/` directory.
pub async fn run_migrations(pool: &PgPool) -> Result<(), sqlx::migrate::MigrateError> {
    sqlx::migrate!("./migrations").run(pool).await
}
