pub mod commands;
pub mod proposal;
pub mod queries;

use anyhow::Result;
use sqlx_postgres::PgPool;
use std::sync::{Arc, Mutex};
use teloxide::prelude::*;
use tokio::sync::oneshot;

use crate::strategy::risk_monitor::RiskMonitor;

/// A rebalance proposal awaiting `/approve <id>`.
///
/// The ID binds an approval to the exact proposal the operator saw: a late
/// `/approve` for an expired proposal must never authorize a newer one.
pub struct PendingApproval {
    pub id: u64,
    pub tx: oneshot::Sender<bool>,
}

/// Shared slot holding at most one pending proposal (D-02).
pub type PendingApprovalSlot = Arc<Mutex<Option<PendingApproval>>>;

/// Shared state passed to every bot command handler.
#[derive(Clone)]
#[allow(dead_code)] // fields used by Plans 02 and 03 handler implementations
pub struct BotState {
    pub db_pool: PgPool,
    pub risk_monitor: Arc<Mutex<RiskMonitor>>,
    pub pool_address: String,
    pub mint: String,
    /// When a rebalance proposal is pending, holds its ID and oneshot sender.
    /// `/approve <id>` sends `true`; timeout drops the sender.
    pub pending_approval: PendingApprovalSlot,
    /// Authorized Telegram chat ID. Only messages from this chat are processed.
    /// Loaded from TELEGRAM_CHAT_ID env var.
    pub chat_id: i64,
    /// Authorized Telegram user IDs. Chat membership alone is not enough to
    /// approve fund movement — the sender must also be on this allowlist.
    /// Loaded from TELEGRAM_ALLOWED_USER_IDS env var.
    pub allowed_user_ids: Vec<u64>,
}

/// Load the authorized Telegram chat ID from the TELEGRAM_CHAT_ID env var.
pub fn load_chat_id() -> Result<i64> {
    let id_str = std::env::var("TELEGRAM_CHAT_ID")
        .map_err(|_| anyhow::anyhow!("TELEGRAM_CHAT_ID env var is required for bot mode"))?;
    id_str
        .parse::<i64>()
        .map_err(|e| anyhow::anyhow!("TELEGRAM_CHAT_ID must be a valid i64: {}", e))
}

/// Load the operator user allowlist from the TELEGRAM_ALLOWED_USER_IDS env var
/// (comma-separated Telegram user IDs).
///
/// Required in bot mode: in a group chat any member — including ones added
/// later — can send commands, so authorization must be per-user, not per-chat.
pub fn load_allowed_user_ids() -> Result<Vec<u64>> {
    let raw = std::env::var("TELEGRAM_ALLOWED_USER_IDS").map_err(|_| {
        anyhow::anyhow!(
            "TELEGRAM_ALLOWED_USER_IDS env var is required for bot mode \
             (comma-separated Telegram user IDs allowed to send commands)"
        )
    })?;
    let ids: Vec<u64> = raw
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            s.parse::<u64>()
                .map_err(|e| anyhow::anyhow!("TELEGRAM_ALLOWED_USER_IDS entry '{}': {}", s, e))
        })
        .collect::<Result<_>>()?;
    if ids.is_empty() {
        anyhow::bail!("TELEGRAM_ALLOWED_USER_IDS must contain at least one user ID");
    }
    Ok(ids)
}

/// Spawn the Telegram bot as a background tokio task.
///
/// Returns a `JoinHandle` so the watch loop can detect bot crashes.
/// Reads `TELEGRAM_BOT_TOKEN` from environment (panics if absent per TG security).
pub async fn spawn_bot(state: BotState) -> Result<tokio::task::JoinHandle<()>> {
    let token = std::env::var("TELEGRAM_BOT_TOKEN")
        .map_err(|_| anyhow::anyhow!("TELEGRAM_BOT_TOKEN env var is required for bot mode"))?;

    let bot = Bot::new(token);

    let handler = commands::build_handler();

    let handle = tokio::spawn(async move {
        Dispatcher::builder(bot, handler)
            .dependencies(dptree::deps![state])
            .enable_ctrlc_handler()
            .build()
            .dispatch()
            .await;
    });

    Ok(handle)
}
