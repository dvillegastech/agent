use std::io::{self, Write};

use eventsource_stream::Eventsource;
use futures::StreamExt;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::{AgentConfig, ProviderKind};
use crate::error::{AgentError, Result};
use crate::types::*;

/// Streaming chat client that prints tokens as they arrive.
pub struct StreamingClient {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
    temperature: f32,
    provider: ProviderKind,
}

impl StreamingClient {
    pub fn new(config: &AgentConfig) -> Self {
        Self {
            client: Client::new(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            base_url: config.provider_url(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
            provider: config.provider.clone(),
        }
    }

    /// Stream a chat response, printing text deltas in real-time.
    /// Returns the complete LlmResponse when done.
    pub async fn stream_chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Result<LlmResponse> {
        match self.provider {
            ProviderKind::Anthropic => self.stream_anthropic(messages, tools, system).await,
            ProviderKind::OpenAI => self.stream_openai(messages, tools, system).await,
        }
    }

    async fn stream_anthropic(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Result<LlmResponse> {
        let url = format!("{}/v1/messages", self.base_url);

        let api_messages = build_anthropic_messages(messages);
        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "system": system,
            "messages": api_messages,
            "stream": true,
        });

        if !tools.is_empty() {
            let tools_json: Vec<Value> = tools
                .iter()
                .map(|t| json!({"name": t.name, "description": t.description, "input_schema": t.input_schema}))
                .collect();
            body["tools"] = json!(tools_json);
        }

        let resp = self
            .client
            .post(&url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_body: Value = resp.json().await.unwrap_or_default();
            let msg = err_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            return Err(AgentError::Provider(format!(
                "Anthropic API error ({status}): {msg}"
            )));
        }

        let mut content_blocks: Vec<ContentBlock> = Vec::new();
        let mut current_text = String::new();
        let mut current_tool_id = String::new();
        let mut current_tool_name = String::new();
        let mut current_tool_input = String::new();
        let mut usage = None;
        let mut in_tool = false;

        let mut stream = resp.bytes_stream().eventsource();

        while let Some(event) = stream.next().await {
            let event = match event {
                Ok(e) => e,
                Err(_) => continue,
            };

            if event.data == "[DONE]" {
                break;
            }

            let data: Value = match serde_json::from_str(&event.data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            match event.event.as_str() {
                "content_block_start" => {
                    let block = &data["content_block"];
                    match block["type"].as_str() {
                        Some("text") => {
                            in_tool = false;
                        }
                        Some("tool_use") => {
                            // Flush any accumulated text
                            if !current_text.is_empty() {
                                content_blocks.push(ContentBlock::Text {
                                    text: current_text.clone(),
                                });
                                current_text.clear();
                            }
                            in_tool = true;
                            current_tool_id = block["id"].as_str().unwrap_or_default().to_string();
                            current_tool_name =
                                block["name"].as_str().unwrap_or_default().to_string();
                            current_tool_input.clear();
                        }
                        _ => {}
                    }
                }
                "content_block_delta" => {
                    let delta = &data["delta"];
                    match delta["type"].as_str() {
                        Some("text_delta") => {
                            if let Some(text) = delta["text"].as_str() {
                                current_text.push_str(text);
                                // Print text delta in real-time
                                if !in_tool {
                                    print!("{}", text);
                                    let _ = io::stdout().flush();
                                }
                            }
                        }
                        Some("input_json_delta") => {
                            if let Some(partial) = delta["partial_json"].as_str() {
                                current_tool_input.push_str(partial);
                            }
                        }
                        _ => {}
                    }
                }
                "content_block_stop" => {
                    if in_tool {
                        let input: Value = serde_json::from_str(&current_tool_input)
                            .unwrap_or(json!({}));
                        content_blocks.push(ContentBlock::ToolUse {
                            id: current_tool_id.clone(),
                            name: current_tool_name.clone(),
                            input,
                        });
                        in_tool = false;
                    }
                }
                "message_delta" => {
                    if let (Some(inp), Some(out)) = (
                        data["usage"]["input_tokens"].as_u64(),
                        data["usage"]["output_tokens"].as_u64(),
                    ) {
                        usage = Some(Usage {
                            input_tokens: inp as u32,
                            output_tokens: out as u32,
                        });
                    }
                }
                "message_start" => {
                    if let Some(u) = Usage::from_json(&data["message"]["usage"], "input_tokens", "output_tokens") {
                        usage = Some(u);
                    }
                }
                _ => {}
            }
        }

        // Flush remaining text
        if !current_text.is_empty() {
            content_blocks.push(ContentBlock::Text {
                text: current_text,
            });
        }

        // Newline after streaming
        if content_blocks.iter().any(|b| matches!(b, ContentBlock::Text { .. })) {
            println!();
        }

        Ok(LlmResponse {
            content: content_blocks,
            usage,
        })
    }

    async fn stream_openai(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Result<LlmResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let api_messages = build_openai_messages(messages, system);
        let tools_json: Vec<Value> = tools
            .iter()
            .map(|t| {
                json!({"type": "function", "function": {"name": t.name, "description": t.description, "parameters": t.input_schema}})
            })
            .collect();

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "messages": api_messages,
            "stream": true,
        });

        if !tools.is_empty() {
            body["tools"] = json!(tools_json);
        }

        let resp = self
            .client
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let err_body: Value = resp.json().await.unwrap_or_default();
            let msg = err_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            return Err(AgentError::Provider(format!(
                "OpenAI API error ({status}): {msg}"
            )));
        }

