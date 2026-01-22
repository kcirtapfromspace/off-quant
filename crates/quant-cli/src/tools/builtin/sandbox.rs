//! Sandboxed command execution
//!
//! Provides isolated execution environments for running untrusted commands.
//! Supports multiple backends: firejail, bubblewrap, docker, or native (no sandbox).

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::path::PathBuf;
use std::process::Stdio;
use tokio::process::Command;
use tokio::time::{timeout, Duration};
use tracing::{debug, info, warn};

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Available sandbox backends
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxBackend {
    /// No sandboxing (native execution)
    None,
    /// Firejail (Linux) - lightweight sandboxing
    Firejail,
    /// Bubblewrap (Linux) - container-like isolation
    Bubblewrap,
    /// Docker container isolation
    Docker,
}

impl SandboxBackend {
    /// Detect the best available sandbox backend
    pub fn detect() -> Self {
        // Check for available sandboxing tools
        if is_command_available("firejail") {
            debug!("Sandbox backend: firejail");
            return Self::Firejail;
        }

        if is_command_available("bwrap") {
            debug!("Sandbox backend: bubblewrap");
            return Self::Bubblewrap;
        }

        if is_command_available("docker") {
            debug!("Sandbox backend: docker");
            return Self::Docker;
        }

        debug!("Sandbox backend: none (no sandbox available)");
        Self::None
    }

    /// Get the display name
    pub fn name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Firejail => "firejail",
            Self::Bubblewrap => "bubblewrap",
            Self::Docker => "docker",
        }
    }
}

/// Check if a command is available in PATH
fn is_command_available(cmd: &str) -> bool {
    which::which(cmd).is_ok()
}

/// Tool for executing commands in a sandbox
pub struct SandboxTool {
    backend: SandboxBackend,
    docker_image: String,
}

impl SandboxTool {
    /// Create a new sandbox tool with auto-detected backend
    pub fn new() -> Self {
        Self {
            backend: SandboxBackend::detect(),
            docker_image: "alpine:latest".to_string(),
        }
    }

    /// Create with a specific backend
    pub fn with_backend(backend: SandboxBackend) -> Self {
        Self {
            backend,
            docker_image: "alpine:latest".to_string(),
        }
    }

    /// Set the Docker image to use
    pub fn with_docker_image(mut self, image: impl Into<String>) -> Self {
        self.docker_image = image.into();
        self
    }

    /// Build the sandboxed command
    fn build_command(&self, user_command: &str, working_dir: &PathBuf) -> Command {
        match self.backend {
            SandboxBackend::None => {
                let mut cmd = Command::new("bash");
                cmd.arg("-c").arg(user_command).current_dir(working_dir);
                cmd
            }

            SandboxBackend::Firejail => {
                let mut cmd = Command::new("firejail");
                cmd.args([
                    "--quiet",
                    "--private-tmp",
                    "--private-dev",
                    "--noroot",
                    "--seccomp",
                    "--caps.drop=all",
                    "--nonewprivs",
                    &format!("--whitelist={}", working_dir.display()),
                    "--",
                    "bash", "-c", user_command,
                ])
                .current_dir(working_dir);
                cmd
            }

            SandboxBackend::Bubblewrap => {
                let mut cmd = Command::new("bwrap");
                cmd.args([
                    "--ro-bind", "/usr", "/usr",
                    "--ro-bind", "/lib", "/lib",
                    "--ro-bind", "/lib64", "/lib64",
                    "--ro-bind", "/bin", "/bin",
                    "--symlink", "/usr/lib", "/lib",
                    "--symlink", "/usr/lib64", "/lib64",
                    "--proc", "/proc",
                    "--dev", "/dev",
                    "--tmpfs", "/tmp",
                    "--bind", working_dir.to_str().unwrap_or("."), working_dir.to_str().unwrap_or("."),
                    "--chdir", working_dir.to_str().unwrap_or("."),
                    "--unshare-all",
                    "--die-with-parent",
                    "--new-session",
                    "bash", "-c", user_command,
                ]);
                cmd
            }

            SandboxBackend::Docker => {
                let mut cmd = Command::new("docker");
                cmd.args([
                    "run",
                    "--rm",
                    "--network", "none",
                    "--read-only",
                    "--memory", "256m",
                    "--cpus", "1",
                    "--pids-limit", "50",
                    "-v", &format!("{}:/workspace:rw", working_dir.display()),
                    "-w", "/workspace",
                    &self.docker_image,
                    "/bin/sh", "-c", user_command,
                ]);
                cmd
            }
        }
    }
}

impl Default for SandboxTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for SandboxTool {
    fn name(&self) -> &str {
        "sandbox"
    }

    fn description(&self) -> &str {
        "Execute a command in an isolated sandbox environment. Safer than bash for running untrusted code. Supports firejail, bubblewrap, or docker backends."
    }

