use colored::Colorize;
use futures::future::join_all;

use crate::config::AgentConfig;
use crate::error::Result;
use crate::providers::LlmProvider;
use crate::tools::all_tool_definitions;
use crate::tools::executor::ToolExecutor;
use crate::types::*;

use super::conversation::Conversation;

/// El agente principal que coordina LLM, herramientas y conversación.
pub struct AgentRunner {
    provider: Box<dyn LlmProvider>,
    executor: ToolExecutor,
    conversation: Conversation,
    tools: Vec<ToolDefinition>,
    system_prompt: String,
    max_tool_iterations: usize,
}

impl AgentRunner {
    pub fn new(
        config: &AgentConfig,
        provider: Box<dyn LlmProvider>,
        executor: ToolExecutor,
    ) -> Self {
        Self {
            provider,
            executor,
            conversation: Conversation::new(config.max_conversation_turns),
            tools: all_tool_definitions(),
            system_prompt: config.system_prompt.clone(),
            max_tool_iterations: config.max_tool_iterations,
        }
    }

    /// Procesa un mensaje del usuario y retorna la respuesta final.
    pub async fn process_message(&mut self, user_input: &str) -> Result<String> {
        self.conversation.add_user_message(user_input);

        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > self.max_tool_iterations {
                eprintln!(
                    "{}",
                    "  [warning] Maximum tool iterations reached, stopping.".yellow()
                );
                break;
            }

            let response = self
                .provider
                .chat(
                    self.conversation.messages(),
                    &self.tools,
                    &self.system_prompt,
                )
                .await?;

            if let Some(ref usage) = response.usage {
                eprintln!(
                    "{}",
                    format!(
                        "  [tokens] input: {} | output: {}",
                        usage.input_tokens, usage.output_tokens
                    )
                    .dimmed()
                );
            }

            // Single-pass decomposition: extract text and tool calls together
            let (text, tool_calls) = response.decompose();

            if tool_calls.is_empty() {
                // Final response with no tools - move content instead of cloning
                self.conversation
                    .add_assistant_message(MessageContent::Blocks(response.content));
                return Ok(if text.is_empty() {
                    "[No response from model]".into()
                } else {
                    text
                });
            }

            // Show partial text before executing tools
            if !text.is_empty() {
                println!("{}", text);
            }

            // Collect tool call info before moving response.content
            let tool_infos: Vec<_> = tool_calls
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::ToolUse { id, name, input } = *block {
                        eprintln!(
                            "{} {} {}",
                            "  [tool]".cyan(),
                            name.bright_cyan(),
                            format!("({})", truncate_str(&input.to_string(), 80)).dimmed()
                        );
                        Some((id.clone(), name.clone(), input.clone()))
                    } else {
                        None
                    }
                })
                .collect();

            // Store assistant response (move, not clone)
            self.conversation
                .add_assistant_message(MessageContent::Blocks(response.content));

            // Execute tools concurrently via join_all
            let executor = &self.executor;
            let tool_futures: Vec<_> = tool_infos
                .into_iter()
                .map(|(id, name, input)| {
                    let executor = &executor;
                    async move {
                        let result = executor.execute(&name, &input).await;
                        let (content, is_error) = match result {
                            Ok(output) => {
                                eprintln!(
                                    "{} {}",
                                    "  [result]".green(),
                                    truncate_str(&output, 200).dimmed()
                                );
                                (output, None)
                            }
                            Err(e) => {
                                let msg = e.to_string();
                                eprintln!("{} {}", "  [error]".red(), msg.red());
                                (msg, Some(true))
                            }
                        };
                        ContentBlock::ToolResult {
                            tool_use_id: id,
                            content,
                            is_error,
                        }
                    }
                })
                .collect();

            let tool_results = join_all(tool_futures).await;

            self.conversation
                .add_tool_results(MessageContent::Blocks(tool_results));
        }

        Ok("[Agent stopped after max iterations]".into())
    }

    /// Limpia la conversación.
    pub fn clear_conversation(&mut self) {
        self.conversation.clear();
    }

    /// Retorna estadísticas de la conversación.
    pub fn stats(&self) -> String {
        format!("Messages: {}", self.conversation.len())
    }
}

fn truncate_str(s: &str, max: usize) -> String {
    let replaced = s.replace('\n', " ");
    if replaced.len() <= max {
        replaced
    } else {
        format!("{}...", &replaced[..max])
    }
}
