use async_trait::async_trait;
use reqwest::Client;
use serde_json::{json, Value};

use crate::config::AgentConfig;
use crate::error::{AgentError, Result};
use crate::types::*;

use super::LlmProvider;

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
    base_url: String,
    max_tokens: u32,
    temperature: f32,
}

impl AnthropicProvider {
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

    fn build_messages(&self, messages: &[Message]) -> Vec<Value> {
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

    fn build_tools(&self, tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.input_schema,
                })
            })
            .collect()
    }

    fn parse_response(&self, body: &Value) -> Result<LlmResponse> {
        let content_arr = body["content"]
            .as_array()
            .ok_or_else(|| AgentError::Provider("Missing 'content' in response".into()))?;

        let mut content = Vec::new();
        for block in content_arr {
            match block["type"].as_str() {
                Some("text") => {
                    let text = block["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_string();
                    content.push(ContentBlock::Text { text });
                }
                Some("tool_use") => {
                    let id = block["id"].as_str().unwrap_or_default().to_string();
                    let name = block["name"].as_str().unwrap_or_default().to_string();
                    let input = block["input"].clone();
                    content.push(ContentBlock::ToolUse { id, name, input });
                }
                _ => {}
            }
        }

        let usage = Usage::from_json(&body["usage"], "input_tokens", "output_tokens");

        Ok(LlmResponse { content, usage })
    }
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Result<LlmResponse> {
        let url = format!("{}/v1/messages", self.base_url);

        let mut body = json!({
            "model": self.model,
            "max_tokens": self.max_tokens,
            "temperature": self.temperature,
            "system": system,
            "messages": self.build_messages(messages),
        });

        if !tools.is_empty() {
            body["tools"] = json!(self.build_tools(tools));
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
        let resp_body: Value = resp.json().await?;

        if !status.is_success() {
            let error_msg = resp_body["error"]["message"]
                .as_str()
                .unwrap_or("Unknown API error");
            return Err(AgentError::Provider(format!(
                "Anthropic API error ({status}): {error_msg}"
            )));
        }

        self.parse_response(&resp_body)
    }
}
