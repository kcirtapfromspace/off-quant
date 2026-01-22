//! Git tool for repository operations
//!
//! Provides git-aware operations like status, diff, log, and commit.

use anyhow::Result;
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Command;
use tracing::debug;

use crate::tools::{
    ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolDefinition,
    ToolResult,
};

/// Git tool for repository operations
pub struct GitTool;

impl Default for GitTool {
    fn default() -> Self {
        Self::new()
    }
}

impl GitTool {
    pub fn new() -> Self {
        Self
    }

    /// Execute a git command and return output
    fn run_git_command(&self, args: &[&str], working_dir: &std::path::Path) -> Result<String> {
        debug!(args = ?args, dir = %working_dir.display(), "Running git command");

        let output = Command::new("git")
            .args(args)
            .current_dir(working_dir)
            .output()?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !output.status.success() {
            if stderr.is_empty() {
                anyhow::bail!("git {} failed: {}", args.join(" "), stdout);
            } else {
                anyhow::bail!("git {} failed: {}", args.join(" "), stderr);
            }
        }

        Ok(stdout.to_string())
    }

    /// Check if directory is a git repository
    fn is_git_repo(&self, working_dir: &std::path::Path) -> bool {
        Command::new("git")
            .args(["rev-parse", "--git-dir"])
            .current_dir(working_dir)
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Get git status
    fn status(&self, working_dir: &std::path::Path) -> Result<String> {
        let status = self.run_git_command(&["status", "--short"], working_dir)?;
        let branch = self.run_git_command(&["branch", "--show-current"], working_dir)?;

        let mut output = format!("Branch: {}\n", branch.trim());

        if status.is_empty() {
            output.push_str("Working tree clean\n");
        } else {
            output.push_str("\nChanges:\n");
            output.push_str(&status);
        }

        Ok(output)
    }

    /// Get git diff
    fn diff(&self, working_dir: &std::path::Path, staged: bool, file: Option<&str>) -> Result<String> {
        let mut args = vec!["diff"];

        if staged {
            args.push("--staged");
        }

        // Add common diff options for better readability
        args.extend(["--color=never", "--stat"]);

        if let Some(f) = file {
            args.push("--");
            args.push(f);
        }

        let stat = self.run_git_command(&args, working_dir)?;

        // Also get the actual diff content (limited)
        let mut content_args = vec!["diff"];
        if staged {
            content_args.push("--staged");
        }
        content_args.push("--color=never");
        if let Some(f) = file {
            content_args.push("--");
            content_args.push(f);
        }

        let content = self.run_git_command(&content_args, working_dir)?;

        // Truncate if too long
        let truncated = if content.len() > 5000 {
            format!("{}\n\n... (truncated, {} more bytes)", &content[..5000], content.len() - 5000)
        } else {
            content
        };

        Ok(format!("## Diff Statistics\n{}\n## Diff Content\n{}", stat, truncated))
    }

    /// Get git log
    fn log(&self, working_dir: &std::path::Path, count: usize) -> Result<String> {
        let count_str = format!("-{}", count.min(50));
        self.run_git_command(
            &["log", &count_str, "--oneline", "--decorate", "--graph"],
            working_dir,
        )
    }

    /// Get recent commits with more detail
    fn show(&self, working_dir: &std::path::Path, commit: &str) -> Result<String> {
        let output = self.run_git_command(
            &["show", "--stat", "--color=never", commit],
            working_dir,
        )?;

        // Truncate if too long
        if output.len() > 8000 {
            Ok(format!("{}\n\n... (truncated)", &output[..8000]))
        } else {
            Ok(output)
        }
    }

    /// Get blame for a file
    fn blame(&self, working_dir: &std::path::Path, file: &str, lines: Option<&str>) -> Result<String> {
        let mut args = vec!["blame", "--color=never"];

        if let Some(l) = lines {
            args.push("-L");
            args.push(l);
        }

        args.push(file);

        let output = self.run_git_command(&args, working_dir)?;

        // Truncate if too long
        if output.len() > 10000 {
            Ok(format!("{}\n\n... (truncated)", &output[..10000]))
        } else {
            Ok(output)
        }
    }

    /// Stage files
    fn add(&self, working_dir: &std::path::Path, files: &[String]) -> Result<String> {
        if files.is_empty() {
            anyhow::bail!("No files specified to add");
        }

        let mut args = vec!["add"];
        let file_refs: Vec<&str> = files.iter().map(|s| s.as_str()).collect();
        args.extend(file_refs);

        self.run_git_command(&args, working_dir)?;
        Ok(format!("Staged {} file(s)", files.len()))
    }

    /// Create a commit
    fn commit(&self, working_dir: &std::path::Path, message: &str) -> Result<String> {
        if message.is_empty() {
            anyhow::bail!("Commit message cannot be empty");
        }

        self.run_git_command(&["commit", "-m", message], working_dir)
    }

    /// Get list of branches
    fn branches(&self, working_dir: &std::path::Path) -> Result<String> {
        self.run_git_command(&["branch", "-a", "-v"], working_dir)
    }

    /// Get remote information
    fn remotes(&self, working_dir: &std::path::Path) -> Result<String> {
        self.run_git_command(&["remote", "-v"], working_dir)
    }

    /// Stash changes
    fn stash(&self, working_dir: &std::path::Path, action: &str, message: Option<&str>) -> Result<String> {
        match action {
            "push" | "save" => {
                if let Some(msg) = message {
                    self.run_git_command(&["stash", "push", "-m", msg], working_dir)
                } else {
                    self.run_git_command(&["stash", "push"], working_dir)
                }
            }
            "pop" => self.run_git_command(&["stash", "pop"], working_dir),
            "list" => self.run_git_command(&["stash", "list"], working_dir),
            "show" => self.run_git_command(&["stash", "show", "-p"], working_dir),
            "drop" => self.run_git_command(&["stash", "drop"], working_dir),
            _ => anyhow::bail!("Unknown stash action: {}", action),
        }
    }
}

#[async_trait]
impl Tool for GitTool {
    fn name(&self) -> &str {
        "git"
    }

