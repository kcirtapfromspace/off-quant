//! Bash command execution tool

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Tool for executing bash commands
pub struct BashTool;

#[async_trait]
impl Tool for BashTool {
    fn name(&self) -> &str {
        "bash"
    }

    fn description(&self) -> &str {
        "Execute a bash command and return the output. Use for running terminal commands, git operations, build tools, etc."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Dangerous
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("command", ParameterProperty::string("The bash command to execute"))
            .with_property("timeout", ParameterProperty::number("Timeout in seconds (default: 120)").with_default(Value::Number(120.into())))
            .with_property("working_dir", ParameterProperty::string("Working directory for the command (default: current directory)"))
    }

    async fn execute(&self, args: Value, ctx: &ToolContext) -> Result<ToolResult> {
        let command = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;

        let timeout_secs = args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);

        let working_dir = args.get("working_dir")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        // Check if working directory exists
        if !working_dir.exists() {
            return Ok(ToolResult::error(format!(
                "Working directory does not exist: {}",
                working_dir.display()
            )));
        }

        // Determine shell
        let shell = if cfg!(target_os = "windows") {
            "cmd"
        } else {
            "bash"
        };

        let shell_arg = if cfg!(target_os = "windows") {
            "/C"
        } else {
            "-c"
        };

        // Build command
        let mut cmd = Command::new(shell);
        cmd.arg(shell_arg)
            .arg(command)
            .current_dir(&working_dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());

        // Execute with timeout
        let result = timeout(Duration::from_secs(timeout_secs), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut combined_output = String::new();

                if !stdout.is_empty() {
                    combined_output.push_str(&stdout);
                }

                if !stderr.is_empty() {
                    if !combined_output.is_empty() {
                        combined_output.push_str("\n--- stderr ---\n");
                    }
                    combined_output.push_str(&stderr);
                }

                // Truncate if too long
                let combined_output = if combined_output.len() > ctx.max_output_len {
                    format!(
                        "{}\n\n[Output truncated at {} characters]",
                        &combined_output[..ctx.max_output_len],
                        ctx.max_output_len
                    )
                } else {
                    combined_output
                };

                if output.status.success() {
                    Ok(ToolResult::success(combined_output))
                } else {
                    let exit_code = output.status.code().map(|c| c.to_string()).unwrap_or_else(|| "unknown".to_string());
                    Ok(ToolResult::failure(
                        combined_output,
                        format!("Command exited with code {}", exit_code),
                    ))
                }
            }
            Ok(Err(e)) => Ok(ToolResult::error(format!("Failed to execute command: {}", e))),
            Err(_) => Ok(ToolResult::error(format!(
                "Command timed out after {} seconds",
                timeout_secs
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_bash_echo() {
        let tool = BashTool;
        let ctx = ToolContext::default();
        let args = json!({ "command": "echo 'hello world'" });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("hello world"));
    }

    #[tokio::test]
    async fn test_bash_pwd() {
        let temp_dir = TempDir::new().unwrap();
        let tool = BashTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());
        let args = json!({ "command": "pwd" });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(result.success);
        // On macOS, temp directories are in /private/var, so we need to account for that
        let expected_path = temp_dir.path().canonicalize().unwrap();
        assert!(result.output.contains(expected_path.to_str().unwrap()) ||
                result.output.contains(temp_dir.path().to_str().unwrap()));
    }

    #[tokio::test]
    async fn test_bash_failure() {
        let tool = BashTool;
        let ctx = ToolContext::default();
        let args = json!({ "command": "exit 1" });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.is_some());
    }

    #[tokio::test]
    async fn test_bash_stderr() {
        let tool = BashTool;
        let ctx = ToolContext::default();
        let args = json!({ "command": "echo 'error message' >&2" });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(result.output.contains("error message"));
    }

    #[tokio::test]
    async fn test_bash_timeout() {
        let tool = BashTool;
        let ctx = ToolContext::default();
        let args = json!({
            "command": "sleep 10",
            "timeout": 1
        });

        let result = tool.execute(args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("timed out"));
    }
}
