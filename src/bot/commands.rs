use dptree::case;
use teloxide::{dispatching::UpdateHandler, prelude::*, utils::command::BotCommands};

use super::BotState;

#[derive(BotCommands, Clone)]
#[command(rename_rule = "lowercase", description = "LP Manager commands:")]
pub enum Command {
    #[command(description = "Show current position status and risk metrics")]
    Status,
    #[command(description = "Pause rebalancing (position stays open)")]
    Pause,
    #[command(description = "Resume rebalancing")]
    Resume,
    #[command(description = "24h P&L report")]
    Report,
    #[command(description = "Approve pending rebalance: /approve <id>")]
    Approve(String),
}

fn check_auth(msg: &Message, state: &BotState, cmd: &str) -> bool {
    if msg.chat.id.0 != state.chat_id {
        tracing::warn!(
            unauthorized_chat = msg.chat.id.0,
            "unauthorized {} attempt",
            cmd
        );
        return false;
    }
    // Chat membership is not authorization: in a group any member (or an
    // anonymous admin, where `from` is absent) could otherwise command the bot.
    match &msg.from {
        Some(user) if state.allowed_user_ids.contains(&user.id.0) => true,
        Some(user) => {
            tracing::warn!(
                unauthorized_user = user.id.0,
                "unauthorized {} attempt (user not in TELEGRAM_ALLOWED_USER_IDS)",
                cmd
            );
            false
        }
        None => {
            tracing::warn!("{} attempt without sender identity rejected", cmd);
            false
        }
    }
}

pub fn build_handler() -> UpdateHandler<anyhow::Error> {
    let command_handler = teloxide::filter_command::<Command, _>()
        .branch(case![Command::Status].endpoint(handle_status))
        .branch(case![Command::Pause].endpoint(handle_pause))
        .branch(case![Command::Resume].endpoint(handle_resume))
        .branch(case![Command::Report].endpoint(handle_report))
        .branch(case![Command::Approve(id)].endpoint(handle_approve));

    Update::filter_message().branch(command_handler)
}

async fn handle_status(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    if !check_auth(&msg, &state, "/status") {
        return Ok(());
    }

    match super::queries::query_status(&state.db_pool, &state.pool_address).await {
        Ok(None) => {
            bot.send_message(msg.chat.id, "No data yet — waiting for the first tick.")
                .await?;
        }
        Ok(Some(s)) => {
            let pause_status = if s.halt_flag {
                "HALTED (drawdown limit)"
            } else if s.operator_pause {
                "PAUSED (operator)"
            } else if s.pause_flag {
                "PAUSED (IL limit)"
            } else {
                "ACTIVE"
            };

            let msg_text = format!(
                "STATUS: {}\n\
                 Pool: {}\n\
                 Price: ${:.4}\n\
                 Position value: ${:.2}\n\
                 ---\n\
                 Fees earned: ${:.4}\n\
                 IL: ${:.4}\n\
                 Net P&L: ${:.4}\n\
                 ---\n\
                 Drawdown: {:.2}%\n\
                 Peak P&L: ${:.4}\n\
                 Status: {}",
                pause_status,
                s.pool_address,
                s.price,
                s.position_value,
                s.fees_earned,
                s.il_usd,
                s.net_pnl,
                s.drawdown_pct,
                s.peak_pnl,
                pause_status,
            );
            bot.send_message(msg.chat.id, msg_text).await?;
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("Error fetching status: {}", e))
                .await?;
        }
    }
    Ok(())
}

async fn handle_pause(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    if !check_auth(&msg, &state, "/pause") {
        return Ok(());
    }

    match super::queries::set_operator_pause(&state.db_pool, &state.pool_address, true).await {
        Ok(()) => {
            // Also update in-memory state so the watch loop sees it immediately
            if let Ok(mut rm) = state.risk_monitor.lock() {
                rm.state.operator_pause = true;
            }
            bot.send_message(
                msg.chat.id,
                "Rebalancing PAUSED (operator). Position stays open. Use /resume to restart.",
            )
            .await?;
            tracing::info!(pool = %state.pool_address, "operator pause activated via Telegram");
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("Error: {}", e))
                .await?;
        }
    }
    Ok(())
}

