use std::sync::Arc;

use colored::Colorize;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::prelude::*;
use tokio::sync::Mutex as TokioMutex;

use crate::agent::runner::AgentRunner;
use crate::config::AgentConfig;
use crate::tools::executor::ToolExecutor;
use crate::tools::security::SecurityGuard;

/// Key for storing the AgentRunner in serenity's TypeMap.
struct RunnerKey;
impl TypeMapKey for RunnerKey {
    type Value = Arc<TokioMutex<AgentRunner>>;
}

/// Discord event handler.
struct Handler;

#[async_trait]
impl EventHandler for Handler {
    async fn message(&self, ctx: Context, msg: Message) {
        // Ignore bot's own messages
        if msg.author.bot {
            return;
        }

        let bot_user_id = ctx.cache.current_user().id;

        // Only respond if mentioned or in DMs
        let is_dm = msg.guild_id.is_none();
        let is_mentioned = msg.mentions.iter().any(|u| u.id == bot_user_id);

        if !is_dm && !is_mentioned {
            return;
        }

        // Strip bot mention from message text
        let text = msg
            .content
            .replace(&format!("<@{}>", bot_user_id), "")
            .replace(&format!("<@!{}>", bot_user_id), "")
            .trim()
            .to_string();

        if text.is_empty() {
            return;
        }

        let user = &msg.author.name;
        eprintln!(
            "{} {}: {}",
            "  [discord]".bright_magenta(),
            user.cyan(),
            truncate(&text, 80).dimmed()
        );

        // Handle commands
        if text.starts_with('!') {
            handle_command(&ctx, &msg, &text).await;
            return;
        }

        // Get runner from data
        let runner = {
            let data = ctx.data.read().await;
            data.get::<RunnerKey>()
                .expect("RunnerKey must be in TypeMap")
                .clone()
        };

        // Show typing indicator
        let _ = msg.channel_id.broadcast_typing(&ctx.http).await;

        // Process with agent
        let response = {
            let mut runner = runner.lock().await;
            match runner.process_message(&text).await {
                Ok(response) => response,
                Err(e) => {
                    eprintln!("{} {}", "  [discord error]".red(), e);
                    format!("Error: {e}")
                }
            }
        };

        // Split long messages (Discord limit is 2000 chars)
        for chunk in split_message(&response, 1900) {
            if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                eprintln!("{} Failed to send: {}", "  [discord]".red(), e);
            }
        }
    }

    async fn ready(&self, _: Context, ready: Ready) {
        eprintln!(
            "{} Connected as {}",
            "  [discord]".bright_magenta(),
            ready.user.name.bright_cyan()
        );
    }
}

async fn handle_command(ctx: &Context, msg: &Message, text: &str) {
    let cmd = text.split_whitespace().next().unwrap_or("");

    let runner = {
        let data = ctx.data.read().await;
        data.get::<RunnerKey>()
            .expect("RunnerKey must be in TypeMap")
            .clone()
    };

    match cmd {
        "!clear" => {
            let mut runner = runner.lock().await;
            runner.clear_conversation();
            let _ = msg.channel_id.say(&ctx.http, "Conversation cleared.").await;
            eprintln!("{} Conversation cleared", "  [discord]".bright_magenta());
        }
        "!stats" => {
            let runner = runner.lock().await;
            let stats = runner.stats();
            let _ = msg.channel_id.say(&ctx.http, format!("📊 {stats}")).await;
        }
        "!cost" => {
            let runner = runner.lock().await;
            let cost = runner.cost_summary();
            let _ = msg.channel_id.say(&ctx.http, format!("💰 {cost}")).await;
        }
        "!help" => {
            let help = "**RustClaw Agent**\n\
                        Mention me or DM me to chat!\n\n\
                        **Commands:**\n\
                        `!clear` - Clear conversation\n\
                        `!stats` - Show stats\n\
                        `!cost` - Show cost\n\
                        `!help` - Show this help";
            let _ = msg.channel_id.say(&ctx.http, help).await;
        }
        _ => {
            // Unknown command - treat as regular message, process with agent
            let _ = msg.channel_id.broadcast_typing(&ctx.http).await;
            let response = {
                let mut runner = runner.lock().await;
                match runner.process_message(text).await {
                    Ok(r) => r,
                    Err(e) => format!("Error: {e}"),
                }
            };
            for chunk in split_message(&response, 1900) {
                let _ = msg.channel_id.say(&ctx.http, chunk).await;
            }
        }
    }
}

/// Run the Discord bot.
pub async fn run_discord_bot(config: AgentConfig, token: &str) -> anyhow::Result<()> {
    let guard = SecurityGuard::new(config.security.clone());
    let executor = ToolExecutor::new(guard);
    let runner = AgentRunner::new(&config, executor);

    let shared_runner = Arc::new(TokioMutex::new(runner));

    eprintln!(
        "\n{} Discord bot starting...",
        "  [discord]".bright_magenta()
    );
    eprintln!(
        "  {} Model: {} ({})\n",
        "│".dimmed(),
        config.model.bright_white(),
        config.provider.to_string().cyan()
    );

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(token, intents)
        .event_handler(Handler)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create Discord client: {e}"))?;

    // Store runner in client data
    {
        let mut data = client.data.write().await;
        data.insert::<RunnerKey>(shared_runner);
    }

    client
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("Discord bot error: {e}"))?;

    Ok(())
}

/// Split a message into chunks for Discord's 2000 char limit.
fn split_message(text: &str, max_len: usize) -> Vec<&str> {
    if text.len() <= max_len {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let mut end = (start + max_len).min(text.len());

        while end > start && !text.is_char_boundary(end) {
            end -= 1;
        }

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
