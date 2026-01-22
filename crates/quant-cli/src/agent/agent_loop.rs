//! Agent loop implementation

use std::io::{stdout, Write};
use std::sync::Arc;

use anyhow::Result;
use futures::StreamExt;
use llm_core::{
    ChatMessageWithTools, ChatOptions, FunctionCall as LlmFunctionCall,
    FunctionDefinition as LlmFunctionDefinition, OllamaClient, Role, ToolCall as LlmToolCall,
    ToolDefinition as OllamaToolDefinition,
};
use tokio::sync::Mutex;
use tracing::{debug, info, instrument, warn};

use crate::context::{SmartContext, SmartContextSelector};
use crate::hooks::{HookContext, HookEvent, HookManager};
use crate::mcp::{McpManager, McpRegistryExt};
use crate::progress::Spinner;
use crate::project::ProjectContext;
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
    project_context: Option<ProjectContext>,
    hook_manager: HookManager,
    mcp_manager: Arc<Mutex<McpManager>>,
}

impl AgentLoop {
    /// Create a new agent loop
    pub fn new(client: OllamaClient, router: ToolRouter, config: AgentConfig) -> Self {
        // Auto-discover project context from working directory
        let project_context = ProjectContext::discover(&config.working_dir);
        if let Some(ref ctx) = project_context {
            info!(
                project = %ctx.name,
                project_type = %ctx.project_type,
                has_quant_md = ctx.quant_file.is_some(),
                "Discovered project context"
            );
        }

        // Initialize hook manager and load hooks from QUANT.md
        let mut hook_manager = HookManager::new();
        if let Some(ref ctx) = project_context {
            if let Some(ref quant_file) = ctx.quant_file {
                if let Ok(content) = std::fs::read_to_string(&quant_file.path) {
                    match hook_manager.load_from_quant_md(&content) {
                        Ok(count) if count > 0 => {
                            info!(hooks = count, "Loaded hooks from QUANT.md");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!(error = %e, "Failed to parse hooks from QUANT.md");
                        }
                    }
                }
            }
        }

        // Initialize MCP manager
        let mcp_manager = Arc::new(Mutex::new(McpManager::new()));

        Self {
            client,
            router,
            config,
            project_context,
            hook_manager,
            mcp_manager,
        }
    }

    /// Create a new agent loop with async MCP initialization
    pub async fn new_with_mcp(
        client: OllamaClient,
        mut router: ToolRouter,
        config: AgentConfig,
    ) -> Result<Self> {
        // Auto-discover project context from working directory
        let project_context = ProjectContext::discover(&config.working_dir);
        if let Some(ref ctx) = project_context {
            info!(
                project = %ctx.name,
                project_type = %ctx.project_type,
                has_quant_md = ctx.quant_file.is_some(),
                "Discovered project context"
            );
        }

        // Initialize hook manager and load hooks from QUANT.md
        let mut hook_manager = HookManager::new();
        if let Some(ref ctx) = project_context {
            if let Some(ref quant_file) = ctx.quant_file {
                if let Ok(content) = std::fs::read_to_string(&quant_file.path) {
                    match hook_manager.load_from_quant_md(&content) {
                        Ok(count) if count > 0 => {
                            info!(hooks = count, "Loaded hooks from QUANT.md");
                        }
                        Ok(_) => {}
                        Err(e) => {
                            warn!(error = %e, "Failed to parse hooks from QUANT.md");
                        }
                    }
                }
            }
        }

        // Initialize MCP manager and start servers from QUANT.md
        let mut mcp_manager = McpManager::new();

        if let Some(ref ctx) = project_context {
            if let Some(ref quant_file) = ctx.quant_file {
                if quant_file.has_mcp_servers() {
                    info!(
                        servers = quant_file.mcp_servers.len(),
                        "Starting MCP servers from QUANT.md"
                    );

                    let failures = mcp_manager
                        .start_all(quant_file.mcp_servers.clone())
                        .await;

                    if !failures.is_empty() {
                        warn!(
                            failed = ?failures,
                            "Some MCP servers failed to start"
                        );
                    }

                    // Discover tools from MCP servers and add to registry
                    match mcp_manager.discover_tools().await {
                        Ok(tools) => {
                            let tool_count = tools.len();
                            router.registry_mut().register_mcp_tools(tools);
                            if tool_count > 0 {
                                info!(tools = tool_count, "Registered MCP tools");
                            }
                        }
                        Err(e) => {
                            warn!(error = %e, "Failed to discover MCP tools");
                        }
                    }
                }
            }
        }

        Ok(Self {
            client,
            router,
            config,
            project_context,
            hook_manager,
            mcp_manager: Arc::new(Mutex::new(mcp_manager)),
        })
    }