async fn handle_resume(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    if !check_auth(&msg, &state, "/resume") {
        return Ok(());
    }

    match super::queries::set_operator_pause(&state.db_pool, &state.pool_address, false).await {
        Ok(()) => {
            // Update in-memory state. NOTE: does NOT clear pause_flag (D-04)
            if let Ok(mut rm) = state.risk_monitor.lock() {
                rm.state.operator_pause = false;
            }
            let warning = {
                let rm = state.risk_monitor.lock().unwrap_or_else(|p| p.into_inner());
                if rm.state.pause_flag {
                    "\nNote: IL-triggered pause is still active. Rebalancing will resume when IL recovers."
                } else {
                    ""
                }
            };
            bot.send_message(
                msg.chat.id,
                format!("Rebalancing RESUMED (operator).{}", warning),
            )
            .await?;
            tracing::info!(pool = %state.pool_address, "operator pause cleared via Telegram");
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("Error: {}", e))
                .await?;
        }
    }
    Ok(())
}

async fn handle_report(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    if !check_auth(&msg, &state, "/report") {
        return Ok(());
    }

    match super::queries::query_24h_report(&state.db_pool, &state.pool_address).await {
        Ok(r) => {
            if r.row_count == 0 {
                bot.send_message(msg.chat.id, "No data in the last 24 hours.")
                    .await?;
            } else {
                let msg_text = format!(
                    "24H REPORT\n\
                     Pool: {}\n\
                     Snapshots: {}\n\
                     ---\n\
                     Total fees: ${:.4}\n\
                     Total IL: ${:.4}\n\
                     Net P&L: ${:.4}\n\
                     ---\n\
                     Price range: ${:.4} - ${:.4}",
                    state.pool_address,
                    r.row_count,
                    r.total_fees,
                    r.total_il,
                    r.total_net_pnl,
                    r.earliest_price,
                    r.latest_price,
                );
                bot.send_message(msg.chat.id, msg_text).await?;
            }
        }
        Err(e) => {
            bot.send_message(msg.chat.id, format!("Error: {}", e))
                .await?;
        }
    }
    Ok(())
}

/// Outcome of matching `/approve <id>` against the pending slot under the lock.
enum ApproveOutcome {
    Matched(super::PendingApproval),
    NoPending,
    WrongId { pending_id: u64 },
}

async fn handle_approve(
    bot: Bot,
    msg: Message,
    state: BotState,
    id_arg: String,
) -> anyhow::Result<()> {
    if !check_auth(&msg, &state, "/approve") {
        return Ok(());
    }

    let requested_id: u64 = match id_arg.trim().parse() {
        Ok(id) => id,
        Err(_) => {
            bot.send_message(
                msg.chat.id,
                "Usage: /approve <id> (the ID from the proposal message).",
            )
            .await?;
            return Ok(());
        }
    };

    let outcome = {
        let mut lock = state
            .pending_approval
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        match lock.as_ref() {
            Some(p) if p.id == requested_id => match lock.take() {
                Some(p) => ApproveOutcome::Matched(p),
                None => ApproveOutcome::NoPending,
            },
            Some(p) => ApproveOutcome::WrongId { pending_id: p.id },
            None => ApproveOutcome::NoPending,
        }
    };

    match outcome {
        ApproveOutcome::Matched(pending) => {
            // send() fails when the watch loop already timed out and dropped
            // the receiver — never confirm execution in that case.
            if pending.tx.send(true).is_ok() {
                bot.send_message(
                    msg.chat.id,
                    format!("Approved proposal #{}. Executing rebalance.", pending.id),
                )
                .await?;
            } else {
                bot.send_message(
                    msg.chat.id,
                    format!(
                        "Proposal #{} already expired. Rebalance NOT executed.",
                        pending.id
                    ),
                )
                .await?;
            }
        }
        ApproveOutcome::WrongId { pending_id } => {
            bot.send_message(
                msg.chat.id,
                format!(
                    "No proposal #{}. Pending proposal is #{} — re-check before approving.",
                    requested_id, pending_id
                ),
            )
            .await?;
        }
        ApproveOutcome::NoPending => {
            bot.send_message(msg.chat.id, "No pending rebalance to approve.")
                .await?;
        }
    }
    Ok(())
}