        let mut text = String::new();
        let mut tool_calls: Vec<(String, String, String)> = Vec::new(); // (id, name, args)
        let mut usage = None;

        let mut stream = resp.bytes_stream().eventsource();

        while let Some(event) = stream.next().await {
            let event = match event {
                Ok(e) => e,
                Err(_) => continue,
            };

            if event.data == "[DONE]" {
                break;
            }

            let data: Value = match serde_json::from_str(&event.data) {
                Ok(v) => v,
                Err(_) => continue,
            };

            if let Some(choices) = data["choices"].as_array() {
                if let Some(choice) = choices.first() {
                    let delta = &choice["delta"];

                    // Text content
                    if let Some(content) = delta["content"].as_str() {
                        text.push_str(content);
                        print!("{}", content);
                        let _ = io::stdout().flush();
                    }

                    // Tool calls
                    if let Some(tcs) = delta["tool_calls"].as_array() {
                        for tc in tcs {
                            let index = tc["index"].as_u64().unwrap_or(0) as usize;

                            // Grow vector if needed
                            while tool_calls.len() <= index {
                                tool_calls.push((String::new(), String::new(), String::new()));
                            }

                            if let Some(id) = tc["id"].as_str() {
                                tool_calls[index].0 = id.to_string();
                            }
                            if let Some(name) = tc["function"]["name"].as_str() {
                                tool_calls[index].1 = name.to_string();
                            }
                            if let Some(args) = tc["function"]["arguments"].as_str() {
                                tool_calls[index].2.push_str(args);
                            }
                        }
                    }
                }
            }

            // Usage in final chunk
            if let Some(u) = Usage::from_json(&data["usage"], "prompt_tokens", "completion_tokens")
            {
                usage = Some(u);
            }
        }

        let mut content = Vec::new();

        if !text.is_empty() {
            println!(); // Newline after streaming
            content.push(ContentBlock::Text { text });
        }

        for (id, name, args) in tool_calls {
            if !name.is_empty() {
                let input: Value = serde_json::from_str(&args).unwrap_or(json!({}));
                content.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        Ok(LlmResponse { content, usage })
    }
}

// ─── Helper functions for building API messages ─────────────────────

fn build_anthropic_messages(messages: &[Message]) -> Vec<Value> {
    messages
        .iter()
        .map(|m| {
            let role = m.role.as_str();
            let content = match &m.content {
                MessageContent::Text(text) => json!(text),
                MessageContent::Blocks(blocks) => {
                    let serialized: Vec<Value> = blocks
                        .iter()
                        .map(|b| serde_json::to_value(b).unwrap_or(json!(null)))
                        .collect();
                    json!(serialized)
                }
            };
            json!({ "role": role, "content": content })
        })
        .collect()
}

fn build_openai_messages(messages: &[Message], system: &str) -> Vec<Value> {
    let mut msgs = vec![json!({"role": "system", "content": system})];

    for m in messages {
        match &m.content {
            MessageContent::Text(text) => {
                msgs.push(json!({ "role": m.role.as_str(), "content": text }));
            }
            MessageContent::Blocks(blocks) => {
                for block in blocks {
                    match block {
                        ContentBlock::Text { text } => {
                            msgs.push(json!({ "role": m.role.as_str(), "content": text }));
                        }
                        ContentBlock::ToolUse { id, name, input } => {
                            msgs.push(json!({
                                "role": "assistant",
                                "content": null,
                                "tool_calls": [{"id": id, "type": "function", "function": {"name": name, "arguments": input.to_string()}}]
                            }));
                        }
                        ContentBlock::ToolResult { tool_use_id, content, .. } => {
                            msgs.push(json!({"role": "tool", "tool_call_id": tool_use_id, "content": content}));
                        }
                    }
                }
            }
        }
    }

    msgs
}
