//! Agent loop implementation

use std::io::{stdout, Write};

use anyhow::Result;
use llm_core::{
    ChatMessageWithTools, ChatOptions, FunctionDefinition as LlmFunctionDefinition, OllamaClient,
    Role, ToolDefinition as OllamaToolDefinition,
};
use tracing::{debug, info, instrument, warn};

use crate::tools::router::{RouteResult, ToolRouter};
use crate::tools::{ToolCall, ToolContext};

use super::state::{AgentConfig, AgentState, FailureTracker};

// ANSI colors
const GREEN: &str = "\x1b[92m";
const BLUE: &str = "\x1b[94m";
const YELLOW: &str = "\x1b[93m";
const CYAN: &str = "\x1b[96m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

/// The agent loop orchestrator
pub struct AgentLoop {
    client: OllamaClient,
    router: ToolRouter,
    config: AgentConfig,
}

impl AgentLoop {
    /// Create a new agent loop
    pub fn new(client: OllamaClient, router: ToolRouter, config: AgentConfig) -> Self {
        Self {
            client,
            router,
            config,
        }
    }

    /// Run the agent with a task
    #[instrument(skip(self), fields(model = %self.config.model))]
    pub async fn run(&self, task: &str) -> Result<AgentState> {
        info!(task_len = task.len(), max_iterations = self.config.max_iterations, "Starting agent loop");
        let mut state = AgentState::new();

        // Add system prompt if configured
        if let Some(ref system) = self.config.system_prompt {
            state.add_message(ChatMessageWithTools {
                role: Role::System,
                content: system.clone(),
                tool_calls: None,
                tool_call_id: None,
            });
        } else {
            // Default agent system prompt
            let default_system = self.default_system_prompt();
            state.add_message(ChatMessageWithTools {
                role: Role::System,
                content: default_system,
                tool_calls: None,
                tool_call_id: None,
            });
        }

        // Add the user task
        state.add_message(ChatMessageWithTools {
            role: Role::User,
            content: task.to_string(),
            tool_calls: None,
            tool_call_id: None,
        });

        // Get tool definitions
        let tool_defs = self.get_tool_definitions();

        // Create tool context
        let tool_ctx = ToolContext::new(self.config.working_dir.clone())
            .with_auto_mode(self.config.auto_mode);

        // Main agent loop
        while !state.finished && state.iteration < self.config.max_iterations {
            state.increment_iteration();
            debug!(iteration = state.iteration, messages = state.messages.len(), "Starting iteration");

            if self.config.verbose {
                print!(
                    "{}[Iteration {}]{} ",
                    DIM, state.iteration, RESET
                );
                stdout().flush()?;
            }

            // Call the LLM with tools
            debug!("Calling LLM with tools");
            let response = match self
                .client
                .chat_with_tools(
                    &self.config.model,
                    &state.messages,
                    Some(&tool_defs),
                    Some(ChatOptions::default()),
                )
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!(error = %e, "LLM request failed");
                    state.mark_error(format!("LLM error: {}", e));
                    break;
                }
            };

            let message = response.message;

            // Check if LLM wants to call tools
            if message.tool_calls.is_empty() {
                // No tool calls - LLM is done
                info!(iterations = state.iteration, "Agent completed task");
                if self.config.verbose {
                    println!("{}Done{}", GREEN, RESET);
                }
                state.mark_finished(message.content.clone());
                state.add_message(ChatMessageWithTools {
                    role: Role::Assistant,
                    content: message.content,
                    tool_calls: None,
                    tool_call_id: None,
                });
                break;
            }

            // Print assistant's thinking if present
            if !message.content.is_empty() && self.config.verbose {
                println!();
                println!("{}{}{}", DIM, message.content, RESET);
            }

            // Add assistant message with tool calls
            state.add_message(ChatMessageWithTools {
                role: Role::Assistant,
                content: message.content.clone(),
                tool_calls: Some(message.tool_calls.clone()),
                tool_call_id: None,
            });

