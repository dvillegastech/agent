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
}
