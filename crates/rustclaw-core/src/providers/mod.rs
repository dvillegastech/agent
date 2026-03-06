pub mod anthropic;
pub mod openai;

use async_trait::async_trait;
use crate::error::Result;
use crate::types::{LlmResponse, Message, ToolDefinition};

/// Trait que deben implementar todos los proveedores LLM.
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Envía mensajes al LLM y obtiene una respuesta.
    async fn chat(
        &self,
        messages: &[Message],
        tools: &[ToolDefinition],
        system: &str,
    ) -> Result<LlmResponse>;
}
