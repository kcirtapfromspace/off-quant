//! Security and confirmation handling for tools

use async_trait::async_trait;
use std::io::{self, IsTerminal, Write};
use tokio::io::{AsyncBufReadExt, BufReader};
use tracing::{debug, warn};

use super::{SecurityLevel, ToolCall};

/// Check if stdin is connected to a terminal
pub fn is_interactive() -> bool {
    io::stdin().is_terminal()
}

/// Result of a confirmation prompt
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationResult {
    /// User approved the action
    Approved,
    /// User denied the action
    Denied,
    /// User wants to skip this tool call
    Skip,
    /// User wants to abort the entire operation
    Abort,
}

/// Trait for handling tool execution confirmations
#[async_trait]
pub trait ConfirmationHandler: Send + Sync {
    /// Request confirmation for a tool call
    async fn confirm(&self, tool_call: &ToolCall, security_level: SecurityLevel) -> ConfirmationResult;
}

/// Default terminal-based confirmation handler
pub struct TerminalConfirmation {
    /// Whether to auto-approve all actions
    pub auto_approve: bool,
}

impl TerminalConfirmation {
    pub fn new() -> Self {
        Self { auto_approve: false }
    }

    pub fn auto() -> Self {
        Self { auto_approve: true }
    }
}

impl Default for TerminalConfirmation {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl ConfirmationHandler for TerminalConfirmation {
    async fn confirm(&self, tool_call: &ToolCall, security_level: SecurityLevel) -> ConfirmationResult {
        if self.auto_approve {
            debug!(tool = %tool_call.name, "Auto-approving tool execution");
            return ConfirmationResult::Approved;
        }

        // Safe tools don't need confirmation
        if security_level == SecurityLevel::Safe {
            return ConfirmationResult::Approved;
        }

        // P2: TTY detection - if not interactive, deny dangerous actions
        if !is_interactive() {
            warn!(
                tool = %tool_call.name,
                security_level = %security_level,
                "Non-interactive mode: denying tool that requires confirmation"
            );
            eprintln!(
                "\x1b[93m[Warning]\x1b[0m Non-interactive mode: tool '{}' ({}) requires confirmation but stdin is not a TTY.",
                tool_call.name, security_level
            );
            eprintln!("Use --auto flag to bypass confirmations in non-interactive mode.");
            return ConfirmationResult::Denied;
        }

        // Display the tool call
        let level_color = match security_level {
            SecurityLevel::Safe => "\x1b[92m",      // green
            SecurityLevel::Moderate => "\x1b[93m", // yellow
            SecurityLevel::Dangerous => "\x1b[91m", // red
        };

        println!();
        println!(
            "{}[{}]{} Tool: {}{}{}",
            level_color,
            security_level,
            "\x1b[0m",
            "\x1b[1m",
            tool_call.name,
            "\x1b[0m"
        );

        // Pretty print arguments
        if let Ok(pretty) = serde_json::to_string_pretty(&tool_call.arguments) {
            for line in pretty.lines() {
                println!("  {}", line);
            }
        }

        println!();
        print!("Allow this action? [y/n/s(kip)/a(bort)] ");
        io::stdout().flush().unwrap();

        // Use async stdin to avoid blocking the runtime
        let stdin = tokio::io::stdin();
        let mut reader = BufReader::new(stdin);
        let mut input = String::new();

        if reader.read_line(&mut input).await.is_err() {
            debug!("Failed to read stdin, aborting");
            return ConfirmationResult::Abort;
        }

        let result = match input.trim().to_lowercase().as_str() {
            "y" | "yes" | "" => ConfirmationResult::Approved,
            "n" | "no" => ConfirmationResult::Denied,
            "s" | "skip" => ConfirmationResult::Skip,
            "a" | "abort" | "q" | "quit" => ConfirmationResult::Abort,
            _ => ConfirmationResult::Denied,
        };

        debug!(tool = %tool_call.name, result = ?result, "User confirmation response");
        result
    }
}

/// A confirmation handler that always approves (for testing or auto mode)
pub struct AutoApprove;

#[async_trait]
impl ConfirmationHandler for AutoApprove {
    async fn confirm(&self, _tool_call: &ToolCall, _security_level: SecurityLevel) -> ConfirmationResult {
        ConfirmationResult::Approved
    }
}

/// A confirmation handler that always denies (for testing)
pub struct AutoDeny;

#[async_trait]
impl ConfirmationHandler for AutoDeny {
    async fn confirm(&self, _tool_call: &ToolCall, _security_level: SecurityLevel) -> ConfirmationResult {
        ConfirmationResult::Denied
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn test_auto_approve() {
        let handler = AutoApprove;
        let tool_call = ToolCall {
            name: "test".to_string(),
            arguments: json!({}),
        };

        let result = handler.confirm(&tool_call, SecurityLevel::Dangerous).await;
        assert_eq!(result, ConfirmationResult::Approved);
    }

    #[tokio::test]
    async fn test_auto_deny() {
        let handler = AutoDeny;
        let tool_call = ToolCall {
            name: "test".to_string(),
            arguments: json!({}),
        };

        let result = handler.confirm(&tool_call, SecurityLevel::Dangerous).await;
        assert_eq!(result, ConfirmationResult::Denied);
    }

    #[tokio::test]
    async fn test_terminal_auto_mode() {
        let handler = TerminalConfirmation::auto();
        let tool_call = ToolCall {
            name: "test".to_string(),
            arguments: json!({}),
        };

        let result = handler.confirm(&tool_call, SecurityLevel::Dangerous).await;
        assert_eq!(result, ConfirmationResult::Approved);
    }

    #[test]
    fn test_is_interactive_in_test() {
        // In test environment, stdin is typically not a terminal
        // This test just ensures the function works without crashing
        let _result = super::is_interactive();
    }
}
