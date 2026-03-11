use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use teloxide::prelude::*;
use tokio::sync::oneshot;
use tracing::{info, warn};

/// Simulated outcome data sent in the proposal message.
pub struct ProposalData {
    pub pool_address: String,
    pub trigger_reason: String,
    pub price: f64,
    pub simulated_fees_earned: f64,
    pub simulated_il_usd: f64,
    pub simulated_net_pnl: f64,
    pub range_width: f64,
}

/// Format a proposal message for Telegram.
fn format_proposal(data: &ProposalData) -> String {
    format!(
        "REBALANCE PROPOSAL\n\
         Pool: {}\n\
         Trigger: {}\n\
         Price: ${:.4}\n\
         ---\n\
         Simulated fees: ${:.4}\n\
         Simulated IL: ${:.4}\n\
         Simulated net P&L: ${:.4}\n\
         New range width: {:.1} ticks\n\
         ---\n\
         /approve within timeout to execute\n\
         Timeout = auto-skip",
        data.pool_address,
        data.trigger_reason,
        data.price,
        data.simulated_fees_earned,
        data.simulated_il_usd,
        data.simulated_net_pnl,
        data.range_width,
    )
}

/// Send a proposal message to the authorized chat and return a oneshot receiver.
///
/// Installs a fresh oneshot::Sender into `pending_approval` so the /approve
/// handler can complete it. Only one proposal can be pending at a time (D-02).
pub async fn send_proposal(
    bot: &Bot,
    chat_id: ChatId,
    data: &ProposalData,
    pending: &Arc<Mutex<Option<oneshot::Sender<bool>>>>,
) -> Result<oneshot::Receiver<bool>> {
    // Drop any existing pending sender (previous proposal times out)
    {
        let mut lock = pending.lock().unwrap_or_else(|p| p.into_inner());
        *lock = None;
    }

    let msg = format_proposal(data);
    bot.send_message(chat_id, msg).await?;
    info!(pool = %data.pool_address, "proposal sent to Telegram");

    let (tx, rx) = oneshot::channel();
    {
        let mut lock = pending.lock().unwrap_or_else(|p| p.into_inner());
        *lock = Some(tx);
    }

    Ok(rx)
}

/// Await approval with timeout. Returns true if approved, false if timed out or rejected.
pub async fn await_approval(rx: oneshot::Receiver<bool>, timeout_secs: u64) -> bool {
    match tokio::time::timeout(Duration::from_secs(timeout_secs), rx).await {
        Ok(Ok(true)) => {
            info!("rebalance approved via Telegram");
            true
        }
        Ok(Ok(false)) => {
            info!("rebalance explicitly rejected via Telegram");
            false
        }
        Ok(Err(_)) => {
            // Sender dropped without sending (e.g., new proposal replaced it)
            warn!("approval channel closed without response");
            false
        }
        Err(_) => {
            info!("rebalance approval timed out");
            false
        }
    }
}
