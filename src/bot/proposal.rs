use anyhow::Result;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;
use teloxide::prelude::*;
use tokio::sync::oneshot;
use tracing::{info, warn};

use super::{PendingApproval, PendingApprovalSlot};

/// Monotonic proposal counter. IDs bind an `/approve <id>` to the exact
/// proposal message the operator saw (starts at 1).
static NEXT_PROPOSAL_ID: AtomicU64 = AtomicU64::new(1);

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
fn format_proposal(id: u64, data: &ProposalData) -> String {
    format!(
        "REBALANCE PROPOSAL #{}\n\
         Pool: {}\n\
         Trigger: {}\n\
         Price: ${:.4}\n\
         ---\n\
         Simulated fees: ${:.4}\n\
         Simulated IL: ${:.4}\n\
         Simulated net P&L: ${:.4}\n\
         New range width: {:.1} ticks\n\
         ---\n\
         /approve {} within timeout to execute\n\
         Timeout = auto-skip",
        id,
        data.pool_address,
        data.trigger_reason,
        data.price,
        data.simulated_fees_earned,
        data.simulated_il_usd,
        data.simulated_net_pnl,
        data.range_width,
        id,
    )
}

/// Send a proposal message to the authorized chat and return its ID plus a
/// oneshot receiver.
///
/// Installs a fresh [`PendingApproval`] into `pending` *before* sending the
/// message so an instant `/approve <id>` cannot race the installation. Only
/// one proposal can be pending at a time (D-02). On send failure the slot is
/// cleared again and the error is propagated — the caller must treat that as
/// "not approved" (fail-closed).
pub async fn send_proposal(
    bot: &Bot,
    chat_id: ChatId,
    data: &ProposalData,
    pending: &PendingApprovalSlot,
) -> Result<(u64, oneshot::Receiver<bool>)> {
    let id = NEXT_PROPOSAL_ID.fetch_add(1, Ordering::Relaxed);
    let (tx, rx) = oneshot::channel();
    {
        let mut lock = pending.lock().unwrap_or_else(|p| p.into_inner());
        // Drop any existing pending sender (previous proposal is dead).
        *lock = Some(PendingApproval { id, tx });
    }

    let msg = format_proposal(id, data);
    if let Err(e) = bot.send_message(chat_id, msg).await {
        // The operator never saw this proposal — nobody may approve it.
        clear_pending(pending, id);
        return Err(e.into());
    }
    info!(pool = %data.pool_address, proposal_id = id, "proposal sent to Telegram");

    Ok((id, rx))
}

/// Remove the pending proposal with the given ID, if it is still installed.
///
/// Called by the watch loop after a timeout so a dead sender does not linger
/// in the slot and swallow a later `/approve`. A different ID means a newer
/// proposal already replaced this one — leave it alone.
pub fn clear_pending(pending: &PendingApprovalSlot, id: u64) {
    let mut lock = pending.lock().unwrap_or_else(|p| p.into_inner());
    if lock.as_ref().is_some_and(|p| p.id == id) {
        *lock = None;
    }
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
