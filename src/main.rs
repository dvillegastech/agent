mod agent;
mod cli;
mod config;
mod cost;
mod error;
mod export;
mod markdown;
mod onboarding;
mod retry;
mod streaming;
mod tools;
mod types;

use std::path::PathBuf;

use clap::Parser;
use colored::Colorize;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

use crate::agent::runner::AgentRunner;
use crate::cli::{Cli, Commands};
use crate::config::{AgentConfig, ProviderKind};
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

async fn run_interactive(mut runner: AgentRunner) -> anyhow::Result<()> {
    print_banner();

    println!(
        "{}",
        "  Type your message and press Enter. Commands:".dimmed()
    );
    println!("{}", "    /clear   - Clear conversation history".dimmed());
    println!("{}", "    /stats   - Show conversation stats & cost".dimmed());
    println!("{}", "    /cost    - Show cost breakdown".dimmed());
    println!(
        "{}",
        "    /export  - Export conversation (md/json)".dimmed()
    );
    println!(
        "{}",
        "    /multi   - Toggle multi-line input mode".dimmed()
    );
    println!("{}", "    /help    - Show this help".dimmed());
    println!(
        "{}",
        "    /quit    - Exit (or Ctrl+D / Ctrl+C)".dimmed()
    );
    println!();

    let mut rl = DefaultEditor::new()?;
    let mut multiline_mode = false;

    let history_path = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustclaw")
        .join("history.txt");

    if let Some(parent) = history_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = rl.load_history(&history_path);

    loop {
        let input = if multiline_mode {
            read_multiline(&mut rl)?
        } else {
            let prompt = format!("{} ", ">>".bright_green());
            match rl.readline(&prompt) {
                Ok(line) => line,
                Err(ReadlineError::Interrupted) => {
                    println!("{}", "\nUse /quit to exit.".dimmed());
                    continue;
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
        };

        let input = input.trim();
        if input.is_empty() {
            continue;
        }

        let _ = rl.add_history_entry(input);

        // Handle slash commands
        match input {
            "/quit" | "/exit" | "/q" => {
                println!("\n{}", runner.cost_summary().dimmed());
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
            "/cost" => {
                println!("{}", runner.cost_summary().dimmed());
                continue;
            }
            "/multi" => {
                multiline_mode = !multiline_mode;
                if multiline_mode {
                    println!(
                        "{}",
                        "Multi-line mode ON. Enter an empty line to send.".green()
                    );
                } else {
                    println!("{}", "Multi-line mode OFF.".yellow());
                }
                continue;
            }
            cmd if cmd.starts_with("/export") => {
                let parts: Vec<&str> = cmd.split_whitespace().collect();
                let format = parts.get(1).copied().unwrap_or("md");
                let filename = parts.get(2).copied().unwrap_or_else(|| {
                    if format == "json" {
                        "conversation.json"
                    } else {
                        "conversation.md"
                    }
                });
                let path = PathBuf::from(filename);
                match runner.export_conversation(&path, format) {
                    Ok(()) => println!("{} {}", "Exported to".green(), path.display()),
                    Err(e) => eprintln!("{} {}", "Export failed:".red(), e),
                }
                continue;
            }
            "/help" => {
                println!("{}", "Commands:".bright_white());
                println!("  {} - Clear conversation", "/clear".cyan());
                println!("  {} - Conversation statistics & cost", "/stats".cyan());
                println!("  {} - Show cost breakdown", "/cost".cyan());
                println!(
                    "  {} - Export conversation (format: md|json)",
                    "/export [format] [file]".cyan()
                );
                println!("  {} - Toggle multi-line input mode", "/multi".cyan());
                println!("  {} - Show this help", "/help".cyan());
                println!("  {} - Exit", "/quit".cyan());
                continue;
            }
            _ => {}
        }

        println!();
        match runner.process_message(input).await {
            Ok(_response) => {
                // Response was already streamed to stdout; just add spacing
                println!();
            }
            Err(e) => {
                eprintln!("\n{} {}\n", "Error:".red().bold(), e);
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

/// Read multi-line input. An empty line signals end of input.
fn read_multiline(rl: &mut DefaultEditor) -> std::result::Result<String, ReadlineError> {
    let mut lines = Vec::new();
    let first_prompt = format!("{} ", ">>".bright_green());
    let cont_prompt = format!("{} ", "..".bright_green());

    loop {
        let prompt = if lines.is_empty() {
            &first_prompt
        } else {
            &cont_prompt
        };
        match rl.readline(prompt) {
            Ok(line) => {
                if line.trim().is_empty() && !lines.is_empty() {
                    break;
                }
                lines.push(line);
            }
            Err(e) => return Err(e),
        }
    }
    Ok(lines.join("\n"))
}

async fn run_single(mut runner: AgentRunner, prompt: &str) -> anyhow::Result<()> {
    let _response = runner.process_message(prompt).await?;
    // Response was already streamed to stdout
    println!();
    eprintln!("\n{}", runner.cost_summary().dimmed());
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
            if !onboarding::config_exists() {
                print_banner();
                println!(
                    "  {}",
                    "No configuration found. Starting setup wizard...".yellow()
                );

                let result = onboarding::run_onboarding().await?;

                let _ = dotenvy::from_path(&result.env_file_path);

                Ok(AgentConfig {
                    provider: result.provider,
                    api_key: result.api_key,
                    model: result.model,
                    ..AgentConfig::default()
                })
            } else {
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

    // Handle init subcommand first
    if matches!(cli.command, Some(Commands::Init)) {
        print_banner();
        onboarding::run_onboarding().await?;
        return Ok(());
    }

    // Handle config subcommand
    if matches!(cli.command, Some(Commands::Config)) {
        let config = AgentConfig::from_env().unwrap_or_else(|_| {
            let mut c = AgentConfig::default();
            c.api_key = "<not set>".into();
            c
        });
        show_config(&config);
        return Ok(());
    }

    // Load config with auto-onboarding
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
    let guard = SecurityGuard::new(config.security.clone());
    let executor = ToolExecutor::new(guard);
    let runner = AgentRunner::new(&config, executor);

    // Run mode
    if let Some(ref prompt) = cli.prompt {
        run_single(runner, prompt).await
    } else {
        run_interactive(runner).await
    }
}
