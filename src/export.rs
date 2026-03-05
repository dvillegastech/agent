use std::path::Path;

use chrono::Local;

use crate::error::{AgentError, Result};
use crate::types::*;

/// Export conversation to a Markdown file.
pub fn to_markdown(messages: &[Message], path: &Path) -> Result<()> {
    let mut md = String::new();

    md.push_str(&format!(
        "# RustClaw Conversation\n\n_Exported: {}_\n\n---\n\n",
        Local::now().format("%Y-%m-%d %H:%M:%S")
    ));

    for msg in messages {
        let role_label = match msg.role {
            Role::User => "**User**",
            Role::Assistant => "**Assistant**",
        };

        md.push_str(&format!("### {}\n\n", role_label));

        match &msg.content {
            MessageContent::Text(text) => {
                md.push_str(text);
                md.push_str("\n\n");
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => {
                            md.push_str(text);
                            md.push_str("\n\n");
                        }
                        ContentBlock::ToolUse { name, input, .. } => {
                            md.push_str(&format!(
                                "**Tool call:** `{}`\n```json\n{}\n```\n\n",
                                name,
                                serde_json::to_string_pretty(input).unwrap_or_default()
                            ));
                        }
                        ContentBlock::ToolResult {
                            content, is_error, ..
                        } => {
                            let label = if is_error.unwrap_or(false) {
                                "Tool error"
                            } else {
                                "Tool result"
                            };
                            // Truncate very long results (UTF-8 safe)
                            let display = if content.len() > 2000 {
                                let end = content
                                    .char_indices()
                                    .take_while(|(i, _)| *i < 2000)
                                    .last()
                                    .map(|(i, c)| i + c.len_utf8())
                                    .unwrap_or(0);
                                format!("{}...\n\n_(truncated)_", &content[..end])
                            } else {
                                content.clone()
                            };
                            md.push_str(&format!(
                                "**{}:**\n```\n{}\n```\n\n",
                                label, display
                            ));
                        }
                    }
                }
            }
        }

        md.push_str("---\n\n");
    }

    std::fs::write(path, md).map_err(|e| {
        AgentError::Tool(format!("Failed to export conversation: {e}"))
    })
}

/// Export conversation to a JSON file.
pub fn to_json(messages: &[Message], path: &Path) -> Result<()> {
    let json = serde_json::to_string_pretty(messages).map_err(|e| {
        AgentError::Tool(format!("Failed to serialize conversation: {e}"))
    })?;

    std::fs::write(path, json).map_err(|e| {
        AgentError::Tool(format!("Failed to export conversation: {e}"))
    })
}
