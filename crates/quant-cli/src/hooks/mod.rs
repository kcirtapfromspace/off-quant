//! Hooks system for agent lifecycle events
//!
//! Provides extensibility through pre/post hooks for:
//! - Agent start/finish
//! - Tool execution (before/after)
//! - Iteration start/end
//!
//! Hooks can be defined in:
//! - QUANT.md file
//! - quant.toml config
//! - Environment variables

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};

/// Hook execution points
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HookEvent {
    /// Before agent starts processing
    AgentStart,
    /// After agent finishes (success or failure)
    AgentFinish,
    /// Before each agent iteration
    IterationStart,
    /// After each agent iteration
    IterationEnd,
    /// Before any tool execution
    ToolBefore,
    /// After any tool execution
    ToolAfter,
    /// Before a specific tool (use with tool_name filter)
    ToolBeforeNamed,
    /// After a specific tool (use with tool_name filter)
    ToolAfterNamed,
}

impl HookEvent {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::AgentStart => "agent_start",
            Self::AgentFinish => "agent_finish",
            Self::IterationStart => "iteration_start",
            Self::IterationEnd => "iteration_end",
            Self::ToolBefore => "tool_before",
            Self::ToolAfter => "tool_after",
            Self::ToolBeforeNamed => "tool_before_named",
            Self::ToolAfterNamed => "tool_after_named",
        }
    }
}

/// A hook definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hook {
    /// Unique name for this hook
    pub name: String,
    /// When to run this hook
    pub event: HookEvent,
    /// Command to execute
    pub command: String,
    /// Optional: only run for specific tool names
    #[serde(default)]
    pub tool_filter: Option<String>,
    /// Timeout in seconds (default: 30)
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
    /// Whether to abort on hook failure
    #[serde(default)]
    pub abort_on_failure: bool,
    /// Whether this hook is enabled
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_timeout() -> u64 {
    30
}

fn default_enabled() -> bool {
    true
}

/// Context passed to hooks
#[derive(Debug, Clone, Serialize)]
pub struct HookContext {
    /// Working directory
    pub working_dir: PathBuf,
    /// Current iteration (if applicable)
    pub iteration: Option<usize>,
    /// Tool name (if applicable)
    pub tool_name: Option<String>,
    /// Tool arguments as JSON (if applicable)
    pub tool_args: Option<String>,
    /// Tool result (for after hooks)
    pub tool_result: Option<String>,
    /// Whether tool succeeded (for after hooks)
    pub tool_success: Option<bool>,
    /// Task description
    pub task: Option<String>,
    /// Agent finished successfully
    pub agent_success: Option<bool>,
    /// Error message (if any)
    pub error: Option<String>,
}

impl Default for HookContext {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            iteration: None,
            tool_name: None,
            tool_args: None,
            tool_result: None,
            tool_success: None,
            task: None,
            agent_success: None,
            error: None,
        }
    }
}

impl HookContext {
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            ..Default::default()
        }
    }

    pub fn with_iteration(mut self, iteration: usize) -> Self {
        self.iteration = Some(iteration);
        self
    }

    pub fn with_tool(mut self, name: &str, args: &serde_json::Value) -> Self {
        self.tool_name = Some(name.to_string());
        self.tool_args = Some(args.to_string());
        self
    }

    pub fn with_tool_result(mut self, output: &str, success: bool) -> Self {
        self.tool_result = Some(output.to_string());
        self.tool_success = Some(success);
        self
    }

    pub fn with_task(mut self, task: &str) -> Self {
        self.task = Some(task.to_string());
        self
    }

    pub fn with_agent_result(mut self, success: bool, error: Option<String>) -> Self {
        self.agent_success = Some(success);
        self.error = error;
        self
    }

    /// Convert to environment variables for subprocess
    pub fn to_env_vars(&self) -> HashMap<String, String> {
        let mut vars = HashMap::new();

        vars.insert("QUANT_WORKING_DIR".to_string(), self.working_dir.display().to_string());

        if let Some(iter) = self.iteration {
            vars.insert("QUANT_ITERATION".to_string(), iter.to_string());
        }

        if let Some(ref name) = self.tool_name {
            vars.insert("QUANT_TOOL_NAME".to_string(), name.clone());
        }

        if let Some(ref args) = self.tool_args {
            vars.insert("QUANT_TOOL_ARGS".to_string(), args.clone());
        }

        if let Some(ref result) = self.tool_result {
            // Truncate for env var safety
            let truncated = if result.len() > 4096 {
                format!("{}...[truncated]", &result[..4096])
            } else {
                result.clone()
            };
            vars.insert("QUANT_TOOL_RESULT".to_string(), truncated);
        }

        if let Some(success) = self.tool_success {
            vars.insert("QUANT_TOOL_SUCCESS".to_string(), success.to_string());
        }

        if let Some(ref task) = self.task {
            vars.insert("QUANT_TASK".to_string(), task.clone());
        }

        if let Some(success) = self.agent_success {
            vars.insert("QUANT_AGENT_SUCCESS".to_string(), success.to_string());
        }

        if let Some(ref error) = self.error {
            vars.insert("QUANT_ERROR".to_string(), error.clone());
        }

        vars
    }
}