    /// Get the MCP manager for external access
    pub fn mcp_manager(&self) -> Arc<Mutex<McpManager>> {
        Arc::clone(&self.mcp_manager)
    }

    /// Shutdown MCP servers
    pub async fn shutdown_mcp(&self) {
        let mut manager = self.mcp_manager.lock().await;
        manager.stop_all().await;
    }

    /// Run the agent with a task
    #[instrument(skip(self), fields(model = %self.config.model))]
    pub async fn run(&self, task: &str) -> Result<AgentState> {
        info!(task_len = task.len(), max_iterations = self.config.max_iterations, "Starting agent loop");
        let mut state = AgentState::new();

        // Create base hook context
        let base_hook_ctx = HookContext::new(self.config.working_dir.clone())
            .with_task(task);

        // Run agent start hooks
        let start_results = self.hook_manager.run_hooks(
            HookEvent::AgentStart,
            &base_hook_ctx,
            None,
        ).await;

        // Check if any abort_on_failure hooks failed
        for result in &start_results {
            if !result.success && self.hook_manager.has_aborting_hooks(HookEvent::AgentStart) {
                state.mark_error(format!("Agent start hook '{}' failed: {:?}", result.name, result.error));
                return Ok(state);
            }
        }

        // Select smart context based on the task
        let smart_context = self.select_smart_context(task);

        // Add system prompt if configured
        if let Some(ref system) = self.config.system_prompt {
            state.add_message(ChatMessageWithTools {
                role: Role::System,
                content: system.clone(),
                tool_calls: None,
                tool_call_id: None,
            });
        } else {
            // Default agent system prompt with smart context
            let default_system = self.default_system_prompt_with_context(&smart_context);
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

            // Run iteration start hooks
            let iter_hook_ctx = base_hook_ctx.clone().with_iteration(state.iteration);
            self.hook_manager.run_hooks(HookEvent::IterationStart, &iter_hook_ctx, None).await;

            if self.config.verbose {
                print!(
                    "{}[Iteration {}]{} ",
                    DIM, state.iteration, RESET
                );
                stdout().flush()?;
            }

            // Call the LLM with streaming
            debug!("Calling LLM with tools (streaming)");

            // Get streaming response
            let stream_result = self
                .client
                .chat_stream_with_tools(
                    &self.config.model,
                    &state.messages,
                    Some(&tool_defs),
                    Some(ChatOptions::default()),
                )
                .await;

            let mut stream = match stream_result {
                Ok(s) => s,
                Err(e) => {
                    warn!(error = %e, "LLM request failed");
                    state.mark_error(format!("LLM error: {}", e));
                    break;
                }
            };

            // Accumulate response from stream
            let mut content = String::new();
            let mut tool_calls: Vec<LlmToolCall> = Vec::new();
            let mut started_output = false;

            // Process stream chunks
            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(e) => {
                        warn!(error = %e, "Stream error");
                        state.mark_error(format!("Stream error: {}", e));
                        break;
                    }
                };

                // Extract content from chunk
                if let Some(ref msg) = chunk.message {
                    // Print streaming content
                    if !msg.content.is_empty() && self.config.verbose {
                        if !started_output {
                            println!(); // Start on new line
                            started_output = true;
                        }
                        print!("{}", msg.content);
                        stdout().flush()?;
                    }
                    content.push_str(&msg.content);

                    // Collect tool calls (usually in final chunk)
                    if !msg.tool_calls.is_empty() {
                        tool_calls.extend(msg.tool_calls.clone());
                    }
                }

                // Check if done - extract token usage from final chunk
                if chunk.done {
                    // Record token usage
                    state.record_tokens(
                        chunk.prompt_eval_count.unwrap_or(0),
                        chunk.eval_count.unwrap_or(0),
                        chunk.total_duration.unwrap_or(0),
                        chunk.eval_duration.unwrap_or(0),
                    );
                    debug!(
                        prompt_tokens = chunk.prompt_eval_count,
                        completion_tokens = chunk.eval_count,
                        "Recorded token usage"
                    );
                    break;
                }
            }

