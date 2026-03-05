use std::collections::HashMap;
use std::sync::Arc;

use colored::Colorize;
use serenity::async_trait;
use serenity::model::channel::Message;
use serenity::model::gateway::Ready;
use serenity::model::id::ChannelId;
use serenity::prelude::*;
use tokio::sync::Mutex as TokioMutex;

use crate::agent::runner::AgentRunner;
use crate::config::AgentConfig;
use crate::utils;

/// Per-channel runner map for conversation isolation.
type ChannelRunners = Arc<TokioMutex<HashMap<ChannelId, Arc<TokioMutex<AgentRunner>>>>>;

/// Shared state stored in serenity's TypeMap.
struct BotStateKey;
impl TypeMapKey for BotStateKey {
    type Value = Arc<BotState>;
}

struct BotState {
    config: AgentConfig,
    runners: ChannelRunners,
}

/// Get or create a runner for a specific channel.
async fn get_runner(state: &BotState, channel_id: ChannelId) -> Arc<TokioMutex<AgentRunner>> {
    let mut runners = state.runners.lock().await;
    runners
        .entry(channel_id)
        .or_insert_with(|| {
            Arc::new(TokioMutex::new(AgentRunner::from_config(&state.config)))
        })
        .clone()
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
            utils::truncate(&text, 80).dimmed()
        );

        // Get bot state (fallible)
        let state = {
            let data = ctx.data.read().await;
            match data.get::<BotStateKey>() {
                Some(s) => s.clone(),
                None => {
                    eprintln!("{} BotState not found in TypeMap", "  [discord error]".red());
                    return;
                }
            }
        };

        // Handle commands
        if text.starts_with('!') {
            handle_command(&ctx, &msg, &text, &state).await;
            return;
        }

        // Get per-channel runner
        let runner = get_runner(&state, msg.channel_id).await;

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
        for chunk in utils::split_message(&response, 1900) {
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

async fn handle_command(ctx: &Context, msg: &Message, text: &str, state: &Arc<BotState>) {
    let cmd = text.split_whitespace().next().unwrap_or("");
    let channel_id = msg.channel_id;

    match cmd {
        "!clear" => {
            let runner = get_runner(state, channel_id).await;
            let mut runner = runner.lock().await;
            runner.clear_conversation();
            if let Err(e) = msg.channel_id.say(&ctx.http, "Conversation cleared.").await {
                eprintln!("{} Failed to send: {}", "  [discord]".red(), e);
            }
            eprintln!("{} Conversation cleared", "  [discord]".bright_magenta());
        }
        "!stats" => {
            let runner = get_runner(state, channel_id).await;
            let runner = runner.lock().await;
            let stats = runner.stats();
            if let Err(e) = msg.channel_id.say(&ctx.http, format!("📊 {stats}")).await {
                eprintln!("{} Failed to send: {}", "  [discord]".red(), e);
            }
        }
        "!cost" => {
            let runner = get_runner(state, channel_id).await;
            let runner = runner.lock().await;
            let cost = runner.cost_summary();
            if let Err(e) = msg.channel_id.say(&ctx.http, format!("💰 {cost}")).await {
                eprintln!("{} Failed to send: {}", "  [discord]".red(), e);
            }
        }
        "!help" => {
            let help = "**RustClaw Agent**\n\
                        Mention me or DM me to chat!\n\n\
                        **Commands:**\n\
                        `!clear` - Clear conversation\n\
                        `!stats` - Show stats\n\
                        `!cost` - Show cost\n\
                        `!help` - Show this help";
            if let Err(e) = msg.channel_id.say(&ctx.http, help).await {
                eprintln!("{} Failed to send: {}", "  [discord]".red(), e);
            }
        }
        _ => {
            // Unknown command - treat as regular message, process with agent
            let runner = get_runner(state, channel_id).await;
            let _ = msg.channel_id.broadcast_typing(&ctx.http).await;
            let response = {
                let mut runner = runner.lock().await;
                match runner.process_message(text).await {
                    Ok(r) => r,
                    Err(e) => format!("Error: {e}"),
                }
            };
            for chunk in utils::split_message(&response, 1900) {
                if let Err(e) = msg.channel_id.say(&ctx.http, chunk).await {
                    eprintln!("{} Failed to send: {}", "  [discord]".red(), e);
                }
            }
        }
    }
}

/// Run the Discord bot.
pub async fn run_discord_bot(config: AgentConfig, token: &str) -> anyhow::Result<()> {
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

    let state = Arc::new(BotState {
        config,
        runners: Arc::new(TokioMutex::new(HashMap::new())),
    });

    let intents = GatewayIntents::GUILD_MESSAGES
        | GatewayIntents::DIRECT_MESSAGES
        | GatewayIntents::MESSAGE_CONTENT;

    let mut client = Client::builder(token, intents)
        .event_handler(Handler)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to create Discord client: {e}"))?;

    // Store state in client data
    {
        let mut data = client.data.write().await;
        data.insert::<BotStateKey>(state);
    }

    client
        .start()
        .await
        .map_err(|e| anyhow::anyhow!("Discord bot error: {e}"))?;

    Ok(())
}