    fn security_level(&self) -> SecurityLevel {
        // Still dangerous because it can write to working directory
        SecurityLevel::Dangerous
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("command", ParameterProperty::string("The command to execute in the sandbox"))
            .with_property("timeout", ParameterProperty::number("Timeout in seconds (default: 60)"))
            .with_property("network", ParameterProperty::boolean("Allow network access (default: false, docker only)"))
            .with_property("memory_mb", ParameterProperty::number("Memory limit in MB (default: 256, docker only)"))
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let command = args.get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: command"))?;

        let timeout_secs = args.get("timeout")
            .and_then(|v| v.as_u64())
            .unwrap_or(60);

        info!(
            backend = self.backend.name(),
            command = command.chars().take(50).collect::<String>(),
            timeout_secs,
            "Executing sandboxed command"
        );

        // Validate working directory exists
        if !ctx.working_dir.exists() {
            return Ok(ToolResult::error(format!(
                "Working directory does not exist: {}",
                ctx.working_dir.display()
            )));
        }

        let mut cmd = self.build_command(command, &ctx.working_dir);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

        // Execute with timeout
        let result = timeout(Duration::from_secs(timeout_secs), cmd.output()).await;

        match result {
            Ok(Ok(output)) => {
                let exit_code = output.status.code();
                debug!(exit_code = ?exit_code, backend = self.backend.name(), "Sandbox command completed");

                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                let mut combined_output = String::new();
                combined_output.push_str(&format!("[sandbox: {}]\n", self.backend.name()));

                if !stdout.is_empty() {
                    combined_output.push_str(&stdout);
                }

                if !stderr.is_empty() {
                    if !stdout.is_empty() {
                        combined_output.push_str("\n--- stderr ---\n");
                    }
                    combined_output.push_str(&stderr);
                }

                // Truncate if too long
                let combined_output = if combined_output.len() > ctx.max_output_len {
                    let safe_end = combined_output
                        .char_indices()
                        .take_while(|(idx, _)| *idx < ctx.max_output_len)
                        .last()
                        .map(|(idx, c)| idx + c.len_utf8())
                        .unwrap_or(0);
                    format!(
                        "{}\n\n[Output truncated at {} characters]",
                        &combined_output[..safe_end],
                        safe_end
                    )
                } else {
                    combined_output
                };

                if output.status.success() {
                    Ok(ToolResult::success(combined_output))
                } else {
                    let exit_code = output.status.code()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "unknown".to_string());
                    Ok(ToolResult::failure(
                        combined_output,
                        format!("Sandboxed command exited with code {}", exit_code),
                    ))
                }
            }
            Ok(Err(e)) => {
                warn!(error = %e, backend = self.backend.name(), "Failed to execute sandboxed command");

                // Provide helpful message if sandbox backend not available
                if self.backend != SandboxBackend::None {
                    Ok(ToolResult::error(format!(
                        "Failed to execute sandboxed command (backend: {}): {}. Try installing {} or use the 'bash' tool instead.",
                        self.backend.name(),
                        e,
                        self.backend.name()
                    )))
                } else {
                    Ok(ToolResult::error(format!("Failed to execute command: {}", e)))
                }
            }
            Err(_) => {
                warn!(timeout_secs, backend = self.backend.name(), "Sandboxed command timed out");
                Ok(ToolResult::error(format!(
                    "Sandboxed command timed out after {} seconds",
                    timeout_secs
                )))
            }
        }
    }
}

/// Configuration for sandbox settings
#[derive(Debug, Clone)]
pub struct SandboxConfig {
    /// Preferred backend (None = auto-detect)
    pub backend: Option<SandboxBackend>,
    /// Default Docker image
    pub docker_image: String,
    /// Whether to enable sandbox by default for bash commands
    pub sandbox_by_default: bool,
    /// Network access allowed in sandbox
    pub allow_network: bool,
    /// Memory limit in MB
    pub memory_limit_mb: u32,
}

impl Default for SandboxConfig {
    fn default() -> Self {
        Self {
            backend: None,
            docker_image: "alpine:latest".to_string(),
            sandbox_by_default: false,
            allow_network: false,
            memory_limit_mb: 256,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[test]
    fn test_sandbox_backend_detect() {
        // Just test that detection doesn't panic
        let backend = SandboxBackend::detect();
        println!("Detected sandbox backend: {:?}", backend);
    }

    #[test]
    fn test_sandbox_tool_creation() {
        let tool = SandboxTool::new();
        assert_eq!(tool.name(), "sandbox");
    }

    #[tokio::test]
    async fn test_sandbox_echo() {
        let tool = SandboxTool::with_backend(SandboxBackend::None);
        let temp_dir = TempDir::new().unwrap();
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "command": "echo 'hello from sandbox'"
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success, "Expected success but got: {:?}", result.error);
        assert!(result.output.contains("hello from sandbox"));
        assert!(result.output.contains("[sandbox: none]"));
    }

    #[tokio::test]
    async fn test_sandbox_timeout() {
        let tool = SandboxTool::with_backend(SandboxBackend::None);
        let temp_dir = TempDir::new().unwrap();
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "command": "sleep 10",
            "timeout": 1
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.as_ref().unwrap().contains("timed out"));
    }

    #[tokio::test]
    async fn test_sandbox_file_operations() {
        let tool = SandboxTool::with_backend(SandboxBackend::None);
        let temp_dir = TempDir::new().unwrap();
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        // Write a file in sandbox
        let args = json!({
            "command": "echo 'test content' > sandbox_test.txt && cat sandbox_test.txt"
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success, "Expected success but got: {:?}", result.error);
        assert!(result.output.contains("test content"));
    }
}
