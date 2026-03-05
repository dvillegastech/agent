use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(
    name = "rustclaw",
    about = "RustClaw - A secure LLM agent CLI written in Rust",
    version,
    author
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Option<Commands>,

    /// Execute a single prompt and exit (non-interactive mode)
    #[arg(short, long)]
    pub prompt: Option<String>,

    /// LLM provider to use (anthropic, openai)
    #[arg(long, env = "RUSTCLAW_PROVIDER")]
    pub provider: Option<String>,

    /// Model name to use
    #[arg(long, env = "RUSTCLAW_MODEL")]
    pub model: Option<String>,

    /// API base URL override
    #[arg(long, env = "RUSTCLAW_BASE_URL")]
    pub base_url: Option<String>,

    /// Disable shell command confirmation prompts
    #[arg(long)]
    pub no_confirm: bool,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Start interactive chat session (default)
    Chat,
    /// Run the configuration wizard (first-time setup)
    Init,
    /// Show current configuration
    Config,
    /// Start HTTP Gateway API server
    Gateway {
        /// Host to bind to
        #[arg(long, default_value = "0.0.0.0")]
        host: String,
        /// Port to listen on
        #[arg(long, default_value_t = 3000)]
        port: u16,
    },
    /// Start Telegram bot
    Telegram {
        /// Telegram bot token (or set TELEGRAM_BOT_TOKEN env var)
        #[arg(long, env = "TELEGRAM_BOT_TOKEN")]
        token: String,
    },
    /// Start Discord bot
    Discord {
        /// Discord bot token (or set DISCORD_BOT_TOKEN env var)
        #[arg(long, env = "DISCORD_BOT_TOKEN")]
        token: String,
    },
    /// Start Web Admin Dashboard
    Dashboard {
        /// Host to bind to
        #[arg(long, default_value = "127.0.0.1")]
        host: String,
        /// Port to listen on
        #[arg(long, default_value_t = 8080)]
        port: u16,
    },
}
