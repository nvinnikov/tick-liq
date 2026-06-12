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
        .with_context(|| format!("failed to connect to database at {}", redact_db_url(db_url)))
}

/// Strip the password from a connection URL so it never reaches logs.
///
/// `postgres://user:secret@host/db` → `postgres://user:***@host/db`.
/// URLs without a userinfo section are returned unchanged.
fn redact_db_url(db_url: &str) -> String {
    let Some(scheme_end) = db_url.find("://") else {
        return db_url.to_string();
    };
    let rest = &db_url[scheme_end + 3..];
    let authority = &rest[..rest.find('/').unwrap_or(rest.len())];
    let Some(at) = authority.rfind('@') else {
        return db_url.to_string();
    };
    let userinfo = &rest[..at];
    match userinfo.find(':') {
        Some(colon) => format!(
            "{}://{}:***{}",
            &db_url[..scheme_end],
            &userinfo[..colon],
            &rest[at..]
        ),
        None => db_url.to_string(),
    }
}

/// Run the embedded schema against the given pool.
pub async fn run_migrations(pool: &PgPool) -> Result<()> {
    pool.execute(raw_sql(SCHEMA_SQL))
        .await
        .context("failed to execute schema.sql")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::redact_db_url;

    #[test]
    fn redacts_password() {
        assert_eq!(
            redact_db_url("postgres://tick:s3cret@localhost:5432/tick_liq"),
            "postgres://tick:***@localhost:5432/tick_liq"
        );
    }

    #[test]
    fn redacts_password_containing_at_sign() {
        assert_eq!(
            redact_db_url("postgres://tick:p@ss@db.internal/tick_liq"),
            "postgres://tick:***@db.internal/tick_liq"
        );
    }

    #[test]
    fn leaves_passwordless_url_unchanged() {
        assert_eq!(
            redact_db_url("postgres://tick@localhost/tick_liq"),
            "postgres://tick@localhost/tick_liq"
        );
        assert_eq!(
            redact_db_url("postgres://localhost/tick_liq"),
            "postgres://localhost/tick_liq"
        );
    }

    #[test]
    fn leaves_non_url_unchanged() {
        assert_eq!(redact_db_url("not a url"), "not a url");
    }
}
