//! Agent state management

use llm_core::ChatMessageWithTools;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::PathBuf;

/// Configuration for the agent
#[derive(Debug, Clone)]
pub struct AgentConfig {
    /// Model to use
    pub model: String,
    /// System prompt
    pub system_prompt: Option<String>,
    /// Maximum iterations before stopping
    pub max_iterations: usize,
    /// Working directory
    pub working_dir: PathBuf,
    /// Auto mode (skip confirmations)
    pub auto_mode: bool,
    /// Whether to print tool executions
    pub verbose: bool,
}

impl Default for AgentConfig {
    fn default() -> Self {
        Self {
            model: "llama3.2".to_string(),
            system_prompt: None,
            max_iterations: 50,
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            auto_mode: false,
            verbose: true,
        }
    }
}

impl AgentConfig {
    pub fn new(model: impl Into<String>) -> Self {
        Self {
            model: model.into(),
            ..Default::default()
        }
    }

    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }

    pub fn with_max_iterations(mut self, max: usize) -> Self {
        self.max_iterations = max;
        self
    }

    pub fn with_working_dir(mut self, dir: PathBuf) -> Self {
        self.working_dir = dir;
        self
    }

    pub fn with_auto_mode(mut self, auto: bool) -> Self {
        self.auto_mode = auto;
        self
    }

    pub fn with_verbose(mut self, verbose: bool) -> Self {
        self.verbose = verbose;
        self
    }
}

/// State of the agent during execution
#[derive(Debug)]
pub struct AgentState {
    /// Message history
    pub messages: Vec<ChatMessageWithTools>,
    /// Current iteration
    pub iteration: usize,
    /// Whether the agent has finished
    pub finished: bool,
    /// Final response (if finished)
    pub final_response: Option<String>,
    /// Error message (if failed)
    pub error: Option<String>,
    /// Failure tracker for detecting infinite loops
    pub failure_tracker: FailureTracker,
}

/// Default max consecutive failures before aborting
const DEFAULT_MAX_CONSECUTIVE_FAILURES: usize = 3;

impl AgentState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            iteration: 0,
            finished: false,
            final_response: None,
            error: None,
            failure_tracker: FailureTracker::new(DEFAULT_MAX_CONSECUTIVE_FAILURES),
        }
    }

    /// Create with custom max consecutive failures
    pub fn with_max_consecutive_failures(max: usize) -> Self {
        Self {
            failure_tracker: FailureTracker::new(max),
            ..Self::new()
        }
    }

    pub fn add_message(&mut self, message: ChatMessageWithTools) {
        self.messages.push(message);
    }

    pub fn mark_finished(&mut self, response: String) {
        self.finished = true;
        self.final_response = Some(response);
    }

    pub fn mark_error(&mut self, error: String) {
        self.finished = true;
        self.error = Some(error);
    }

    pub fn increment_iteration(&mut self) {
        self.iteration += 1;
    }
}

impl Default for AgentState {
    fn default() -> Self {
        Self::new()
    }
}

/// Tracks consecutive failures for tool calls to detect infinite loops
#[derive(Debug, Default)]
pub struct FailureTracker {
    /// Map from tool call signature to consecutive failure count
    failures: HashMap<String, ConsecutiveFailure>,
    /// Last tool call signature
    last_signature: Option<String>,
    /// Maximum consecutive failures before aborting
    max_consecutive: usize,
}

#[derive(Debug, Clone)]
pub struct ConsecutiveFailure {
    pub count: usize,
    pub last_error: String,
}

impl FailureTracker {
    pub fn new(max_consecutive: usize) -> Self {
        Self {
            failures: HashMap::new(),
            last_signature: None,
            max_consecutive,
        }
    }

    /// Create a signature for a tool call (name + arguments hash)
    pub fn tool_signature(name: &str, args: &serde_json::Value) -> String {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        let args_str = args.to_string();
        args_str.hash(&mut hasher);
        format!("{}:{:x}", name, hasher.finish())
    }

