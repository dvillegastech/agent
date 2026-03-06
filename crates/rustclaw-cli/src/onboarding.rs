use std::io::{self, Write};
use std::path::PathBuf;

use colored::Colorize;
use reqwest::Client;
use serde_json::json;

use rustclaw_core::config::ProviderKind;

/// Result of the onboarding wizard.
pub struct OnboardingResult {
    pub provider: ProviderKind,
    pub api_key: String,
    pub model: String,
    pub env_file_path: PathBuf,
}

/// Returns the path to the project-local .env file.
fn env_file_path() -> PathBuf {
    std::env::current_dir()
        .unwrap_or_else(|_| PathBuf::from("."))
        .join(".env")
}

/// Returns the path to the global config .env file.
fn global_env_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustclaw")
        .join(".env")
}

/// Check if configuration already exists (either local or global .env).
pub fn config_exists() -> bool {
    env_file_path().exists() || global_env_path().exists()
}

/// Run the interactive onboarding wizard.
pub async fn run_onboarding() -> anyhow::Result<OnboardingResult> {
    println!();
    println!(
        "{}",
        "  Welcome to RustClaw Setup!".bright_cyan().bold()
    );
    println!(
        "{}",
        "  Let's configure your LLM agent.\n".dimmed()
    );

    // Step 1: Choose provider
    let provider = choose_provider()?;

    // Step 2: Enter API key (skip for Ollama)
    let api_key = if provider == ProviderKind::Ollama {
        String::new()
    } else {
        enter_api_key(&provider)?
    };

    // Step 3: Validate
    let default_model = match provider {
        ProviderKind::Anthropic => "claude-sonnet-4-20250514",
        ProviderKind::OpenAI => "gpt-4o",
        ProviderKind::Ollama => "qwen3:8b",
    };

    if provider == ProviderKind::Ollama {
        println!(
            "\n  {} {}",
            "Checking Ollama server...".dimmed(),
            "(http://localhost:11434)".dimmed()
        );
        match validate_ollama().await {
            Ok(()) => {
                println!("  {} {}\n", "OK".bright_green().bold(), "Ollama is running!".green());
            }
            Err(e) => {
                println!(
                    "  {} {}\n",
                    "WARNING".yellow().bold(),
                    format!("Ollama check failed: {e}").yellow()
                );
                println!("  {}", "Make sure Ollama is installed and running: https://ollama.ai".dimmed());
                let proceed = confirm("  Save anyway and continue?")?;
                if !proceed {
                    anyhow::bail!("Onboarding cancelled by user.");
                }
            }
        }
    } else {
        println!(
            "\n  {} {}",
            "Validating API key...".dimmed(),
            "(making a test request)".dimmed()
        );

        match validate_api_key(&provider, &api_key, default_model).await {
            Ok(()) => {
                println!("  {} {}\n", "OK".bright_green().bold(), "API key is valid!".green());
            }
            Err(e) => {
                println!(
                    "  {} {}\n",
                    "WARNING".yellow().bold(),
                    format!("Validation failed: {e}").yellow()
                );
                let proceed = confirm("  Save anyway and continue?")?;
                if !proceed {
                    anyhow::bail!("Onboarding cancelled by user.");
                }
            }
        }
    }

    // Step 4: Choose model
    let model = choose_model(&provider, default_model)?;

    // Step 5: Choose where to save
    let save_path = choose_save_location()?;

    // Step 6: Save .env file
    save_env_file(&save_path, &provider, &api_key, &model)?;

    println!(
        "\n  {} Configuration saved to {}",
        "Done!".bright_green().bold(),
        save_path.display().to_string().cyan()
    );
    println!(
        "  {}\n",
        "You can edit this file anytime or re-run 'rustclaw init'.".dimmed()
    );

    Ok(OnboardingResult {
        provider,
        api_key,
        model: model.to_string(),
        env_file_path: save_path,
    })
}

fn choose_provider() -> anyhow::Result<ProviderKind> {
    println!("  {}", "Step 1: Choose your LLM provider".bright_white().bold());
    println!();
    println!("    {} Anthropic (Claude)", "[1]".cyan());
    println!("    {} OpenAI (GPT)", "[2]".cyan());
    println!("    {} Ollama (Local - free, no API key needed)", "[3]".cyan());
    println!();

    loop {
        let input = prompt_input("  Choice [1]: ")?;
        let trimmed = input.trim();

        if trimmed.is_empty() || trimmed == "1" {
            println!("  {} Anthropic\n", "Selected:".dimmed());
            return Ok(ProviderKind::Anthropic);
        } else if trimmed == "2" {
            println!("  {} OpenAI\n", "Selected:".dimmed());
            return Ok(ProviderKind::OpenAI);
        } else if trimmed == "3" {
            println!("  {} Ollama (Local)\n", "Selected:".dimmed());
            return Ok(ProviderKind::Ollama);
        } else {
            println!("  {} Please enter 1, 2, or 3.", "Invalid:".red());
        }
    }
}