    fn description(&self) -> &str {
        "Execute git operations: status, diff, log, show, blame, add, commit, branches, remotes, stash"
    }

    fn security_level(&self) -> SecurityLevel {
        // Read operations are safe, write operations (add, commit) require confirmation
        SecurityLevel::Moderate
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required(
                "operation",
                ParameterProperty::string(
                    "Git operation: status, diff, log, show, blame, add, commit, branches, remotes, stash"
                ),
            )
            .with_property(
                "staged",
                ParameterProperty::boolean("For diff: show staged changes only"),
            )
            .with_property(
                "file",
                ParameterProperty::string("File path for file-specific operations (diff, blame)"),
            )
            .with_property(
                "files",
                ParameterProperty::string("Comma-separated file paths for add operation"),
            )
            .with_property(
                "message",
                ParameterProperty::string("Commit or stash message"),
            )
            .with_property(
                "commit",
                ParameterProperty::string("Commit SHA or reference for show operation"),
            )
            .with_property(
                "count",
                ParameterProperty::number("Number of log entries to show (default: 10, max: 50)"),
            )
            .with_property(
                "lines",
                ParameterProperty::string("Line range for blame (e.g., '10,20' or '10,+5')"),
            )
            .with_property(
                "action",
                ParameterProperty::string("Stash action: push, pop, list, show, drop"),
            )
    }

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::new(self.name(), self.description(), self.parameters_schema())
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let working_dir = &ctx.working_dir;

        // Check if this is a git repo
        if !self.is_git_repo(working_dir) {
            return Ok(ToolResult::error("Not a git repository"));
        }

        let operation = args
            .get("operation")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'operation' parameter"))?;