            // Execute each tool call
            debug!(tool_count = message.tool_calls.len(), "Processing tool calls");
            for tool_call in &message.tool_calls {
                let call = ToolCall {
                    name: tool_call.function.name.clone(),
                    arguments: tool_call.function.arguments.clone(),
                };
                debug!(tool = %call.name, "Executing tool call");

                // Create signature for failure tracking
                let signature = FailureTracker::tool_signature(&call.name, &call.arguments);

                // Check if this is a repeated failing call
                if state.failure_tracker.is_repeated_call(&signature) {
                    let count = state.failure_tracker.failure_count(&signature);
                    if count > 0 && self.config.verbose {
                        println!(
                            "{}[Warning: This tool call has failed {} time(s)]{}",
                            YELLOW, count, RESET
                        );
                    }
                }

                if self.config.verbose {
                    println!();
                    print!(
                        "{}[Tool: {}]{} ",
                        CYAN, call.name, RESET
                    );
                    stdout().flush()?;
                }

                let result = self.router.route(&call, &tool_ctx).await;

                let (tool_result, is_success, should_abort) = match result {
                    RouteResult::Success(r) => {
                        if self.config.verbose {
                            if r.success {
                                println!("{}OK{}", GREEN, RESET);
                            } else {
                                println!("{}Failed{}", YELLOW, RESET);
                            }
                        }
                        (r.output.clone(), r.success, false)
                    }
                    RouteResult::Skipped => {
                        if self.config.verbose {
                            println!("{}Skipped{}", DIM, RESET);
                        }
                        ("Tool execution was skipped by user".to_string(), false, false)
                    }
                    RouteResult::Denied => {
                        if self.config.verbose {
                            println!("{}Denied{}", YELLOW, RESET);
                        }
                        ("Tool execution was denied by user".to_string(), false, false)
                    }
                    RouteResult::Aborted => {
                        if self.config.verbose {
                            println!("{}Aborted{}", YELLOW, RESET);
                        }
                        state.mark_error("Operation aborted by user".to_string());
                        ("Operation aborted".to_string(), false, true)
                    }
                    RouteResult::NotFound(name) => {
                        if self.config.verbose {
                            println!("{}Not found{}", YELLOW, RESET);
                        }
                        (format!("Tool not found: {}", name), false, false)
                    }
                    RouteResult::Error(e) => {
                        if self.config.verbose {
                            println!("{}Error{}", YELLOW, RESET);
                        }
                        (format!("Tool error: {}", e), false, false)
                    }
                };

                // Track success/failure for loop detection
                if is_success {
                    state.failure_tracker.record_success(&signature);
                } else {
                    if let Some(abort_reason) = state.failure_tracker.record_failure(&signature, &tool_result) {
                        warn!(
                            tool = %call.name,
                            failures = state.failure_tracker.failure_count(&signature),
                            "Aborting due to consecutive failures"
                        );
                        if self.config.verbose {
                            println!();
                            println!(
                                "{}[Abort]{} {}",
                                YELLOW, RESET, abort_reason
                            );
                        }
                        state.mark_error(abort_reason);
                        break;
                    }
                }

                // Add tool result to messages
                let tool_call_id = tool_call.id.clone();
                state.add_message(ChatMessageWithTools::tool_result(
                    if tool_call_id.is_empty() {
                        tool_call.function.name.clone()
                    } else {
                        tool_call_id
                    },
                    tool_result,
                ));

                if should_abort {
                    break;
                }
            }
        }

        // Check if we hit max iterations
        if !state.finished && state.iteration >= self.config.max_iterations {
            warn!(max_iterations = self.config.max_iterations, "Agent reached maximum iterations");
            state.mark_error(format!(
                "Agent reached maximum iterations ({})",
                self.config.max_iterations
            ));
        }

        info!(
            finished = state.finished,
            iterations = state.iteration,
            error = ?state.error,
            "Agent loop completed"
        );

        Ok(state)
    }

    fn default_system_prompt(&self) -> String {
        format!(
            r#"You are an AI assistant with access to tools for completing tasks. You can read files, search for content, execute commands, and more.

Working directory: {}

Available tools:
{}

Guidelines:
- Use tools to gather information before responding
- For file operations, prefer reading before writing
- For commands, explain what you're doing
- Be concise but thorough
- If a task is unclear, ask for clarification

When you have completed the task, provide a final summary response without calling any more tools."#,
            self.config.working_dir.display(),
            self.format_tool_list()
        )
    }

    fn format_tool_list(&self) -> String {
        self.router
            .registry()
            .all_tools()
            .iter()
            .map(|t| format!("- {}: {}", t.name(), t.description()))
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn get_tool_definitions(&self) -> Vec<OllamaToolDefinition> {
        self.router
            .registry()
            .all_tools()
            .iter()
            .map(|t| {
                let def = t.to_definition();
                OllamaToolDefinition {
                    tool_type: def.tool_type,
                    function: LlmFunctionDefinition {
                        name: def.function.name,
                        description: def.function.description,
                        parameters: serde_json::to_value(&def.function.parameters).unwrap_or_default(),
                    },
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::builtin::create_safe_registry;
    use crate::tools::security::AutoApprove;

    // Integration tests would require a running Ollama instance
    // Unit tests for the loop logic

    #[test]
    fn test_agent_config_builder() {
        let config = AgentConfig::new("test-model")
            .with_system_prompt("You are helpful")
            .with_max_iterations(10)
            .with_auto_mode(true);

        assert_eq!(config.model, "test-model");
        assert_eq!(config.system_prompt, Some("You are helpful".to_string()));
        assert_eq!(config.max_iterations, 10);
        assert!(config.auto_mode);
    }

    #[test]
    fn test_agent_state() {
        let mut state = AgentState::new();
        assert_eq!(state.iteration, 0);
        assert!(!state.finished);

        state.increment_iteration();
        assert_eq!(state.iteration, 1);

        state.mark_finished("Done".to_string());
        assert!(state.finished);
        assert_eq!(state.final_response, Some("Done".to_string()));
    }
}