/// Result of running a hook
#[derive(Debug)]
pub struct HookResult {
    /// Hook name
    pub name: String,
    /// Whether the hook succeeded
    pub success: bool,
    /// Output from the hook
    pub output: String,
    /// Error message if failed
    pub error: Option<String>,
    /// Execution time in milliseconds
    pub duration_ms: u64,
}

/// Hook manager for registering and executing hooks
#[derive(Debug, Default)]
pub struct HookManager {
    hooks: Vec<Hook>,
}

impl HookManager {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    /// Register a hook
    pub fn register(&mut self, hook: Hook) {
        info!(
            name = %hook.name,
            event = hook.event.as_str(),
            "Registered hook"
        );
        self.hooks.push(hook);
    }

    /// Register multiple hooks
    pub fn register_all(&mut self, hooks: Vec<Hook>) {
        for hook in hooks {
            self.register(hook);
        }
    }

    /// Load hooks from QUANT.md frontmatter
    pub fn load_from_quant_md(&mut self, content: &str) -> Result<usize> {
        // Parse YAML frontmatter if present
        if !content.starts_with("---") {
            return Ok(0);
        }

        let end = content[3..].find("---").map(|i| i + 3);
        if let Some(end_idx) = end {
            let yaml_content = &content[3..end_idx];

            #[derive(Deserialize)]
            struct QuantMdFrontmatter {
                #[serde(default)]
                hooks: Vec<Hook>,
            }

            if let Ok(frontmatter) = serde_yaml::from_str::<QuantMdFrontmatter>(yaml_content) {
                let count = frontmatter.hooks.len();
                self.register_all(frontmatter.hooks);
                return Ok(count);
            }
        }

        Ok(0)
    }

    /// Get hooks for a specific event
    pub fn hooks_for_event(&self, event: HookEvent, tool_name: Option<&str>) -> Vec<&Hook> {
        self.hooks
            .iter()
            .filter(|h| h.enabled && h.event == event)
            .filter(|h| {
                // Filter by tool name if applicable
                match (&h.tool_filter, tool_name) {
                    (Some(filter), Some(name)) => filter == name,
                    (Some(_), None) => false,
                    (None, _) => true,
                }
            })
            .collect()
    }

    /// Execute all hooks for an event
    pub async fn run_hooks(
        &self,
        event: HookEvent,
        ctx: &HookContext,
        tool_name: Option<&str>,
    ) -> Vec<HookResult> {
        let hooks = self.hooks_for_event(event, tool_name);

        if hooks.is_empty() {
            return Vec::new();
        }

        debug!(
            event = event.as_str(),
            hook_count = hooks.len(),
            "Running hooks"
        );

        let mut results = Vec::new();

        for hook in hooks {
            let result = self.run_hook(hook, ctx).await;
            let should_abort = !result.success && hook.abort_on_failure;

            results.push(result);

            if should_abort {
                warn!(
                    hook = %hook.name,
                    event = event.as_str(),
                    "Hook failed with abort_on_failure=true, stopping hook chain"
                );
                break;
            }
        }

        results
    }

