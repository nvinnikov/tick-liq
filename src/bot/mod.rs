pub mod commands;

use anyhow::Result;
use sqlx_postgres::PgPool;
use std::sync::{Arc, Mutex};
use teloxide::prelude::*;
use tokio::sync::oneshot;

use crate::strategy::risk_monitor::RiskMonitor;

/// Shared state passed to every bot command handler.
#[derive(Clone)]
#[allow(dead_code)] // fields used by Plans 02 and 03 handler implementations
pub struct BotState {
    pub db_pool: PgPool,
    pub risk_monitor: Arc<Mutex<RiskMonitor>>,
    pub pool_address: String,
    pub mint: String,
    /// When a rebalance proposal is pending, holds the oneshot sender.
    /// `/approve` sends `true`, timeout or `/reject` sends nothing (sender dropped).
    pub pending_approval: Arc<Mutex<Option<oneshot::Sender<bool>>>>,
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
