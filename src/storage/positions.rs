use sqlx_postgres::PgPool;

/// Repository for the `positions` table.
///
/// Scaffold only — no writes yet.
pub struct PositionsRepo {
    pub pool: PgPool,
}

impl PositionsRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}