fn enter_api_key(provider: &ProviderKind) -> anyhow::Result<String> {
    let key_name = match provider {
        ProviderKind::Anthropic => "Anthropic API key (sk-ant-...)",
        ProviderKind::OpenAI => "OpenAI API key (sk-...)",
        ProviderKind::Ollama => return Ok(String::new()),
    };

    println!("  {}", "Step 2: Enter your API key".bright_white().bold());
    println!(
        "  {}",
        format!("  Get one at: {}", match provider {
            ProviderKind::Anthropic => "https://console.anthropic.com/settings/keys",
            ProviderKind::OpenAI => "https://platform.openai.com/api-keys",
            ProviderKind::Ollama => "",
        })
        .dimmed()
    );
    println!();

    loop {
        let key = prompt_input(&format!("  {key_name}: "))?;
        let trimmed = key.trim().to_string();

        if trimmed.is_empty() {
            println!("  {} API key cannot be empty.", "Error:".red());
            continue;
        }

        // Basic format validation
        match provider {
            ProviderKind::Anthropic => {
                if !trimmed.starts_with("sk-ant-") {
                    println!(
                        "  {} Key doesn't look like an Anthropic key (should start with 'sk-ant-').",
                        "Warning:".yellow()
                    );
                    if !confirm("  Use it anyway?")? {
                        continue;
                    }
                }
            }
            ProviderKind::OpenAI => {
                if !trimmed.starts_with("sk-") {
                    println!(
                        "  {} Key doesn't look like an OpenAI key (should start with 'sk-').",
                        "Warning:".yellow()
                    );
                    if !confirm("  Use it anyway?")? {
                        continue;
                    }
                }
            }
            ProviderKind::Ollama => {}
        }

        return Ok(trimmed);
    }
}

async fn validate_api_key(
    provider: &ProviderKind,
    api_key: &str,
    model: &str,
) -> anyhow::Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()?;

    match provider {
        ProviderKind::Anthropic => {
            let resp = client
                .post("https://api.anthropic.com/v1/messages")
                .header("x-api-key", api_key)
                .header("anthropic-version", "2023-06-01")
                .header("content-type", "application/json")
                .json(&json!({
                    "model": model,
                    "max_tokens": 10,
                    "messages": [{"role": "user", "content": "Hi"}]
                }))
                .send()
                .await?;

            if resp.status().is_success() {
                Ok(())
            } else {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let msg = body["error"]["message"]
                    .as_str()
                    .unwrap_or("Unknown error");
                anyhow::bail!("{msg}");
            }
        }
        ProviderKind::OpenAI => {
            let resp = client
                .post("https://api.openai.com/v1/chat/completions")
                .header("Authorization", format!("Bearer {api_key}"))
                .header("Content-Type", "application/json")
                .json(&json!({
                    "model": model,
                    "max_tokens": 10,
                    "messages": [{"role": "user", "content": "Hi"}]
                }))
                .send()
                .await?;

            if resp.status().is_success() {
                Ok(())
            } else {
                let body: serde_json::Value = resp.json().await.unwrap_or_default();
                let msg = body["error"]["message"]
                    .as_str()
                    .unwrap_or("Unknown error");
                anyhow::bail!("{msg}");
            }
        }
        ProviderKind::Ollama => validate_ollama().await,
    }
}

