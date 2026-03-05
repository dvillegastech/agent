mod agent;
mod cli;
mod config;
mod error;
mod onboarding;
mod providers;
mod tools;
mod types;

use clap::Parser;
use colored::Colorize;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::agent::runner::AgentRunner;
use crate::cli::{Cli, Commands};
use crate::config::{AgentConfig, ProviderKind};
use crate::providers::anthropic::AnthropicProvider;
use crate::providers::openai::OpenAIProvider;
use crate::providers::LlmProvider;
use crate::tools::executor::ToolExecutor;
use crate::tools::security::SecurityGuard;

fn print_banner() {
    println!(
        "{}",
        r#"
  ____            _    ____ _
 |  _ \ _   _ ___| |_ / ___| | __ ___      __
 | |_) | | | / __| __| |   | |/ _` \ \ /\ / /
 |  _ <| |_| \__ \ |_| |___| | (_| |\ V  V /
 |_| \_\\__,_|___/\__|\____|_|\__,_| \_/\_/
"#
        .bright_cyan()
    );
    println!(
        "  {} {}\n",
        "Secure LLM Agent".bright_white(),
        format!("v{}", env!("CARGO_PKG_VERSION")).dimmed()
    );
}

fn build_provider(config: &AgentConfig) -> Box<dyn LlmProvider> {
    match config.provider {
        ProviderKind::Anthropic => Box::new(AnthropicProvider::new(config)),
        ProviderKind::OpenAI => Box::new(OpenAIProvider::new(config)),
    }
}

async fn run_interactive(mut runner: AgentRunner) -> anyhow::Result<()> {
    print_banner();

    println!(
        "{}",
        "  Type your message and press Enter. Commands:".dimmed()
    );
    println!("{}", "    /clear  - Clear conversation history".dimmed());
    println!("{}", "    /stats  - Show conversation stats".dimmed());
    println!("{}", "    /help   - Show this help".dimmed());
    println!(
        "{}",
        "    /quit   - Exit (or Ctrl+D / Ctrl+C)".dimmed()
    );
    println!();

    let mut rl = DefaultEditor::new()?;

    let history_path = dirs::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("rustclaw")
        .join("history.txt");

    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rl.load_history(&history_path);

    loop {
        let prompt = format!("{} ", ">>".bright_green());
        match rl.readline(&prompt) {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(input);

                match input {
                    "/quit" | "/exit" | "/q" => {
                        println!("{}", "Goodbye!".bright_cyan());
                        break;
                    }
                    "/clear" => {
                        runner.clear_conversation();
                        println!("{}", "Conversation cleared.".green());
                        continue;
                    }
                    "/stats" => {
                        println!("{}", runner.stats().dimmed());
                        continue;
                    }
                    "/help" => {
                        println!("{}", "Commands:".bright_white());
                        println!("  {} - Clear conversation", "/clear".cyan());
                        println!("  {} - Conversation statistics", "/stats".cyan());
                        println!("  {} - Show this help", "/help".cyan());
                        println!("  {} - Exit", "/quit".cyan());
                        continue;
                    }
                    _ => {}
                }

                println!();
                match runner.process_message(input).await {
                    Ok(response) => {
                        println!("\n{}\n", response);
                    }
                    Err(e) => {
                        eprintln!("\n{} {}\n", "Error:".red().bold(), e);
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("{}", "\nUse /quit to exit.".dimmed());
            }
            Err(ReadlineError::Eof) => {
                println!("{}", "\nGoodbye!".bright_cyan());
                break;
            }
            Err(e) => {
                eprintln!("{} {}", "Readline error:".red(), e);
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

async fn run_single(mut runner: AgentRunner, prompt: &str) -> anyhow::Result<()> {
    let response = runner.process_message(prompt).await?;
    println!("{response}");
    Ok(())
}

fn show_config(config: &AgentConfig) {
    println!("{}", "RustClaw Configuration".bright_white().bold());
    println!("  {}: {}", "Provider".cyan(), config.provider);
    println!("  {}: {}", "Model".cyan(), config.model);
    println!("  {}: {}", "Base URL".cyan(), config.provider_url());
    println!("  {}: {}", "Max tokens".cyan(), config.max_tokens);
    println!("  {}: {}", "Temperature".cyan(), config.temperature);
    println!(
        "  {}: {}",
        "Max tool iterations".cyan(),
        config.max_tool_iterations
    );
    println!(
        "  {}: {}s",
        "Command timeout".cyan(),
        config.security.command_timeout_secs
    );
    println!(
        "  {}: {}",
        "Shell confirmation".cyan(),
        config.security.require_shell_confirmation
    );
    println!(
        "  {}: {:?}",
        "Allowed dirs".cyan(),
        config.security.allowed_dirs
    );
    println!(
        "  {}: {} patterns",
        "Blocked commands".cyan(),
        config.security.blocked_commands.len()
    );
}

/// Try to load config from env, or run onboarding wizard if not configured.
async fn load_or_onboard_config() -> anyhow::Result<AgentConfig> {
    match AgentConfig::from_env() {
        Ok(config) => Ok(config),
        Err(_) => {
            // No valid config found - check if this looks like first run
            if !onboarding::config_exists() {
                print_banner();
                println!(
                    "  {}",
                    "No configuration found. Starting setup wizard...".yellow()
                );

                let result = onboarding::run_onboarding().await?;

                // Reload env from the newly written file
                let _ = dotenvy::from_path(&result.env_file_path);

                Ok(AgentConfig {
                    provider: result.provider,
                    api_key: result.api_key,
                    model: result.model,
                    ..AgentConfig::default()
                })
            } else {
                // Config file exists but is broken
                eprintln!(
                    "{} {}",
                    "Configuration error:".red().bold(),
                    "API key not found or invalid."
                );
                eprintln!(
                    "{}",
                    "Run 'rustclaw init' to reconfigure, or set ANTHROPIC_API_KEY / OPENAI_API_KEY."
                        .dimmed()
                );
                std::process::exit(1);
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    // Handle init subcommand first (doesn't need existing config)
    if matches!(cli.command, Some(Commands::Init)) {
        print_banner();
        onboarding::run_onboarding().await?;
        return Ok(());
    }

    // Handle config subcommand (can work without API key)
    if matches!(cli.command, Some(Commands::Config)) {
        let config = AgentConfig::from_env().unwrap_or_else(|_| {
            let mut c = AgentConfig::default();
            c.api_key = "<not set>".into();
            c
        });
        show_config(&config);
        return Ok(());
    }

    // Load config with auto-onboarding for first-time users
    let mut config = load_or_onboard_config().await?;

    // Apply CLI overrides
    if let Some(ref provider) = cli.provider {
        config.provider = match provider.to_lowercase().as_str() {
            "openai" => ProviderKind::OpenAI,
            _ => ProviderKind::Anthropic,
        };
    }
    if let Some(ref model) = cli.model {
        config.model = model.clone();
    }
    if let Some(ref url) = cli.base_url {
        config.base_url = Some(url.clone());
    }
    if cli.no_confirm {
        config.security.require_shell_confirmation = false;
    }

    // Build components
    let provider = build_provider(&config);
    let guard = SecurityGuard::new(config.security.clone());
    let executor = ToolExecutor::new(guard);
    let runner = AgentRunner::new(&config, provider, executor);

    // Run mode
    if let Some(ref prompt) = cli.prompt {
        run_single(runner, prompt).await
    } else {
        run_interactive(runner).await
    }
}