    /// Execute a single hook
    async fn run_hook(&self, hook: &Hook, ctx: &HookContext) -> HookResult {
        let start = std::time::Instant::now();

        debug!(name = %hook.name, command = %hook.command, "Executing hook");

        let mut cmd = Command::new("bash");
        cmd.arg("-c")
            .arg(&hook.command)
            .current_dir(&ctx.working_dir)
            .envs(ctx.to_env_vars())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        let result = timeout(Duration::from_secs(hook.timeout_secs), cmd.output()).await;

        let duration_ms = start.elapsed().as_millis() as u64;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                let combined = if stderr.is_empty() {
                    stdout.to_string()
                } else {
                    format!("{}\n{}", stdout, stderr)
                };

                if output.status.success() {
                    debug!(name = %hook.name, duration_ms, "Hook succeeded");
                    HookResult {
                        name: hook.name.clone(),
                        success: true,
                        output: combined,
                        error: None,
                        duration_ms,
                    }
                } else {
                    let code = output.status.code().unwrap_or(-1);
                    warn!(name = %hook.name, exit_code = code, "Hook failed");
                    HookResult {
                        name: hook.name.clone(),
                        success: false,
                        output: combined,
                        error: Some(format!("Exit code: {}", code)),
                        duration_ms,
                    }
                }
            }
            Ok(Err(e)) => {
                warn!(name = %hook.name, error = %e, "Hook execution failed");
                HookResult {
                    name: hook.name.clone(),
                    success: false,
                    output: String::new(),
                    error: Some(format!("Execution error: {}", e)),
                    duration_ms,
                }
            }
            Err(_) => {
                warn!(name = %hook.name, timeout = hook.timeout_secs, "Hook timed out");
                HookResult {
                    name: hook.name.clone(),
                    success: false,
                    output: String::new(),
                    error: Some(format!("Timed out after {}s", hook.timeout_secs)),
                    duration_ms,
                }
            }
        }
    }

    /// Check if any hook would abort on failure
    pub fn has_aborting_hooks(&self, event: HookEvent) -> bool {
        self.hooks
            .iter()
            .any(|h| h.enabled && h.event == event && h.abort_on_failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_hook_context_env_vars() {
        let ctx = HookContext::new(PathBuf::from("/test"))
            .with_iteration(5)
            .with_tool("bash", &serde_json::json!({"command": "echo hi"}))
            .with_task("Test task");

        let vars = ctx.to_env_vars();
        assert_eq!(vars.get("QUANT_WORKING_DIR").unwrap(), "/test");
        assert_eq!(vars.get("QUANT_ITERATION").unwrap(), "5");
        assert_eq!(vars.get("QUANT_TOOL_NAME").unwrap(), "bash");
        assert!(vars.get("QUANT_TASK").unwrap().contains("Test task"));
    }

    #[test]
    fn test_hook_manager_register() {
        let mut manager = HookManager::new();

        manager.register(Hook {
            name: "test_hook".to_string(),
            event: HookEvent::AgentStart,
            command: "echo 'starting'".to_string(),
            tool_filter: None,
            timeout_secs: 30,
            abort_on_failure: false,
            enabled: true,
        });

        assert_eq!(manager.hooks.len(), 1);
        assert_eq!(manager.hooks_for_event(HookEvent::AgentStart, None).len(), 1);
        assert_eq!(manager.hooks_for_event(HookEvent::AgentFinish, None).len(), 0);
    }

    #[test]
    fn test_hook_manager_tool_filter() {
        let mut manager = HookManager::new();

        manager.register(Hook {
            name: "bash_hook".to_string(),
            event: HookEvent::ToolBefore,
            command: "echo 'before bash'".to_string(),
            tool_filter: Some("bash".to_string()),
            timeout_secs: 30,
            abort_on_failure: false,
            enabled: true,
        });

        // Should match when tool_name is "bash"
        assert_eq!(manager.hooks_for_event(HookEvent::ToolBefore, Some("bash")).len(), 1);
        // Should not match when tool_name is different
        assert_eq!(manager.hooks_for_event(HookEvent::ToolBefore, Some("grep")).len(), 0);
    }

    #[tokio::test]
    async fn test_hook_execution() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = HookManager::new();

        manager.register(Hook {
            name: "echo_hook".to_string(),
            event: HookEvent::AgentStart,
            command: "echo \"Task: $QUANT_TASK\"".to_string(),
            tool_filter: None,
            timeout_secs: 5,
            abort_on_failure: false,
            enabled: true,
        });

        let ctx = HookContext::new(temp_dir.path().to_path_buf())
            .with_task("my test task");

        let results = manager.run_hooks(HookEvent::AgentStart, &ctx, None).await;

        assert_eq!(results.len(), 1);
        assert!(results[0].success);
        assert!(results[0].output.contains("my test task"));
    }

    #[tokio::test]
    async fn test_hook_timeout() {
        let temp_dir = TempDir::new().unwrap();
        let mut manager = HookManager::new();

        manager.register(Hook {
            name: "slow_hook".to_string(),
            event: HookEvent::AgentStart,
            command: "sleep 10".to_string(),
            tool_filter: None,
            timeout_secs: 1,
            abort_on_failure: false,
            enabled: true,
        });

        let ctx = HookContext::new(temp_dir.path().to_path_buf());
        let results = manager.run_hooks(HookEvent::AgentStart, &ctx, None).await;

        assert_eq!(results.len(), 1);
        assert!(!results[0].success);
        assert!(results[0].error.as_ref().unwrap().contains("Timed out"));
    }

    #[test]
    fn test_load_from_quant_md() {
        let mut manager = HookManager::new();

        let quant_md = r#"---
hooks:
  - name: pre_build
    event: agent_start
    command: "cargo check"
    abort_on_failure: true
  - name: post_test
    event: agent_finish
    command: "echo done"
---
# Project
Regular content here
"#;

        let count = manager.load_from_quant_md(quant_md).unwrap();
        assert_eq!(count, 2);
        assert_eq!(manager.hooks.len(), 2);
    }

    #[test]
    fn test_disabled_hooks_not_run() {
        let mut manager = HookManager::new();

        manager.register(Hook {
            name: "disabled_hook".to_string(),
            event: HookEvent::AgentStart,
            command: "echo 'should not run'".to_string(),
            tool_filter: None,
            timeout_secs: 30,
            abort_on_failure: false,
            enabled: false,
        });

        assert_eq!(manager.hooks_for_event(HookEvent::AgentStart, None).len(), 0);
    }
}
