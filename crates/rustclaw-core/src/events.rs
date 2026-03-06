use serde::{Deserialize, Serialize};

/// Events emitted during agent processing for UI consumption.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    /// A text delta from the streaming LLM response.
    #[serde(rename = "text_delta")]
    TextDelta { text: String },

    /// A tool call is starting.
    #[serde(rename = "tool_start")]
    ToolStart {
        id: String,
        name: String,
        input: String,
    },

    /// A tool call finished with a result.
    #[serde(rename = "tool_result")]
    ToolResult {
        id: String,
        name: String,
        output: String,
        is_error: bool,
    },

    /// Token usage update.
    #[serde(rename = "usage")]
    Usage {
        input_tokens: u32,
        output_tokens: u32,
        total_input: u64,
        total_output: u64,
        estimated_cost: f64,
    },

    /// Agent processing is complete.
    #[serde(rename = "done")]
    Done { text: String },

    /// An error occurred.
    #[serde(rename = "error")]
    Error { message: String },
}

/// Callback for receiving agent events.
/// Implementations can send these to a UI, log them, etc.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: AgentEvent);
}

/// Default no-op sink for CLI (which uses stdout directly).
pub struct NoopSink;

impl EventSink for NoopSink {
    fn emit(&self, _event: AgentEvent) {}
}