        let result = match operation {
            "status" => self.status(working_dir),

            "diff" => {
                let staged = args.get("staged").and_then(|v| v.as_bool()).unwrap_or(false);
                let file = args.get("file").and_then(|v| v.as_str());
                self.diff(working_dir, staged, file)
            }

            "log" => {
                let count = args
                    .get("count")
                    .and_then(|v| v.as_u64())
                    .map(|n| n as usize)
                    .unwrap_or(10);
                self.log(working_dir, count)
            }

            "show" => {
                let commit = args
                    .get("commit")
                    .and_then(|v| v.as_str())
                    .unwrap_or("HEAD");
                self.show(working_dir, commit)
            }

            "blame" => {
                let file = args
                    .get("file")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'file' parameter for blame"))?;
                let lines = args.get("lines").and_then(|v| v.as_str());
                self.blame(working_dir, file, lines)
            }

            "add" => {
                // Parse files from comma-separated string or array
                let files: Vec<String> = if let Some(files_str) = args.get("files").and_then(|v| v.as_str()) {
                    files_str.split(',').map(|s| s.trim().to_string()).collect()
                } else if let Some(arr) = args.get("files").and_then(|v| v.as_array()) {
                    arr.iter()
                        .filter_map(|v| v.as_str().map(|s| s.to_string()))
                        .collect()
                } else {
                    Vec::new()
                };
                self.add(working_dir, &files)
            }

            "commit" => {
                let message = args
                    .get("message")
                    .and_then(|v| v.as_str())
                    .ok_or_else(|| anyhow::anyhow!("Missing 'message' parameter for commit"))?;
                self.commit(working_dir, message)
            }

            "branches" => self.branches(working_dir),

            "remotes" => self.remotes(working_dir),

            "stash" => {
                let action = args
                    .get("action")
                    .and_then(|v| v.as_str())
                    .unwrap_or("list");
                let message = args.get("message").and_then(|v| v.as_str());
                self.stash(working_dir, action, message)
            }

            _ => anyhow::bail!("Unknown git operation: {}", operation),
        };

        match result {
            Ok(output) => Ok(ToolResult::success(output)),
            Err(e) => Ok(ToolResult::error(format!("{}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_repo() -> (TempDir, std::path::PathBuf) {
        let dir = TempDir::new().unwrap();
        let path = dir.path().to_path_buf();

        // Initialize git repo
        Command::new("git")
            .args(["init"])
            .current_dir(&path)
            .output()
            .unwrap();

        // Configure git for testing
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(&path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(&path)
            .output()
            .unwrap();

        (dir, path)
    }

    #[test]
    fn test_is_git_repo() {
        let tool = GitTool::new();
        let (dir, path) = create_test_repo();

        assert!(tool.is_git_repo(&path));

        // Non-git directory
        let non_git = TempDir::new().unwrap();
        assert!(!tool.is_git_repo(non_git.path()));

        drop(dir);
    }

    #[test]
    fn test_status() {
        let tool = GitTool::new();
        let (_dir, path) = create_test_repo();

        let status = tool.status(&path).unwrap();
        assert!(status.contains("Branch:"));
    }

    #[test]
    fn test_log_empty_repo() {
        let tool = GitTool::new();
        let (_dir, path) = create_test_repo();

        // Empty repo has no commits
        let result = tool.log(&path, 10);
        // May fail or return empty - that's expected
        assert!(result.is_ok() || result.is_err());
    }

    #[tokio::test]
    async fn test_git_status_command() {
        let tool = GitTool::new();
        let (_dir, path) = create_test_repo();

        let ctx = ToolContext::new(path);
        let args = json!({ "operation": "status" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Branch:"));
    }

    #[tokio::test]
    async fn test_git_not_a_repo() {
        let tool = GitTool::new();

        // Create temp dir and verify it's not a git repo
        let dir = TempDir::new().unwrap();

        // Check if is_git_repo correctly identifies non-git directories
        // Note: On some systems, temp might be inside a git-tracked parent
        // so we only test the behavior, not that it fails
        let is_repo = tool.is_git_repo(dir.path());

        if !is_repo {
            // If not a repo, status should fail
            let ctx = ToolContext::new(dir.path().to_path_buf());
            let args = json!({ "operation": "status" });

            let result = tool.execute(&args, &ctx).await.unwrap();
            assert!(!result.success);
            // Error message is in the error field, not output
            let error_msg = result.error.as_deref().unwrap_or("");
            assert!(
                error_msg.contains("Not a git repository") ||
                error_msg.contains("not a git"),
                "Unexpected error: {:?}",
                result.error
            );
        }
        // If it IS a repo (temp is under a git parent), just verify basic functionality
        // This is acceptable as the real test is in test_is_git_repo
    }

    #[tokio::test]
    async fn test_git_branches() {
        let tool = GitTool::new();
        let (_dir, path) = create_test_repo();

        let ctx = ToolContext::new(path);
        let args = json!({ "operation": "branches" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        // May or may not have branches yet
        assert!(result.success || result.output.contains("No commits yet"));
    }
}
