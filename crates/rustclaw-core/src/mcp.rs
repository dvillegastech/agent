use std::collections::HashMap;
use std::process::Stdio;

use colored::Colorize;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};

use crate::error::{AgentError, Result};
use crate::types::ToolDefinition;

/// MCP server connection configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    #[serde(default)]
    pub env: HashMap<String, String>,
}

/// An active MCP server connection (stdio transport).
pub struct McpServer {
    pub name: String,
    child: Child,
    request_id: u64,
}

impl McpServer {
    /// Start an MCP server process and initialize it.
    pub async fn connect(config: &McpServerConfig) -> Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let child = cmd.spawn().map_err(|e| {
            AgentError::Tool(format!("Failed to start MCP server '{}': {e}", config.name))
        })?;

        let mut server = Self {
            name: config.name.clone(),
            child,
            request_id: 0,
        };

        // Initialize the server
        let _init_response = server
            .send_request(
                "initialize",
                json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": {
                        "name": "rustclaw",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                }),
            )
            .await?;

        // Send initialized notification
        server.send_notification("notifications/initialized", json!({})).await?;

        eprintln!(
            "{} {}",
            "  [mcp]".bright_magenta(),
            format!("Connected to '{}'", config.name).dimmed()
        );

        Ok(server)
    }

    /// List tools available from this MCP server.
    pub async fn list_tools(&mut self) -> Result<Vec<ToolDefinition>> {
        let response = self.send_request("tools/list", json!({})).await?;

        let tools = response["tools"]
            .as_array()
            .cloned()
            .unwrap_or_default();

        let definitions: Vec<ToolDefinition> = tools
            .into_iter()
            .filter_map(|t| {
                let name = t["name"].as_str()?.to_string();
                let description = t["description"].as_str().unwrap_or("").to_string();
                let input_schema = t["inputSchema"].clone();
                Some(ToolDefinition {
                    name: format!("mcp__{}__{}", self.name, name),
                    description: format!("[MCP:{}] {}", self.name, description),
                    input_schema,
                })
            })
            .collect();

        Ok(definitions)
    }

    /// Call a tool on this MCP server.
    pub async fn call_tool(&mut self, tool_name: &str, arguments: &Value) -> Result<String> {
        let response = self
            .send_request(
                "tools/call",
                json!({
                    "name": tool_name,
                    "arguments": arguments
                }),
            )
            .await?;

        // Extract text content from response
        let content = response["content"]
            .as_array()
            .map(|blocks| {
                blocks
                    .iter()
                    .filter_map(|b| b["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .unwrap_or_else(|| response.to_string());

        Ok(content)
    }

    async fn send_request(&mut self, method: &str, params: Value) -> Result<Value> {
        self.request_id += 1;
        let request = json!({
            "jsonrpc": "2.0",
            "id": self.request_id,
            "method": method,
            "params": params
        });

        let stdin = self.child.stdin.as_mut().ok_or_else(|| {
            AgentError::Tool("MCP server stdin not available".into())
        })?;

        let mut msg = serde_json::to_string(&request).map_err(|e| {
            AgentError::Tool(format!("Failed to serialize MCP request: {e}"))
        })?;
        msg.push('\n');

        stdin.write_all(msg.as_bytes()).await.map_err(|e| {
            AgentError::Tool(format!("Failed to write to MCP server: {e}"))
        })?;
        stdin.flush().await.map_err(|e| {
            AgentError::Tool(format!("Failed to flush MCP stdin: {e}"))
        })?;

        // Read response line
        let stdout = self.child.stdout.as_mut().ok_or_else(|| {
            AgentError::Tool("MCP server stdout not available".into())
        })?;

        let mut reader = BufReader::new(stdout);
        let mut line = String::new();

        // Read with timeout
        let read_result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            reader.read_line(&mut line),
        )
        .await;

        match read_result {
            Ok(Ok(_)) => {
                let response: Value = serde_json::from_str(&line).map_err(|e| {
                    AgentError::Tool(format!("Invalid MCP response JSON: {e}"))
                })?;

                if let Some(error) = response.get("error") {
                    return Err(AgentError::Tool(format!(
                        "MCP error: {}",
                        error["message"].as_str().unwrap_or("unknown")
                    )));
                }

                Ok(response["result"].clone())
            }
            Ok(Err(e)) => Err(AgentError::Tool(format!("Failed to read MCP response: {e}"))),
            Err(_) => Err(AgentError::Tool("MCP server response timed out".into())),
        }
    }

    async fn send_notification(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        let stdin = self.child.stdin.as_mut().ok_or_else(|| {
            AgentError::Tool("MCP server stdin not available".into())
        })?;

        let mut msg = serde_json::to_string(&notification).map_err(|e| {
            AgentError::Tool(format!("Failed to serialize MCP notification: {e}"))
        })?;
        msg.push('\n');

        stdin.write_all(msg.as_bytes()).await.map_err(|e| {
            AgentError::Tool(format!("Failed to write MCP notification: {e}"))
        })?;
        stdin.flush().await.map_err(|e| {
            AgentError::Tool(format!("Failed to flush MCP stdin: {e}"))
        })?;

        Ok(())
    }
}

impl Drop for McpServer {
    fn drop(&mut self) {
        // Best-effort kill the server process
        let _ = self.child.start_kill();
    }
}

/// Parse MCP tool name to extract server name and original tool name.
/// Format: mcp__servername__toolname
pub fn parse_mcp_tool_name(full_name: &str) -> Option<(&str, &str)> {
    let stripped = full_name.strip_prefix("mcp__")?;
    let sep = stripped.find("__")?;
    let server_name = &stripped[..sep];
    let tool_name = &stripped[sep + 2..];
    Some((server_name, tool_name))
}
