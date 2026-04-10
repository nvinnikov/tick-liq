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
    #[command(description = "Approve pending rebalance")]
    Approve,
}

pub fn build_handler() -> UpdateHandler<anyhow::Error> {
    let command_handler = teloxide::filter_command::<Command, _>()
        .branch(case![Command::Status].endpoint(handle_status))
        .branch(case![Command::Pause].endpoint(handle_pause))
        .branch(case![Command::Resume].endpoint(handle_resume))
        .branch(case![Command::Report].endpoint(handle_report))
        .branch(case![Command::Approve].endpoint(handle_approve));

    Update::filter_message().branch(command_handler)
}

async fn handle_status(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    // Auth gate: only authorized chat_id can interact (T-07-02)
    if msg.chat.id.0 != state.chat_id {
        tracing::warn!(
            unauthorized_chat = msg.chat.id.0,
            "unauthorized /status attempt"
        );
        return Ok(());
    }
    bot.send_message(msg.chat.id, "Status: not yet implemented (Plan 03)")
        .await?;
    Ok(())
}

async fn handle_pause(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    // Auth gate: only authorized chat_id can interact (T-07-02)
    if msg.chat.id.0 != state.chat_id {
        tracing::warn!(
            unauthorized_chat = msg.chat.id.0,
            "unauthorized /pause attempt"
        );
        return Ok(());
    }
    bot.send_message(msg.chat.id, "Pause: not yet implemented (Plan 03)")
        .await?;
    Ok(())
}

async fn handle_resume(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    // Auth gate: only authorized chat_id can interact (T-07-02)
    if msg.chat.id.0 != state.chat_id {
        tracing::warn!(
            unauthorized_chat = msg.chat.id.0,
            "unauthorized /resume attempt"
        );
        return Ok(());
    }
    bot.send_message(msg.chat.id, "Resume: not yet implemented (Plan 03)")
        .await?;
    Ok(())
}

async fn handle_report(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    // Auth gate: only authorized chat_id can interact (T-07-02)
    if msg.chat.id.0 != state.chat_id {
        tracing::warn!(
            unauthorized_chat = msg.chat.id.0,
            "unauthorized /report attempt"
        );
        return Ok(());
    }
    bot.send_message(msg.chat.id, "Report: not yet implemented (Plan 03)")
        .await?;
    Ok(())
}

async fn handle_approve(bot: Bot, msg: Message, state: BotState) -> anyhow::Result<()> {
    // Auth gate: only authorized chat_id can approve (T-07-02)
    if msg.chat.id.0 != state.chat_id {
        tracing::warn!(
            unauthorized_chat = msg.chat.id.0,
            "unauthorized /approve attempt"
        );
        return Ok(());
    }

    let sender = {
        let mut lock = state
            .pending_approval
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        lock.take()
    };

    match sender {
        Some(tx) => {
            let _ = tx.send(true);
            bot.send_message(msg.chat.id, "Approved. Executing rebalance.")
                .await?;
        }
        None => {
            bot.send_message(msg.chat.id, "No pending rebalance to approve.")
                .await?;
        }
    }
    Ok(())
}
