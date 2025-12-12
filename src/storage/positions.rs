// Writes are not wired into CLI yet — these methods are ready for shadow mode.

#![allow(dead_code)]

use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_core::query_scalar::query_scalar;
use sqlx_postgres::PgPool;

/// Repository for the `positions` table.
pub struct PositionsRepo {
    pub pool: PgPool,
}

/// Data required to open a new position record.
pub struct NewPosition<'a> {
    pub mint: &'a str,
    /// Protocol identifier: `"orca"` or `"raydium"`.
    pub protocol: &'a str,
    pub pool_address: &'a str,
    pub tick_lower: i32,
    pub tick_upper: i32,
    /// Entry price of token A denominated in token B. `None` if unknown.
    pub entry_price: Option<f64>,
}

impl PositionsRepo {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// Insert a new position row and return its generated `id`.
    ///
    /// If a position with the same `mint` already exists the existing row is
    /// left unchanged and its `id` is returned (ON CONFLICT DO NOTHING +
    /// a follow-up SELECT).
    ///
    /// Uses the non-macro `query` / `query_scalar` path so the crate
    /// compiles without a live `DATABASE_URL` at build time.
    pub async fn insert_position(&self, p: &NewPosition<'_>) -> anyhow::Result<i64> {
        // Try insert; ignore conflict on the UNIQUE mint column.
        self.pool
            .execute(
                query(
                    "INSERT INTO positions \
                     (mint, protocol, pool_address, tick_lower, tick_upper, entry_price) \
                     VALUES ($1, $2, $3, $4, $5, $6) \
                     ON CONFLICT (mint) DO NOTHING",
                )
                .bind(p.mint)
                .bind(p.protocol)
                .bind(p.pool_address)
                .bind(p.tick_lower)
                .bind(p.tick_upper)
                .bind(p.entry_price),
            )
            .await
            .map_err(|e| anyhow::anyhow!("insert_position failed: {e}"))?;

        // Retrieve the id whether we just inserted or the row pre-existed.
        let id: i64 = query_scalar("SELECT id FROM positions WHERE mint = $1")
            .bind(p.mint)
            .fetch_one(&self.pool)
            .await
            .map_err(|e| anyhow::anyhow!("insert_position id fetch failed: {e}"))?;

        Ok(id)
    }

    /// Append a P&L snapshot to `pnl_history` for the given position mint.
    ///
    /// TODO: Legacy signature kept for back-compat with callers that predate the
    /// PERSIST-02 schema update (fees_usd→fees_earned, net_usd→net_pnl, +pool_address,
    /// +position_value). New code should use `storage::writer::write_pnl_snapshot`
    /// and `storage::writer::spawn_pnl_write` instead.
    pub async fn record_pnl(
        &self,
        mint: &str,
        fees_usd: f64,
        il_usd: f64,
        net_usd: f64,
        price: f64,
    ) -> anyhow::Result<()> {
        self.pool
            .execute(
                query(
                    "INSERT INTO pnl_history \
                     (time, mint, pool_address, fees_earned, il_usd, net_pnl, position_value, price) \
                     VALUES (NOW(), $1, $2, $3, $4, $5, $6, $7)",
                )
                .bind(mint)
                .bind("")          // pool_address: unknown at legacy call sites
                .bind(fees_usd)    // fees_earned
                .bind(il_usd)
                .bind(net_usd)     // net_pnl
                .bind(0.0_f64)     // position_value: unknown at legacy call sites
                .bind(price),
            )
            .await
            .map_err(|e| anyhow::anyhow!("record_pnl failed: {e}"))?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    /// Integration tests require a live TimescaleDB instance.
    /// Run manually with: DATABASE_URL=postgres://... cargo test -- --ignored
    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn insert_position_roundtrip() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }

    #[tokio::test]
    #[ignore = "requires live PostgreSQL/TimescaleDB"]
    async fn record_pnl_appends_row() {
        // Placeholder — real test needs DATABASE_URL env var and a running DB.
    }
}