            // Finish output line if we printed content
            if started_output && self.config.verbose {
                println!();
            }

            // Check if LLM wants to call tools
            // First check native tool_calls, then fallback to parsing JSON from content
            if tool_calls.is_empty() {
                // Try to parse JSON tool calls from content (for models that don't use native tool calling)
                if let Some(parsed_calls) = parse_json_tool_calls(&content) {
                    debug!(
                        count = parsed_calls.len(),
                        "Parsed tool calls from content JSON"
                    );
                    tool_calls = parsed_calls;
                }
            }

            if tool_calls.is_empty() {
                // No tool calls - LLM is done
                info!(iterations = state.iteration, "Agent completed task");
                if self.config.verbose {
                    println!("{}Done{}", GREEN, RESET);
                }
                state.mark_finished(content.clone());
                state.add_message(ChatMessageWithTools {
                    role: Role::Assistant,
                    content,
                    tool_calls: None,
                    tool_call_id: None,
                });
                break;
            }

            // Add assistant message with tool calls
            state.add_message(ChatMessageWithTools {
                role: Role::Assistant,
                content: content.clone(),
                tool_calls: Some(tool_calls.clone()),
                tool_call_id: None,
            });

            // Execute each tool call
            debug!(tool_count = tool_calls.len(), "Processing tool calls");
            for tool_call in &tool_calls {
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

                // Run tool_before hooks
                let tool_hook_ctx = base_hook_ctx.clone()
                    .with_iteration(state.iteration)
                    .with_tool(&call.name, &call.arguments);
                self.hook_manager.run_hooks(HookEvent::ToolBefore, &tool_hook_ctx, Some(&call.name)).await;

                // Show tool execution with spinner
                let mut tool_spinner = if self.config.verbose {
                    println!();
                    let mut s = Spinner::new(format!("Running {}...", call.name));
                    s.start();
                    Some(s)
                } else {
                    None
                };

                let result = self.router.route(&call, &tool_ctx).await;

                // Stop tool spinner
                if let Some(ref mut s) = tool_spinner {
                    s.stop().await;
                }

                if self.config.verbose {
                    print!(
                        "{}[Tool: {}]{} ",
                        CYAN, call.name, RESET
                    );
                    stdout().flush()?;
                }

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

                // Run tool_after hooks
                let tool_after_ctx = tool_hook_ctx.clone()
                    .with_tool_result(&tool_result, is_success);
                self.hook_manager.run_hooks(HookEvent::ToolAfter, &tool_after_ctx, Some(&call.name)).await;

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

            // Run iteration end hooks
            self.hook_manager.run_hooks(HookEvent::IterationEnd, &iter_hook_ctx, None).await;
        }

        // Check if we hit max iterations
        if !state.finished && state.iteration >= self.config.max_iterations {
            warn!(max_iterations = self.config.max_iterations, "Agent reached maximum iterations");
            state.mark_error(format!(
                "Agent reached maximum iterations ({})",
                self.config.max_iterations
            ));
        }

        // Display token usage summary
        if self.config.verbose && state.token_usage.call_count > 0 {
            println!();
            println!(
                "{}[Usage]{} {}",
                DIM,
                RESET,
                state.token_usage.summary()
            );
        }

