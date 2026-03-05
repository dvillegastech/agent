use serde::{Deserialize, Serialize};

// ─── Mensajes de conversación ───────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::User => "user",
            Role::Assistant => "assistant",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: MessageContent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MessageContent {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

// ─── Definición de herramientas ─────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}

// ─── Respuesta del proveedor LLM ────────────────────────────────────

#[derive(Debug, Clone)]
pub struct LlmResponse {
    pub content: Vec<ContentBlock>,
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

impl Usage {
    /// Parse usage from a JSON value with configurable key names.
    pub fn from_json(value: &serde_json::Value, input_key: &str, output_key: &str) -> Option<Self> {
        let input_tokens = value[input_key].as_u64()? as u32;
        let output_tokens = value[output_key].as_u64()? as u32;
        Some(Self { input_tokens, output_tokens })
    }
}

impl LlmResponse {
    /// Extract text and detect tool calls in a single pass.
    pub fn decompose(&self) -> (String, Vec<&ContentBlock>) {
        let mut text = String::new();
        let mut tool_calls = Vec::new();
        for block in &self.content {
            match block {
                ContentBlock::Text { text: t } => text.push_str(t),
                ContentBlock::ToolUse { .. } => tool_calls.push(block),
                _ => {}
            }
        }
        (text, tool_calls)
    }
}
