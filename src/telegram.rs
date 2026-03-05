use std::sync::Arc;

use colored::Colorize;
use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tokio::sync::Mutex;

use crate::agent::runner::AgentRunner;
use crate::config::AgentConfig;
use crate::tools::executor::ToolExecutor;
use crate::tools::security::SecurityGuard;

/// Run the Telegram bot.
pub async fn run_telegram_bot(config: AgentConfig, token: &str) -> anyhow::Result<()> {
    let guard = SecurityGuard::new(config.security.clone());
    let executor = ToolExecutor::new(guard);
    let runner = AgentRunner::new(&config, executor);

    let shared_runner = Arc::new(Mutex::new(runner));

    eprintln!(
        "\n{} Telegram bot starting...",
        "  [telegram]".bright_magenta()
    );
    eprintln!(
        "  {} Model: {} ({})\n",
        "│".dimmed(),
        config.model.bright_white(),
        config.provider.to_string().cyan()
    );

    let bot = Bot::new(token);

    // Get bot info for display
    match bot.get_me().await {
        Ok(me) => {
            eprintln!(
                "{} Connected as @{}",
                "  [telegram]".bright_magenta(),
                me.username().bright_cyan()
            );
        }
        Err(e) => {
            eprintln!("{} Failed to connect: {}", "  [telegram]".red(), e);
            return Err(anyhow::anyhow!("Failed to connect to Telegram: {e}"));
        }
    }

    let handler = Update::filter_message().endpoint(
        move |bot: Bot, msg: Message, runner: Arc<Mutex<AgentRunner>>| async move {
            handle_message(bot, msg, runner).await
        },
    );

    Dispatcher::builder(bot, handler)
        .dependencies(dptree::deps![shared_runner])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

async fn handle_message(
    bot: Bot,
    msg: Message,
    runner: Arc<Mutex<AgentRunner>>,
) -> Result<(), teloxide::RequestError> {
    let text = match msg.text() {
        Some(t) => t,
        None => return Ok(()), // Ignore non-text messages
    };

    let chat_id = msg.chat.id;
    let user = msg
        .from
        .as_ref()
        .and_then(|u| u.username.as_deref())
        .unwrap_or("unknown");

    eprintln!(
        "{} @{}: {}",
        "  [telegram]".bright_magenta(),
        user.cyan(),
        truncate(text, 80).dimmed()
    );

    // Handle bot commands
    if text.starts_with('/') {
        return handle_command(&bot, chat_id, text, &runner).await;
    }

    // Process with agent
    let response = {
        let mut runner = runner.lock().await;
        match runner.process_message(text).await {
            Ok(response) => response,
            Err(e) => {
                eprintln!("{} {}", "  [telegram error]".red(), e);
                format!("Error: {e}")
            }
        }
    };

    // Split long messages (Telegram limit is 4096 chars)
    for chunk in split_message(&response, 4000) {
        bot.send_message(chat_id, chunk).await?;
    }

    Ok(())
}

async fn handle_command(
    bot: &Bot,
    chat_id: ChatId,
    text: &str,
    runner: &Arc<Mutex<AgentRunner>>,
) -> Result<(), teloxide::RequestError> {
    let cmd = text.split_whitespace().next().unwrap_or("");

    match cmd {
        "/start" => {
            let msg = "🦀 *RustClaw Agent*\n\n\
                       I'm an AI coding assistant. Send me any message and I'll help!\n\n\
                       Commands:\n\
                       /clear \\- Clear conversation history\n\
                       /stats \\- Show conversation stats\n\
                       /cost \\- Show cost breakdown\n\
                       /help \\- Show this help";
            bot.send_message(chat_id, msg)
                .parse_mode(ParseMode::MarkdownV2)
                .await?;
        }
        "/clear" => {
            let mut runner = runner.lock().await;
            runner.clear_conversation();
            bot.send_message(chat_id, "Conversation cleared.").await?;
            eprintln!("{} Conversation cleared", "  [telegram]".bright_magenta());
        }
        "/stats" => {
            let runner = runner.lock().await;
            let stats = runner.stats();
            bot.send_message(chat_id, format!("📊 {stats}")).await?;
        }
        "/cost" => {
            let runner = runner.lock().await;
            let cost = runner.cost_summary();
            bot.send_message(chat_id, format!("💰 {cost}")).await?;
        }
        "/help" => {
            let msg = "Available commands:\n\
                       /clear - Clear conversation\n\
                       /stats - Show stats\n\
                       /cost - Show cost\n\
                       /help - Show help\n\n\
                       Just send any text message to chat with the AI agent.";
            bot.send_message(chat_id, msg).await?;
        }
        _ => {
            bot.send_message(chat_id, "Unknown command. Use /help to see available commands.")
                .await?;
        }
    }

    Ok(())
}

/// Split a message into chunks that fit Telegram's message size limit.
fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let mut end = (start + max_len).min(text.len());

        // Find valid UTF-8 boundary
        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }

        // Try to break at a newline for cleaner splits
        if end < text.len() {
            if let Some(nl) = text[start..end].rfind('\n') {
                end = start + nl + 1;
            }
        }

        chunks.push(&text[start..end]);
        start = end;
    }

    chunks
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}
