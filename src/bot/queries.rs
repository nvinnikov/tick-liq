use anyhow::{Context, Result};
use sqlx_core::executor::Executor;
use sqlx_core::query::query;
use sqlx_core::row::Row;
use sqlx_postgres::PgPool;

/// Status data for /status command (TG-03).
pub struct StatusData {
    pub pool_address: String,
    pub position_value: f64,
    pub price: f64,
    pub fees_earned: f64,
    pub il_usd: f64,
    pub net_pnl: f64,
    pub drawdown_pct: f64,
    pub pause_flag: bool,
    pub halt_flag: bool,
    pub operator_pause: bool,
    pub peak_pnl: f64,
}

/// Query latest P&L snapshot + risk state for /status (TG-03).
///
/// Returns `Ok(None)` when no `pnl_history` row exists yet (fresh DB, /status
/// before the first tick) — that is a normal state, not an error. `try_get`
/// is used throughout so a NULL or schema-type mismatch surfaces as an
/// `anyhow::Err` instead of panicking (no-panic invariant).
pub async fn query_status(pool: &PgPool, pool_address: &str) -> Result<Option<StatusData>> {
    // Latest pnl_history row for this pool.
    let Some(pnl_row) = pool
        .fetch_optional(
            query(
                "SELECT position_value, price, fees_earned, il_usd, net_pnl \
                 FROM pnl_history WHERE pool_address = $1 \
                 ORDER BY time DESC LIMIT 1",
            )
            .bind(pool_address),
        )
        .await
        .context("query_status: pnl_history query failed")?
    else {
        return Ok(None);
    };

    // Risk state (load_or_init inserts this at watch start; absence is still
    // treated as "no data yet" rather than an error).
    let Some(risk_row) = pool
        .fetch_optional(
            query(
                "SELECT peak_pnl, current_drawdown_pct, pause_flag, halt_flag, operator_pause \
                 FROM risk_state WHERE pool_address = $1",
            )
            .bind(pool_address),
        )
        .await
        .context("query_status: risk_state query failed")?
    else {
        return Ok(None);
    };

    Ok(Some(StatusData {
        pool_address: pool_address.to_string(),
        position_value: pnl_row.try_get("position_value")?,
        price: pnl_row.try_get("price")?,
        fees_earned: pnl_row.try_get("fees_earned")?,
        il_usd: pnl_row.try_get("il_usd")?,
        net_pnl: pnl_row.try_get("net_pnl")?,
        drawdown_pct: risk_row.try_get("current_drawdown_pct")?,
        pause_flag: risk_row.try_get("pause_flag")?,
        halt_flag: risk_row.try_get("halt_flag")?,
        operator_pause: risk_row.try_get("operator_pause")?,
        peak_pnl: risk_row.try_get("peak_pnl")?,
    }))
}

/// 24h P&L report data for /report (TG-05).
pub struct ReportData {
    pub total_fees: f64,
    pub total_il: f64,
    pub total_net_pnl: f64,
    pub row_count: i64,
    pub earliest_price: f64,
    pub latest_price: f64,
}

/// Query trailing 24h P&L from pnl_history (TG-05).
///
/// `fees_earned` is written by the watch loop as a **session-relative**,
/// USD-valued running total (it re-baselines to ~0 at every watch restart and
/// its USD value moves with price). The previous `MAX(fees_earned) -
/// MIN(fees_earned)` over the window therefore reported phantom fees: it
/// double-counted across a mid-window restart and turned price swings into
/// "fee" income. We instead report the latest cumulative value (fees since the
/// current session started). A true cross-session 24h fee total needs a
/// non-resetting cumulative column — deferred until one exists.
pub async fn query_24h_report(pool: &PgPool, pool_address: &str) -> Result<ReportData> {
    let row = pool
        .fetch_one(
            query(
                "SELECT \
                   COALESCE((ARRAY_AGG(fees_earned ORDER BY time DESC))[1], 0.0) AS total_fees, \
                   COALESCE((ARRAY_AGG(il_usd ORDER BY time DESC))[1], 0.0) AS total_il, \
                   COALESCE((ARRAY_AGG(net_pnl ORDER BY time DESC))[1], 0.0) AS total_net_pnl, \
                   COUNT(*) AS row_count, \
                   COALESCE(MIN(price) FILTER (WHERE price > 0), 0.0) AS min_price, \
                   COALESCE(MAX(price) FILTER (WHERE price > 0), 0.0) AS max_price \
                 FROM pnl_history \
                 WHERE pool_address = $1 AND time >= NOW() - INTERVAL '24 hours'",
            )
            .bind(pool_address),
        )
        .await
        .context("query_24h_report failed")?;

    Ok(ReportData {
        total_fees: row.try_get("total_fees")?,
        total_il: row.try_get("total_il")?,
        total_net_pnl: row.try_get("total_net_pnl")?,
        row_count: row.try_get("row_count")?,
        earliest_price: row.try_get("min_price")?,
        latest_price: row.try_get("max_price")?,
    })
}

/// Set operator_pause flag in risk_state (D-04, TG-04).
pub async fn set_operator_pause(pool: &PgPool, pool_address: &str, paused: bool) -> Result<()> {
    pool.execute(
        query(
            "UPDATE risk_state SET operator_pause = $1, updated_at = NOW() WHERE pool_address = $2",
        )
        .bind(paused)
        .bind(pool_address),
    )
    .await
    .context("set_operator_pause failed")?;
    Ok(())
}
