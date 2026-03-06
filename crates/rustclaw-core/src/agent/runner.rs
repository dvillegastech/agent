use colored::Colorize;
use futures::future::join_all;

use crate::config::AgentConfig;
use crate::cost::CostTracker;
use crate::error::Result;
use crate::events::{AgentEvent, EventSink};
use crate::export;
use crate::retry;
use crate::streaming::StreamingClient;
use crate::tools::all_tool_definitions;
use crate::tools::executor::ToolExecutor;
use crate::tools::security::SecurityGuard;
use crate::types::*;
use crate::utils;

use super::conversation::Conversation;

/// The main agent that coordinates LLM, tools, and conversation.
pub struct AgentRunner {
    streaming_client: StreamingClient,
    executor: ToolExecutor,
    conversation: Conversation,
    tools: Vec<ToolDefinition>,
    system_prompt: String,
    max_tool_iterations: usize,
    cost_tracker: CostTracker,
}

impl AgentRunner {
    pub fn new(config: &AgentConfig, executor: ToolExecutor) -> Self {
        Self {
            streaming_client: StreamingClient::new(config),
            executor,
            conversation: Conversation::new(config.max_conversation_turns),
            tools: all_tool_definitions(),
            system_prompt: config.system_prompt.clone(),
            max_tool_iterations: config.max_tool_iterations,
            cost_tracker: CostTracker::new(&config.model),
        }
    }

    /// Create an AgentRunner directly from config, building SecurityGuard and ToolExecutor internally.
    pub fn from_config(config: &AgentConfig) -> Self {
        let guard = SecurityGuard::new(config.security.clone());
        let executor = ToolExecutor::new(guard);
        Self::new(config, executor)
    }

    /// Process a user message and return the final text response.
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

            // Use retry with backoff for the streaming call
            let messages = self.conversation.messages().to_vec();
            let tools = self.tools.clone();
            let system = self.system_prompt.clone();
            let client = &self.streaming_client;

            let response = retry::with_retry("LLM request", || {
                let msgs = messages.clone();
                let t = tools.clone();
                let s = system.clone();
                async move { client.stream_chat(&msgs, &t, &s).await }
            })
            .await?;

            // Track token usage
            if let Some(ref usage) = response.usage {
                self.cost_tracker
                    .record(usage.input_tokens, usage.output_tokens);
                self.cost_tracker
                    .print_update(usage.input_tokens, usage.output_tokens);
            }

            // Decompose response into text and tool calls
            let (text, tool_calls) = response.decompose();

            if tool_calls.is_empty() {
                // Final response - no tools needed
                self.conversation
                    .add_assistant_message(MessageContent::Blocks(response.content));
                return Ok(if text.is_empty() {
                    "[No response from model]".into()
                } else {
                    text
                });
            }

