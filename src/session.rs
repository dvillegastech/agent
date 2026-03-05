use std::path::{Path, PathBuf};

use chrono::Local;
use serde::{Deserialize, Serialize};

use crate::error::{AgentError, Result};
use crate::types::Message;

/// A saved session that can be resumed.
#[derive(Debug, Serialize, Deserialize)]
pub struct SavedSession {
    pub id: String,
    pub created_at: String,
    pub updated_at: String,
    pub model: String,
    pub messages: Vec<Message>,
}

impl SavedSession {
    pub fn new(model: &str) -> Self {
        let now = Local::now().format("%Y-%m-%dT%H:%M:%S").to_string();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            created_at: now.clone(),
            updated_at: now,
            model: model.into(),
            messages: Vec::new(),
        }
    }
}

/// Get the sessions directory.
fn sessions_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rustclaw")
        .join("sessions")
}

/// Save a session to disk.
pub fn save_session(session: &SavedSession) -> Result<PathBuf> {
    let dir = sessions_dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        AgentError::Tool(format!("Failed to create sessions dir: {e}"))
    })?;

    let path = dir.join(format!("{}.json", session.id));
    let json = serde_json::to_string_pretty(session).map_err(|e| {
        AgentError::Tool(format!("Failed to serialize session: {e}"))
    })?;

    std::fs::write(&path, json).map_err(|e| {
        AgentError::Tool(format!("Failed to save session: {e}"))
    })?;

    Ok(path)
}

/// Load a session from disk by ID (prefix match supported).
#[allow(dead_code)]
pub fn load_session(id_prefix: &str) -> Result<SavedSession> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Err(AgentError::Tool("No sessions directory found".into()));
    }

    // Try exact match first
    let exact = dir.join(format!("{id_prefix}.json"));
    if exact.exists() {
        return load_session_file(&exact);
    }

    // Try prefix match
    let entries = std::fs::read_dir(&dir).map_err(|e| {
        AgentError::Tool(format!("Failed to read sessions dir: {e}"))
    })?;

    let mut matches: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| s.starts_with(id_prefix))
                .unwrap_or(false)
        })
        .collect();

    matches.sort();

    match matches.len() {
        0 => Err(AgentError::Tool(format!("No session matching '{id_prefix}'"))),
        1 => load_session_file(&matches[0]),
        n => Err(AgentError::Tool(format!(
            "Ambiguous session prefix '{id_prefix}' matches {n} sessions"
        ))),
    }
}

/// List recent sessions.
pub fn list_sessions(limit: usize) -> Result<Vec<(String, String, usize)>> {
    let dir = sessions_dir();
    if !dir.exists() {
        return Ok(Vec::new());
    }

    let mut sessions: Vec<(String, String, usize)> = Vec::new();

    let entries = std::fs::read_dir(&dir).map_err(|e| {
        AgentError::Tool(format!("Failed to read sessions dir: {e}"))
    })?;

    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) == Some("json") {
            if let Ok(session) = load_session_file(&path) {
                sessions.push((
                    session.id[..8].to_string(),
                    session.updated_at,
                    session.messages.len(),
                ));
            }
        }
    }

    sessions.sort_by(|a, b| b.1.cmp(&a.1)); // Most recent first
    sessions.truncate(limit);
    Ok(sessions)
}

fn load_session_file(path: &Path) -> Result<SavedSession> {
    let content = std::fs::read_to_string(path).map_err(|e| {
        AgentError::Tool(format!("Failed to read session file: {e}"))
    })?;
    serde_json::from_str(&content).map_err(|e| {
        AgentError::Tool(format!("Failed to parse session: {e}"))
    })
}