async fn validate_ollama() -> anyhow::Result<()> {
    let client = Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()?;

    let resp = client
        .get("http://localhost:11434/api/tags")
        .send()
        .await
        .map_err(|_| anyhow::anyhow!("Cannot connect to Ollama at localhost:11434"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        anyhow::bail!("Ollama returned status {}", resp.status())
    }
}

fn choose_model(provider: &ProviderKind, _default: &str) -> anyhow::Result<String> {
    println!("  {}", "Step 3: Choose your model".bright_white().bold());
    println!();

    match provider {
        ProviderKind::Anthropic => {
            println!("    {} claude-sonnet-4-20250514 (recommended)", "[1]".cyan());
            println!("    {} claude-opus-4-20250514", "[2]".cyan());
            println!("    {} claude-haiku-3-5-20241022 (fast, cheaper)", "[3]".cyan());
            println!("    {} Custom model name", "[4]".cyan());
        }
        ProviderKind::OpenAI => {
            println!("    {} gpt-4o (recommended)", "[1]".cyan());
            println!("    {} gpt-4o-mini (fast, cheaper)", "[2]".cyan());
            println!("    {} gpt-4-turbo", "[3]".cyan());
            println!("    {} Custom model name", "[4]".cyan());
        }
        ProviderKind::Ollama => {
            println!("    {} qwen3:8b (recommended, good for coding)", "[1]".cyan());
            println!("    {} llama3.3:latest", "[2]".cyan());
            println!("    {} deepseek-coder-v2:latest", "[3]".cyan());
            println!("    {} Custom model name (any Ollama model)", "[4]".cyan());
        }
    }

    println!();

    loop {
        let input = prompt_input("  Choice [1]: ")?;
        let trimmed = input.trim();

        let model = match provider {
            ProviderKind::Anthropic => match trimmed {
                "" | "1" => "claude-sonnet-4-20250514".to_string(),
                "2" => "claude-opus-4-20250514".to_string(),
                "3" => "claude-haiku-3-5-20241022".to_string(),
                "4" => prompt_custom_model()?,
                _ => {
                    println!("  {} Please enter 1-4.", "Invalid:".red());
                    continue;
                }
            },
            ProviderKind::OpenAI => match trimmed {
                "" | "1" => "gpt-4o".to_string(),
                "2" => "gpt-4o-mini".to_string(),
                "3" => "gpt-4-turbo".to_string(),
                "4" => prompt_custom_model()?,
                _ => {
                    println!("  {} Please enter 1-4.", "Invalid:".red());
                    continue;
                }
            },
            ProviderKind::Ollama => match trimmed {
                "" | "1" => "qwen3:8b".to_string(),
                "2" => "llama3.3:latest".to_string(),
                "3" => "deepseek-coder-v2:latest".to_string(),
                "4" => prompt_custom_model()?,
                _ => {
                    println!("  {} Please enter 1-4.", "Invalid:".red());
                    continue;
                }
            },
        };

        println!("  {} {}\n", "Selected:".dimmed(), model.cyan());
        return Ok(model);
    }
}

fn prompt_custom_model() -> anyhow::Result<String> {
    let custom = prompt_input("  Enter model name: ")?;
    let m = custom.trim().to_string();
    if m.is_empty() {
        anyhow::bail!("Model name cannot be empty.");
    }
    Ok(m)
}

fn choose_save_location() -> anyhow::Result<PathBuf> {
    println!(
        "  {}",
        "Step 4: Where to save configuration?".bright_white().bold()
    );
    println!();

    let local_path = env_file_path();
    let global_path = global_env_path();

    println!(
        "    {} Project directory ({})",
        "[1]".cyan(),
        local_path.display().to_string().dimmed()
    );
    println!(
        "    {} Global config ({})",
        "[2]".cyan(),
        global_path.display().to_string().dimmed()
    );
    println!();

    loop {
        let input = prompt_input("  Choice [1]: ")?;
        let trimmed = input.trim();

        match trimmed {
            "" | "1" => return Ok(local_path),
            "2" => {
                if let Some(parent) = global_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                return Ok(global_path);
            }
            _ => {
                println!("  {} Please enter 1 or 2.", "Invalid:".red());
            }
        }
    }
}

fn save_env_file(
    path: &PathBuf,
    provider: &ProviderKind,
    api_key: &str,
    model: &str,
) -> anyhow::Result<()> {
    let mut contents = String::new();

    // Preserve existing content if file exists (minus our keys)
    if path.exists() {
        let existing = std::fs::read_to_string(path)?;
        for line in existing.lines() {
            let key = line.split('=').next().unwrap_or("").trim();
            if !matches!(
                key,
                "RUSTCLAW_PROVIDER"
                    | "ANTHROPIC_API_KEY"
                    | "OPENAI_API_KEY"
                    | "RUSTCLAW_MODEL"
            ) {
                contents.push_str(line);
                contents.push('\n');
            }
        }
        if !contents.is_empty() && !contents.ends_with("\n\n") {
            contents.push('\n');
        }
    }

    // Add our config
    contents.push_str("# RustClaw Configuration\n");
    contents.push_str(&format!("RUSTCLAW_PROVIDER={}\n", provider));

    match provider {
        ProviderKind::Anthropic => {
            contents.push_str(&format!("ANTHROPIC_API_KEY={}\n", api_key));
        }
        ProviderKind::OpenAI => {
            contents.push_str(&format!("OPENAI_API_KEY={}\n", api_key));
        }
        ProviderKind::Ollama => {
            // No API key needed for Ollama
        }
    }

    contents.push_str(&format!("RUSTCLAW_MODEL={}\n", model));

    std::fs::write(path, contents)?;

    Ok(())
}

/// Prompt for a line of text input.
fn prompt_input(prompt: &str) -> anyhow::Result<String> {
    print!("{}", prompt);
    io::stdout().flush()?;

    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    Ok(input.trim_end_matches('\n').to_string())
}

/// Ask a yes/no confirmation question.
fn confirm(question: &str) -> anyhow::Result<bool> {
    loop {
        let input = prompt_input(&format!("{question} [y/N]: "))?;
        match input.trim().to_lowercase().as_str() {
            "y" | "yes" => return Ok(true),
            "n" | "no" | "" => return Ok(false),
            _ => println!("  {} Please enter y or n.", "Invalid:".red()),
        }
    }
}
