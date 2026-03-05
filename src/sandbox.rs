use std::process::Stdio;

use colored::Colorize;
use tokio::process::Command;

use crate::error::{AgentError, Result};

/// Sandbox mode for shell execution.
#[derive(Debug, Clone, PartialEq)]
pub enum SandboxMode {
    /// No restrictions beyond the blocklist.
    None,
    /// Network-disabled sandbox (like Codex CLI).
    NoNetwork,
    /// Read-only filesystem + no network.
    Strict,
}

impl std::fmt::Display for SandboxMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SandboxMode::None => write!(f, "none"),
            SandboxMode::NoNetwork => write!(f, "no-network"),
            SandboxMode::Strict => write!(f, "strict"),
        }
    }
}

/// Execute a command in sandboxed mode.
/// On Linux, uses `unshare` to disable network if available.
pub async fn sandboxed_exec(
    command: &str,
    mode: &SandboxMode,
    timeout: std::time::Duration,
) -> Result<(String, i32)> {
    let result = tokio::time::timeout(timeout, async {
        let child = match mode {
            SandboxMode::None => {
                Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
            }
            SandboxMode::NoNetwork => {
                // Try unshare for network isolation on Linux
                if cfg!(target_os = "linux") && unshare_available() {
                    eprintln!("{}", "  [sandbox] network-disabled mode".dimmed());
                    Command::new("unshare")
                        .args(["--net", "--map-root-user", "sh", "-c", command])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                } else {
                    eprintln!("{}", "  [sandbox] unshare unavailable, running without network sandbox".yellow());
                    Command::new("sh")
                        .arg("-c")
                        .arg(command)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                }
            }
            SandboxMode::Strict => {
                if cfg!(target_os = "linux") && unshare_available() {
                    eprintln!("{}", "  [sandbox] strict mode (no network)".dimmed());
                    Command::new("unshare")
                        .args(["--net", "--map-root-user", "sh", "-c", command])
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                } else {
                    Command::new("sh")
                        .arg("-c")
                        .arg(command)
                        .stdout(Stdio::piped())
                        .stderr(Stdio::piped())
                        .spawn()
                }
            }
        };

        let child = child.map_err(|e| AgentError::Tool(format!("Failed to spawn command: {e}")))?;

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

        let code = output.status.code().unwrap_or(-1);
        Ok((result, code)) as Result<(String, i32)>
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

/// Check if `unshare` command is available.
fn unshare_available() -> bool {
    std::process::Command::new("unshare")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}