        // Run agent finish hooks
        let finish_hook_ctx = base_hook_ctx.clone()
            .with_agent_result(state.finished && state.error.is_none(), state.error.clone());
        self.hook_manager.run_hooks(HookEvent::AgentFinish, &finish_hook_ctx, None).await;

        info!(
            finished = state.finished,
            iterations = state.iteration,
            prompt_tokens = state.token_usage.prompt_tokens,
            completion_tokens = state.token_usage.completion_tokens,
            total_tokens = state.token_usage.total_tokens(),
            error = ?state.error,
            "Agent loop completed"
        );

        Ok(state)
    }

    /// Select relevant files based on the task using smart context
    fn select_smart_context(&self, task: &str) -> Option<SmartContext> {
        let project_root = self.project_context.as_ref().map(|c| c.root.clone())
            .unwrap_or_else(|| self.config.working_dir.clone());

        let mut selector = SmartContextSelector::new(project_root)
            .with_max_tokens(4000); // Reserve tokens for smart context

        match selector.select_context(task) {
            Ok(ctx) if !ctx.is_empty() => {
                if self.config.verbose {
                    println!(
                        "{}[Smart Context]{} Auto-selected {} relevant file(s)",
                        CYAN, RESET, ctx.files.len()
                    );
                }
                info!(
                    files = ctx.files.len(),
                    chars = ctx.char_count(),
                    "Smart context selected files"
                );
                Some(ctx)
            }
            Ok(_) => {
                debug!("No relevant files found for smart context");
                None
            }
            Err(e) => {
                warn!(error = %e, "Failed to select smart context");
                None
            }
        }
    }

    /// Build system prompt with optional smart context
    fn default_system_prompt_with_context(&self, smart_context: &Option<SmartContext>) -> String {
        let mut prompt = String::new();

        prompt.push_str("You are an AI assistant with access to tools for completing tasks. You can read files, search for content, execute commands, and more.\n\n");

        // Add project context if available
        if let Some(ref ctx) = self.project_context {
            prompt.push_str(&ctx.to_system_context());
            prompt.push_str("\n");
        } else {
            prompt.push_str(&format!("Working directory: {}\n\n", self.config.working_dir.display()));
        }

        // Add smart context (auto-selected relevant files)
        if let Some(ref ctx) = smart_context {
            prompt.push_str(&ctx.to_context_string());
        }

        prompt.push_str("## Available Tools\n");
        prompt.push_str(&self.format_tool_list());
        prompt.push_str("\n\n");

        prompt.push_str(r#"## Guidelines
- Use tools to gather information before responding
- For file operations, prefer reading before writing
- For commands, explain what you're doing
- Be concise but thorough
- If a task is unclear, ask for clarification
- Follow any project-specific instructions from QUANT.md
- Relevant files have been pre-loaded above - use them as context

When you have completed the task, provide a final summary response without calling any more tools."#);

        prompt
    }

    fn default_system_prompt(&self) -> String {
        self.default_system_prompt_with_context(&None)
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

/// Parse JSON tool calls from content text
///
/// Many models output tool calls as JSON in the content field rather than using
/// Ollama's native tool_calls mechanism. This function extracts those calls.
///
/// Supports:
/// - Raw JSON: `{"name": "tool_name", "arguments": {...}}`
/// - Markdown code blocks: ```json\n{"name": ...}\n```
/// - Multiple tool calls (array or sequential)
fn parse_json_tool_calls(content: &str) -> Option<Vec<LlmToolCall>> {
    let content = content.trim();
    if content.is_empty() {
        return None;
    }

    // Try to extract JSON from markdown code blocks first
    let json_content = extract_json_from_markdown(content).unwrap_or(content);

    // Try parsing as a single tool call object
    if let Some(call) = try_parse_single_tool_call(json_content) {
        return Some(vec![call]);
    }

    // Try parsing as an array of tool calls
    if let Some(calls) = try_parse_tool_call_array(json_content) {
        return Some(calls);
    }

    // Try finding JSON objects in the content
    if let Some(calls) = extract_json_objects(content) {
        return Some(calls);
    }

    None
}

/// Extract JSON content from markdown code blocks
fn extract_json_from_markdown(content: &str) -> Option<&str> {
    // Match ```json ... ``` or ``` ... ```
    let patterns = ["```json\n", "```JSON\n", "```\n"];

    for pattern in patterns {
        if let Some(start) = content.find(pattern) {
            let json_start = start + pattern.len();
            if let Some(end) = content[json_start..].find("```") {
                return Some(content[json_start..json_start + end].trim());
            }
        }
    }

    None
}

/// Try to parse content as a single tool call
fn try_parse_single_tool_call(content: &str) -> Option<LlmToolCall> {
    #[derive(serde::Deserialize)]
    struct ToolCallJson {
        name: String,
        arguments: serde_json::Value,
    }

    let parsed: ToolCallJson = serde_json::from_str(content).ok()?;

    // Validate that name is not empty
    if parsed.name.is_empty() {
        return None;
    }

    Some(LlmToolCall {
        id: uuid::Uuid::new_v4().to_string(),
        function: LlmFunctionCall {
            name: parsed.name,
            arguments: parsed.arguments,
        },
    })
}

/// Try to parse content as an array of tool calls
fn try_parse_tool_call_array(content: &str) -> Option<Vec<LlmToolCall>> {
    #[derive(serde::Deserialize)]
    struct ToolCallJson {
        name: String,
        arguments: serde_json::Value,
    }

    let parsed: Vec<ToolCallJson> = serde_json::from_str(content).ok()?;

    if parsed.is_empty() {
        return None;
    }

    let calls: Vec<LlmToolCall> = parsed
        .into_iter()
        .filter(|c| !c.name.is_empty())
        .map(|c| LlmToolCall {
            id: uuid::Uuid::new_v4().to_string(),
            function: LlmFunctionCall {
                name: c.name,
                arguments: c.arguments,
            },
        })
        .collect();

    if calls.is_empty() {
        None
    } else {
        Some(calls)
    }
}

/// Extract JSON objects from content that look like tool calls
fn extract_json_objects(content: &str) -> Option<Vec<LlmToolCall>> {
    let mut calls = Vec::new();
    let mut depth = 0;
    let mut start = None;

    for (i, c) in content.char_indices() {
        match c {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        let json_str = &content[s..=i];
                        if let Some(call) = try_parse_single_tool_call(json_str) {
                            calls.push(call);
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }

    if calls.is_empty() {
        None
    } else {
        Some(calls)
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

    #[test]
    fn test_parse_json_tool_call_raw() {
        let content = r#"{"name": "glob", "arguments": {"pattern": "*.rs"}}"#;
        let calls = parse_json_tool_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "glob");
    }

    #[test]
    fn test_parse_json_tool_call_markdown() {
        let content = r#"```json
{"name": "read_file", "arguments": {"path": "/tmp/test.txt"}}
```"#;
        let calls = parse_json_tool_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "read_file");
    }

    #[test]
    fn test_parse_json_tool_call_with_text() {
        let content = r#"I'll search for that file.
{"name": "glob", "arguments": {"pattern": "src/**/*.rs"}}
Let me know if you need more."#;
        let calls = parse_json_tool_calls(content).unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0].function.name, "glob");
    }

    #[test]
    fn test_parse_json_tool_call_array() {
        let content = r#"[
            {"name": "read_file", "arguments": {"path": "a.txt"}},
            {"name": "read_file", "arguments": {"path": "b.txt"}}
        ]"#;
        let calls = parse_json_tool_calls(content).unwrap();
        assert_eq!(calls.len(), 2);
    }

    #[test]
    fn test_parse_json_tool_call_no_match() {
        let content = "Just a regular response with no tool calls.";
        assert!(parse_json_tool_calls(content).is_none());
    }

    #[test]
    fn test_extract_json_from_markdown() {
        let content = "```json\n{\"test\": true}\n```";
        let json = extract_json_from_markdown(content).unwrap();
        assert_eq!(json, "{\"test\": true}");
    }
}
