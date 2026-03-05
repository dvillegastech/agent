use std::process::Stdio;

use colored::Colorize;
use regex::Regex;
use serde_json::Value;
use tokio::process::Command;

use crate::error::{AgentError, Result};

use super::security::SecurityGuard;

/// Max directory recursion depth for search_files.
const MAX_SEARCH_DEPTH: usize = 20;

/// Directories to skip during search traversal.
const IGNORED_DIRS: &[&str] = &["node_modules", "target", "__pycache__", ".git"];

/// Extract a required string field from a JSON value.
fn required_str<'a>(input: &'a Value, key: &str) -> Result<&'a str> {
    input[key]
        .as_str()
        .ok_or_else(|| AgentError::Tool(format!("Missing '{key}' parameter")))
}

/// Ejecutor de herramientas con validación de seguridad.
pub struct ToolExecutor {
    guard: SecurityGuard,
}

impl ToolExecutor {
    pub fn new(guard: SecurityGuard) -> Self {
        Self { guard }
    }

    /// Ejecuta una herramienta por nombre con los argumentos dados.
    pub async fn execute(&self, name: &str, input: &Value) -> Result<String> {
        match name {
            "read_file" => self.read_file(input).await,
            "write_file" => self.write_file(input).await,
            "list_dir" => self.list_dir(input).await,
            "shell" => self.shell(input).await,
            "search_files" => self.search_files(input).await,
            _ => Err(AgentError::Tool(format!("Unknown tool: {name}"))),
        }
    }

    async fn read_file(&self, input: &Value) -> Result<String> {
        let path = required_str(input, "path")?;
        let resolved = self.guard.validate_path(path)?;

        let metadata = tokio::fs::metadata(&resolved).await.map_err(|e| {
            AgentError::Tool(format!("Cannot access '{}': {e}", resolved.display()))
        })?;

        self.guard.validate_file_size(metadata.len())?;

        let content = tokio::fs::read_to_string(&resolved).await.map_err(|e| {
            AgentError::Tool(format!("Cannot read '{}': {e}", resolved.display()))
        })?;

        Ok(content)
    }

    async fn write_file(&self, input: &Value) -> Result<String> {
        let path = required_str(input, "path")?;
        let content = required_str(input, "content")?;

        let resolved = self.guard.validate_path(path)?;
        self.guard.validate_file_size(content.len() as u64)?;

        if let Some(parent) = resolved.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(|e| {
                AgentError::Tool(format!("Cannot create directories: {e}"))
            })?;
        }

        tokio::fs::write(&resolved, content).await.map_err(|e| {
            AgentError::Tool(format!("Cannot write '{}': {e}", resolved.display()))
        })?;

