// Prevents additional console window on Windows in release
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::path::PathBuf;
use std::sync::Arc;

use chrono::Local;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, State};
use tokio::sync::Mutex;

use rustclaw_core::agent::runner::AgentRunner;
use rustclaw_core::config::{AgentConfig, ProviderKind};
use rustclaw_core::context;
use rustclaw_core::events::{AgentEvent, EventSink};
use rustclaw_core::session;

// ─── Event Sink for Tauri ──────────────────────────────────────────

/// Bridges core AgentEvents to Tauri's frontend event system.
struct TauriEventSink {
    app: AppHandle,
}

impl EventSink for TauriEventSink {
    fn emit(&self, event: AgentEvent) {
        let _ = self.app.emit("agent-event", &event);
    }
}

// ─── Shared State ──────────────────────────────────────────────────

struct AppState {
    runner: Mutex<Option<AgentRunner>>,
    config: Mutex<Option<AgentConfig>>,
    current_session_id: Mutex<Option<String>>,
}

// ─── Types for frontend ────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatResponse {
    success: bool,
    message: String,
    error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ConfigInfo {
    provider: String,
    model: String,
    configured: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionInfo {
    id: String,
    updated_at: String,
    message_count: usize,
    model: String,
}

// ─── Agent Commands ────────────────────────────────────────────────

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

    load_context_into(&mut config);

    let runner = AgentRunner::from_config(&config);
    let info = ConfigInfo {
        provider: config.provider.to_string(),
        model: config.model.clone(),
        configured: true,
    };

    // Create a new session
    let sess = session::SavedSession::new(&config.model);
    *state.current_session_id.lock().await = Some(sess.id.clone());

    *state.runner.lock().await = Some(runner);
    *state.config.lock().await = Some(config);

    Ok(info)
}

#[tauri::command]
async fn auto_load_config(
    state: State<'_, Arc<AppState>>,
) -> Result<ConfigInfo, String> {
    let _ = dotenvy::dotenv();

    match AgentConfig::from_env() {
        Ok(mut config) => {
            load_context_into(&mut config);

            let runner = AgentRunner::from_config(&config);
            let info = ConfigInfo {
                provider: config.provider.to_string(),
                model: config.model.clone(),
                configured: true,
            };

            let sess = session::SavedSession::new(&config.model);
            *state.current_session_id.lock().await = Some(sess.id.clone());

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

/// Send a message with streaming events emitted to the frontend.
#[tauri::command]
async fn send_message(
    state: State<'_, Arc<AppState>>,
    app: AppHandle,
    message: String,
) -> Result<ChatResponse, String> {
    let sink = TauriEventSink { app };

    let mut runner_guard = state.runner.lock().await;
    let runner = runner_guard
        .as_mut()
        .ok_or("Agent not initialized. Please configure first.")?;

    match runner.process_message_with_events(&message, &sink).await {
        Ok(response) => Ok(ChatResponse {
            success: true,
            message: response,
            error: None,
        }),
        Err(e) => {
            sink.emit(AgentEvent::Error {
                message: e.to_string(),
            });
            Ok(ChatResponse {
                success: false,
                message: String::new(),
                error: Some(e.to_string()),
            })
        }
    }
}

#[tauri::command]
async fn clear_conversation(
    state: State<'_, Arc<AppState>>,
) -> Result<(), String> {
    // Auto-save before clearing
    auto_save_session(&state).await;

    let mut runner_guard = state.runner.lock().await;
    if let Some(runner) = runner_guard.as_mut() {
        runner.clear_conversation();
    }

    // Start a new session
    let config_guard = state.config.lock().await;
    if let Some(config) = config_guard.as_ref() {
        let sess = session::SavedSession::new(&config.model);
        *state.current_session_id.lock().await = Some(sess.id.clone());
    }

    Ok(())
}

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

// ─── Session Commands ──────────────────────────────────────────────

#[tauri::command]
async fn save_session(
    state: State<'_, Arc<AppState>>,
) -> Result<String, String> {
    auto_save_session(&state).await;
    let session_id = state.current_session_id.lock().await;
    Ok(session_id.clone().unwrap_or_else(|| "none".into()))
}

#[tauri::command]
async fn list_sessions() -> Result<Vec<SessionInfo>, String> {
    let sessions = session::list_sessions(50).map_err(|e| e.to_string())?;
    Ok(sessions
        .into_iter()
        .map(|(id, updated_at, message_count)| SessionInfo {
            id,
            updated_at,
            message_count,
            model: String::new(),
        })
        .collect())
}

#[tauri::command]
async fn load_session(
    state: State<'_, Arc<AppState>>,
    session_id: String,
) -> Result<Vec<serde_json::Value>, String> {
    let saved = session::load_session(&session_id).map_err(|e| e.to_string())?;

    // Rebuild runner with saved messages
    let config_guard = state.config.lock().await;
    let config = config_guard
        .as_ref()
        .ok_or("Agent not configured")?;

    let runner = AgentRunner::from_config(config);

    // We need to replay the messages into the conversation
    // For now, just rebuild and set session ID
    drop(config_guard);

    // Convert messages to JSON for the frontend to display
    let messages_json: Vec<serde_json::Value> = saved
        .messages
        .iter()
        .filter_map(|m| serde_json::to_value(m).ok())
        .collect();

    *state.current_session_id.lock().await = Some(saved.id);

    // Clear and set the new runner
    *state.runner.lock().await = Some(runner);

    Ok(messages_json)
}

#[tauri::command]
async fn delete_session(session_id: String) -> Result<(), String> {
    let dir = dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustclaw")
        .join("sessions");

    // Find and delete the session file
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                if stem.starts_with(&session_id) {
                    std::fs::remove_file(&path).map_err(|e| e.to_string())?;
                    return Ok(());
                }
            }
        }
    }

    Err(format!("Session '{session_id}' not found"))
}

// ─── Helpers ───────────────────────────────────────────────────────

fn load_context_into(config: &mut AgentConfig) {
    if let Some(project_ctx) = context::load_project_context() {
        config
            .system_prompt
            .push_str("\n\n--- Project Instructions (from RUSTCLAW.md) ---\n");
        config.system_prompt.push_str(&project_ctx);
    }

    // Skip RAG indexing in desktop app — cwd is typically / or $HOME,
    // which would scan the entire filesystem and hang on startup.
}

async fn auto_save_session(state: &AppState) {
    let runner_guard = state.runner.lock().await;
    let session_id_guard = state.current_session_id.lock().await;
    let config_guard = state.config.lock().await;

    if let (Some(runner), Some(session_id), Some(config)) = (
        runner_guard.as_ref(),
        session_id_guard.as_ref(),
        config_guard.as_ref(),
    ) {
        let messages = runner.get_messages();
        if messages.is_empty() {
            return;
        }

        let saved = session::SavedSession {
            id: session_id.clone(),
            created_at: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            updated_at: Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            model: config.model.clone(),
            messages: messages.to_vec(),
        };

        let _ = session::save_session(&saved);
    }
}

// ─── Main ──────────────────────────────────────────────────────────

fn main() {
    let state = Arc::new(AppState {
        runner: Mutex::new(None),
        config: Mutex::new(None),
        current_session_id: Mutex::new(None),
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
            save_session,
            list_sessions,
            load_session,
            delete_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
