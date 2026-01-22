//! Agent state management

use llm_core::ChatMessageWithTools;
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
}

impl AgentState {
    pub fn new() -> Self {
        Self {
            messages: Vec::new(),
            iteration: 0,
            finished: false,
            final_response: None,
            error: None,
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