        Ok(format!("Successfully wrote {} bytes to {}", content.len(), path))
    }

    async fn list_dir(&self, input: &Value) -> Result<String> {
        let path = input["path"].as_str().unwrap_or(".");
        let resolved = self.guard.validate_path(path)?;

        let mut entries = tokio::fs::read_dir(&resolved).await.map_err(|e| {
            AgentError::Tool(format!("Cannot list '{}': {e}", resolved.display()))
        })?;

        let mut result = Vec::new();
        while let Some(entry) = entries.next_entry().await.map_err(|e| {
            AgentError::Tool(format!("Error reading directory entry: {e}"))
        })? {
            let name = entry.file_name().to_string_lossy().into_owned();
            let file_type = entry.file_type().await.map_err(|e| {
                AgentError::Tool(format!("Error getting file type: {e}"))
            })?;

            if file_type.is_dir() {
                result.push(format!("{name}/"));
            } else {
                result.push(name);
            }
        }

        result.sort();
        Ok(result.join("\n"))
    }

    async fn shell(&self, input: &Value) -> Result<String> {
        let command = required_str(input, "command")?;
        self.guard.validate_command(command)?;

        eprintln!(
            "{} {}",
            "  [shell]".dimmed(),
            command.yellow()
        );

        let timeout = self.guard.command_timeout();

        let result = tokio::time::timeout(timeout, async {
            let child = Command::new("sh")
                .arg("-c")
                .arg(command)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .map_err(|e| AgentError::Tool(format!("Failed to spawn command: {e}")))?;

            // Read stdout and stderr concurrently to avoid deadlock
            let output = child.wait_with_output().await.map_err(|e| {
                AgentError::Tool(format!("Failed to wait for command: {e}"))
            })?;

            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);

            let mut result = String::new();
            if !stdout.is_empty() {
                result.push_str(&stdout);
            }
            if !stderr.is_empty() {
                if !result.is_empty() {
                    result.push('\n');
                }
                result.push_str("[stderr]\n");
                result.push_str(&stderr);
            }

            if !output.status.success() {
                result.push_str(&format!(
                    "\n[exit code: {}]",
                    output.status.code().unwrap_or(-1)
                ));
            }

            // Truncate excessive output (UTF-8 safe)
            const MAX_OUTPUT: usize = 50_000;
            if result.len() > MAX_OUTPUT {
                // Find valid UTF-8 boundary at or before MAX_OUTPUT
                let mut end = MAX_OUTPUT;
                while end > 0 && !result.is_char_boundary(end) {
                    end -= 1;
                }
                result.truncate(end);
                result.push_str("\n... [output truncated]");
            }

            Ok(result) as Result<String>
        })
        .await;

        match result {
            Ok(inner) => inner,
            Err(_) => Err(AgentError::Tool(format!(
                "Command timed out after {} seconds",
                timeout.as_secs()
            ))),
        }
    }

    async fn search_files(&self, input: &Value) -> Result<String> {
        let pattern = required_str(input, "pattern")?;
        let path = input["path"].as_str().unwrap_or(".");
        let file_glob = input["file_glob"].as_str();

        let resolved = self.guard.validate_path(path)?;

        let re = Regex::new(pattern).map_err(|e| {
            AgentError::Tool(format!("Invalid regex pattern: {e}"))
        })?;

        // Pre-compile glob regex once if provided
        let glob_re = file_glob.map(|g| compile_glob(g)).transpose()?;

        let mut results = Vec::new();
        // Stack holds (path, depth)
        let mut stack = vec![(resolved, 0usize)];

        while let Some((dir, depth)) = stack.pop() {
            if depth > MAX_SEARCH_DEPTH {
                continue;
            }

            let mut entries = match tokio::fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };

            while let Ok(Some(entry)) = entries.next_entry().await {
                let entry_path = entry.path();
                let file_type = match entry.file_type().await {
                    Ok(ft) => ft,
                    Err(_) => continue,
                };

                if file_type.is_dir() {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !name.starts_with('.') && !IGNORED_DIRS.contains(&name.as_str()) {
                        stack.push((entry_path, depth + 1));
                    }
                } else if file_type.is_file() {
                    // Check glob filter
                    if let Some(ref glob) = glob_re {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        if !glob.is_match(&name) {
                            continue;
                        }
                    }

                    // Check file size before reading
                    if let Ok(meta) = tokio::fs::metadata(&entry_path).await {
                        if meta.len() > self.guard.max_file_size() {
                            continue;
                        }
                    }

                    if let Ok(content) = tokio::fs::read_to_string(&entry_path).await {
                        for (i, line) in content.lines().enumerate() {
                            if re.is_match(line) {
                                results.push(format!(
                                    "{}:{}:{}",
                                    entry_path.display(),
                                    i + 1,
                                    line.trim()
                                ));

                                if results.len() >= 200 {
                                    results.push("... [results truncated at 200 matches]".into());
                                    return Ok(results.join("\n"));
                                }
                            }
                        }
                    }
                }
            }
        }

        if results.is_empty() {
            Ok("No matches found.".into())
        } else {
            Ok(results.join("\n"))
        }
    }
}

/// Compile a simple glob pattern (supports * and ?) into a Regex.
fn compile_glob(pattern: &str) -> Result<Regex> {
    let re_pattern = format!(
        "^{}$",
        regex::escape(pattern)
            .replace(r"\*", ".*")
            .replace(r"\?", ".")
    );
    Regex::new(&re_pattern).map_err(|e| {
        AgentError::Tool(format!("Invalid glob pattern: {e}"))
    })
}
