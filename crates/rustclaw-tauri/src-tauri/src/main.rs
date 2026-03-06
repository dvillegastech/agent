// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tauri::State;
use tokio::sync::Mutex;

use rustclaw_core::agent::runner::AgentRunner;
use rustclaw_core::config::{AgentConfig, ProviderKind};
use rustclaw_core::context;
use rustclaw_core::rag;

/// Shared application state.
struct AppState {
    runner: Mutex<Option<AgentRunner>>,
    config: Mutex<Option<AgentConfig>>,
}

/// Chat message for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

/// Response sent back to the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatResponse {
    success: bool,
    message: String,
    error: Option<String>,
}

/// Configuration info for the frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigInfo {
    provider: String,
    model: String,
    configured: bool,
}

/// Initialize the agent with the given configuration.
#[tauri::command]
async fn initialize_agent(
    state: State<'_, Arc<AppState>>,
    provider: String,
    api_key: String,
    model: String,
) -> Result<ConfigInfo, String> {
    let provider_kind = match provider.to_lowercase().as_str() {
        "openai" => ProviderKind::OpenAI,
        "ollama" => ProviderKind::Ollama,
        _ => ProviderKind::Anthropic,
    };

    let mut config = AgentConfig {
        provider: provider_kind,
        api_key,
        model: model.clone(),
        ..AgentConfig::default()
    };

    // Load project context
    if let Some(project_ctx) = context::load_project_context() {
        config.system_prompt.push_str("\n\n--- Project Instructions (from RUSTCLAW.md) ---\n");
        config.system_prompt.push_str(&project_ctx);
    }

    // Build codebase index
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let index = rag::CodebaseIndex::build(&cwd);
    if !index.entries.is_empty() {
        config.system_prompt.push_str(&index.summary());
    }

    let runner = AgentRunner::from_config(&config);

    let info = ConfigInfo {
        provider: config.provider.to_string(),
        model: config.model.clone(),
        configured: true,
    };

    *state.runner.lock().await = Some(runner);
    *state.config.lock().await = Some(config);

    Ok(info)
}

/// Try to auto-load config from environment variables.
#[tauri::command]
async fn auto_load_config(
    state: State<'_, Arc<AppState>>,
) -> Result<ConfigInfo, String> {
    let _ = dotenvy::dotenv();

    match AgentConfig::from_env() {
        Ok(mut config) => {
            // Load project context
            if let Some(project_ctx) = context::load_project_context() {
                config.system_prompt.push_str("\n\n--- Project Instructions (from RUSTCLAW.md) ---\n");
                config.system_prompt.push_str(&project_ctx);
            }

            // Build codebase index
            let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
            let index = rag::CodebaseIndex::build(&cwd);
            if !index.entries.is_empty() {
                config.system_prompt.push_str(&index.summary());
            }

            let runner = AgentRunner::from_config(&config);

            let info = ConfigInfo {
                provider: config.provider.to_string(),
                model: config.model.clone(),
                configured: true,
            };

            *state.runner.lock().await = Some(runner);
            *state.config.lock().await = Some(config);

            Ok(info)
        }
        Err(_) => Ok(ConfigInfo {
            provider: String::new(),
            model: String::new(),
            configured: false,
        }),
    }
}

/// Send a message to the agent and get a response.
#[tauri::command]
async fn send_message(
    state: State<'_, Arc<AppState>>,
    message: String,
) -> Result<ChatResponse, String> {
    let mut runner_guard = state.runner.lock().await;
    let runner = runner_guard.as_mut().ok_or("Agent not initialized. Please configure first.")?;

    match runner.process_message(&message).await {
        Ok(response) => Ok(ChatResponse {
            success: true,
            message: response,
            error: None,
        }),
        Err(e) => Ok(ChatResponse {
            success: false,
            message: String::new(),
            error: Some(e.to_string()),
        }),
    }
}

/// Clear the conversation history.
#[tauri::command]
async fn clear_conversation(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    let mut runner_guard = state.runner.lock().await;
    if let Some(runner) = runner_guard.as_mut() {
        runner.clear_conversation();
    }
    Ok(())
}

/// Get conversation stats.
#[tauri::command]
async fn get_stats(
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let runner_guard = state.runner.lock().await;
    match runner_guard.as_ref() {
        Some(runner) => Ok(runner.stats()),
        None => Ok("No active session".into()),
    }
}

/// Get cost summary.
#[tauri::command]
async fn get_cost(
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    let runner_guard = state.runner.lock().await;
    match runner_guard.as_ref() {
        Some(runner) => Ok(runner.cost_summary()),
        None => Ok("No active session".into()),
    }
}

fn main() {
    let state = Arc::new(AppState {
        runner: Mutex::new(None),
        config: Mutex::new(None),
    });

    tauri::Builder::default()
        .manage(state)
        .invoke_handler(tauri::generate_handler![
            initialize_agent,
            auto_load_config,
            send_message,
            clear_conversation,
            get_stats,
            get_cost,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