            // Show partial text before executing tools (already streamed to stdout)
            // Collect tool call info before moving response.content
            let tool_infos: Vec<_> = tool_calls
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::ToolUse { id, name, input } = *block {
                        eprintln!(
                            "{} {} {}",
                            "  [tool]".cyan(),
                            name.bright_cyan(),
                            format!("({})", utils::truncate_oneline(&input.to_string(), 80)).dimmed()
                        );
                        Some((id.clone(), name.clone(), input.clone()))
                    } else {
                        None
                    }
                })
                .collect();

            // Store assistant response
            self.conversation
                .add_assistant_message(MessageContent::Blocks(response.content));

            // Execute tools concurrently
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
                                    utils::truncate_oneline(&output, 200).dimmed()
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

    /// Process a user message with event streaming for UI consumption.
    pub async fn process_message_with_events(
        &mut self,
        user_input: &str,
        sink: &dyn EventSink,
    ) -> Result<String> {
        self.conversation.add_user_message(user_input);

        let mut iterations = 0;

        loop {
            iterations += 1;
            if iterations > self.max_tool_iterations {
                sink.emit(AgentEvent::Error {
                    message: "Maximum tool iterations reached".into(),
                });
                break;
            }

            let messages = self.conversation.messages().to_vec();
            let tools = self.tools.clone();
            let system = self.system_prompt.clone();
            let client = &self.streaming_client;

            let response = retry::with_retry("LLM request", || {
                let msgs = messages.clone();
                let t = tools.clone();
                let s = system.clone();
                async move { client.stream_chat_with_events(&msgs, &t, &s, sink).await }
            })
            .await?;

            // Track token usage
            if let Some(ref usage) = response.usage {
                self.cost_tracker
                    .record(usage.input_tokens, usage.output_tokens);
                sink.emit(AgentEvent::Usage {
                    input_tokens: usage.input_tokens,
                    output_tokens: usage.output_tokens,
                    total_input: self.cost_tracker.total_input_tokens,
                    total_output: self.cost_tracker.total_output_tokens,
                    estimated_cost: self.cost_tracker.estimated_cost(),
                });
            }

            let (text, tool_calls) = response.decompose();

            if tool_calls.is_empty() {
                self.conversation
                    .add_assistant_message(MessageContent::Blocks(response.content));
                let final_text = if text.is_empty() {
                    "[No response from model]".into()
                } else {
                    text
                };
                sink.emit(AgentEvent::Done {
                    text: final_text.clone(),
                });
                return Ok(final_text);
            }

            // Collect tool info and emit events
            let tool_infos: Vec<_> = tool_calls
                .iter()
                .filter_map(|block| {
                    if let ContentBlock::ToolUse { id, name, input } = *block {
                        sink.emit(AgentEvent::ToolStart {
                            id: id.clone(),
                            name: name.clone(),
                            input: utils::truncate_oneline(&input.to_string(), 200),
                        });
                        Some((id.clone(), name.clone(), input.clone()))
                    } else {
                        None
                    }
                })
                .collect();

            self.conversation
                .add_assistant_message(MessageContent::Blocks(response.content));

            // Execute tools
            let executor = &self.executor;
            let tool_futures: Vec<_> = tool_infos
                .into_iter()
                .map(|(id, name, input)| {
                    let executor = &executor;
                    async move {
                        let result = executor.execute(&name, &input).await;
                        let (content, is_error) = match result {
                            Ok(output) => (output, false),
                            Err(e) => (e.to_string(), true),
                        };
                        (id, name, content, is_error)
                    }
                })
                .collect();

            let results = join_all(tool_futures).await;

            let mut tool_result_blocks = Vec::new();
            for (id, name, content, is_error) in results {
                sink.emit(AgentEvent::ToolResult {
                    id: id.clone(),
                    name,
                    output: utils::truncate(&content, 500),
                    is_error,
                });
                tool_result_blocks.push(ContentBlock::ToolResult {
                    tool_use_id: id,
                    content,
                    is_error: if is_error { Some(true) } else { None },
                });
            }

            self.conversation
                .add_tool_results(MessageContent::Blocks(tool_result_blocks));
        }

        Ok("[Agent stopped after max iterations]".into())
    }

    /// Export the conversation to a file.
    pub fn export_conversation(&self, path: &std::path::Path, format: &str) -> Result<()> {
        match format {
            "json" => export::to_json(self.conversation.messages(), path),
            _ => export::to_markdown(self.conversation.messages(), path),
        }
    }

    /// Get cost tracking summary.
    pub fn cost_summary(&self) -> String {
        self.cost_tracker.summary()
    }

    /// Clear conversation history.
    pub fn clear_conversation(&mut self) {
        self.conversation.clear();
    }

    /// Get conversation statistics.
    pub fn stats(&self) -> String {
        format!(
            "Messages: {} | {}",
            self.conversation.len(),
            self.cost_tracker.summary()
        )
    }

    /// Get a reference to conversation messages (for session save).
    pub fn get_messages(&self) -> &[crate::types::Message] {
        self.conversation.messages()
    }
}

