use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::AgentConfig;
use crate::error::{AgentError, Result};
use crate::types::*;

use super::LlmProvider;

pub struct OpenAIProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
    temperature: f32,
}

impl OpenAIProvider {
    pub fn new(config: &AgentConfig) -> Self {
        Self {
            client: Client::new(),
            api_key: config.api_key.clone(),
            model: config.model.clone(),
            base_url: config.provider_url(),
            max_tokens: config.max_tokens,
            temperature: config.temperature,
        }
    }

    fn build_messages(&self, messages: &[Message], system: &str) -> Vec<Value> {
        let mut msgs = vec![json!({
            "role": "system",
            "content": system,
        })];

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
                                    "tool_calls": [{
                                        "id": id,
                                        "type": "function",
                                        "function": {
                                            "name": name,
                                            "arguments": input.to_string(),
                                        }
                                    }]
                                }));
                            }
                            ContentBlock::ToolResult {
                                tool_use_id,
                                content,
                                ..
                            } => {
                                msgs.push(json!({
                                    "role": "tool",
                                    "tool_call_id": tool_use_id,
                                    "content": content,
                                }));
                            }
                        }
                    }
                }
            }
        }

        msgs
    }

    fn build_tools(&self, tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.input_schema,
                    }
                })
            })
            .collect()
    }

    fn parse_response(&self, body: &Value) -> Result<LlmResponse> {
        let choice = body["choices"]
            .as_array()
            .and_then(|c| c.first())
            .ok_or_else(|| AgentError::Provider("No choices in response".into()))?;

        let message = &choice["message"];
        let mut content = Vec::new();

        if let Some(text) = message["content"].as_str() {
            if !text.is_empty() {
                content.push(ContentBlock::Text {
                    text: text.to_string(),
                });
            }
        }

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            for tc in tool_calls {
                let id = tc["id"].as_str().unwrap_or_default().to_string();
                let name = tc["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string();
                let arguments = tc["function"]["arguments"]
                    .as_str()
                    .unwrap_or("{}");
                let input: Value =
                    serde_json::from_str(arguments).unwrap_or(json!({}));
                content.push(ContentBlock::ToolUse { id, name, input });
            }
        }

        let usage = Usage::from_json(&body["usage"], "prompt_tokens", "completion_tokens");

        Ok(LlmResponse { content, usage })
    }
}

#[async_trait]
impl LlmProvider for OpenAIProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Result<LlmResponse> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "messages": self.build_messages(messages, system),
        });

        if !tools.is_empty() {
            body["tools"] = json!(self.build_tools(tools));
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
        let resp_body: Value = resp.json().await?;

        if !status.is_success() {
            let error_msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            return Err(AgentError::Provider(format!(
                "OpenAI API error ({status}): {error_msg}"
            )));
        }

        self.parse_response(&resp_body)
    }
}