    /// Record a successful tool execution, resetting failure count
    pub fn record_success(&mut self, signature: &str) {
        self.failures.remove(signature);
        self.last_signature = Some(signature.to_string());
    }

    /// Record a failed tool execution
    /// Returns Some(error_message) if we should abort due to repeated failures
    pub fn record_failure(&mut self, signature: &str, error: &str) -> Option<String> {
        let entry = self.failures.entry(signature.to_string()).or_insert(ConsecutiveFailure {
            count: 0,
            last_error: String::new(),
        });

        entry.count += 1;
        entry.last_error = error.to_string();
        self.last_signature = Some(signature.to_string());

        if entry.count >= self.max_consecutive {
            Some(format!(
                "Tool call failed {} consecutive times with error: {}",
                entry.count, entry.last_error
            ))
        } else {
            None
        }
    }

    /// Check if we're in a repeated failure pattern (same signature as last call)
    pub fn is_repeated_call(&self, signature: &str) -> bool {
        self.last_signature.as_ref().map_or(false, |s| s == signature)
            && self.failures.contains_key(signature)
    }

    /// Get failure count for a signature
    pub fn failure_count(&self, signature: &str) -> usize {
        self.failures.get(signature).map_or(0, |f| f.count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_failure_tracker_success_resets() {
        let mut tracker = FailureTracker::new(3);
        let sig = FailureTracker::tool_signature("test", &json!({"x": 1}));

        // Record two failures
        assert!(tracker.record_failure(&sig, "error").is_none());
        assert!(tracker.record_failure(&sig, "error").is_none());
        assert_eq!(tracker.failure_count(&sig), 2);

        // Success resets the counter
        tracker.record_success(&sig);
        assert_eq!(tracker.failure_count(&sig), 0);
    }

    #[test]
    fn test_failure_tracker_aborts_after_max() {
        let mut tracker = FailureTracker::new(3);
        let sig = FailureTracker::tool_signature("test", &json!({}));

        assert!(tracker.record_failure(&sig, "error 1").is_none());
        assert!(tracker.record_failure(&sig, "error 2").is_none());

        // Third failure should trigger abort
        let abort = tracker.record_failure(&sig, "error 3");
        assert!(abort.is_some());
        assert!(abort.unwrap().contains("3 consecutive times"));
    }

    #[test]
    fn test_failure_tracker_different_signatures() {
        let mut tracker = FailureTracker::new(3);
        let sig1 = FailureTracker::tool_signature("test", &json!({"x": 1}));
        let sig2 = FailureTracker::tool_signature("test", &json!({"x": 2}));

        // Different args = different signatures
        assert_ne!(sig1, sig2);

        // Failures tracked separately
        assert!(tracker.record_failure(&sig1, "error").is_none());
        assert!(tracker.record_failure(&sig2, "error").is_none());

        assert_eq!(tracker.failure_count(&sig1), 1);
        assert_eq!(tracker.failure_count(&sig2), 1);
    }

    #[test]
    fn test_failure_tracker_is_repeated_call() {
        let mut tracker = FailureTracker::new(3);
        let sig = FailureTracker::tool_signature("test", &json!({}));

        // Not repeated initially
        assert!(!tracker.is_repeated_call(&sig));

        // After failure, should be detected as repeated
        tracker.record_failure(&sig, "error");
        assert!(tracker.is_repeated_call(&sig));

        // Different signature is not repeated
        let other_sig = FailureTracker::tool_signature("other", &json!({}));
        assert!(!tracker.is_repeated_call(&other_sig));
    }

    #[test]
    fn test_tool_signature_deterministic() {
        let args = json!({"path": "/tmp/test.txt", "content": "hello"});
        let sig1 = FailureTracker::tool_signature("file_write", &args);
        let sig2 = FailureTracker::tool_signature("file_write", &args);
        assert_eq!(sig1, sig2);
    }

    #[test]
    fn test_agent_state_with_failure_tracker() {
        let state = AgentState::new();
        assert_eq!(state.failure_tracker.failure_count("any"), 0);
    }
}
