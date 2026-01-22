//! Glob pattern matching tool

use anyhow::Result;
use async_trait::async_trait;
use glob::glob as glob_match;
use serde_json::Value;
use std::path::PathBuf;

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Tool for finding files matching a glob pattern
pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Find files matching a glob pattern. Supports patterns like '**/*.rs', 'src/**/*.ts', etc."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Safe
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("pattern", ParameterProperty::string("Glob pattern to match (e.g., '**/*.rs', 'src/**/*.ts')"))
            .with_property("path", ParameterProperty::string("Base directory to search in (default: working directory)"))
            .with_property("limit", ParameterProperty::number("Maximum number of results to return (default: 100)").with_default(Value::Number(100.into())))
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern = args.get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        let base_path = args.get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let limit = args.get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(100);

        // Construct the full pattern
        let full_pattern = if PathBuf::from(pattern).is_absolute() {
            pattern.to_string()
        } else {
            format!("{}/{}", base_path.display(), pattern)
        };

        // Execute glob
        let entries = match glob_match(&full_pattern) {
            Ok(paths) => paths,
            Err(e) => {
                return Ok(ToolResult::error(format!("Invalid glob pattern: {}", e)));
            }
        };

        let mut matches: Vec<String> = Vec::new();
        let mut errors: Vec<String> = Vec::new();

        for entry in entries {
            match entry {
                Ok(path) => {
                    // Make path relative to working dir if possible
                    let display_path = path
                        .strip_prefix(&ctx.working_dir)
                        .map(|p| p.to_path_buf())
                        .unwrap_or(path);
                    matches.push(display_path.display().to_string());

                    if matches.len() >= limit {
                        break;
                    }
                }
                Err(e) => {
                    errors.push(e.to_string());
                }
            }
        }

        // Sort matches for consistent output
        matches.sort();

        let output = if matches.is_empty() {
            format!("No files found matching pattern: {}", pattern)
        } else {
            let header = format!("Found {} files matching '{}':\n", matches.len(), pattern);
            let files = matches.join("\n");
            let truncated = if matches.len() >= limit {
                format!("\n\n[Results truncated at {} files]", limit)
            } else {
                String::new()
            };
            header + &files + &truncated
        };

        if !errors.is_empty() {
            Ok(ToolResult::failure(
                output,
                format!("Some paths could not be read: {}", errors.join(", ")),
            ))
        } else {
            Ok(ToolResult::success(output))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_glob_pattern() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        // Create test files
        fs::create_dir_all(base.join("src")).unwrap();
        fs::write(base.join("src/main.rs"), "// main").unwrap();
        fs::write(base.join("src/lib.rs"), "// lib").unwrap();
        fs::write(base.join("README.md"), "# Readme").unwrap();

        let tool = GlobTool;
        let ctx = ToolContext::new(base.to_path_buf());
        let args = json!({ "pattern": "**/*.rs" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
        assert!(!result.output.contains("README.md"));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let temp_dir = TempDir::new().unwrap();

        let tool = GlobTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());
        let args = json!({ "pattern": "**/*.xyz" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No files found"));
    }

    #[tokio::test]
    async fn test_glob_with_limit() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        // Create multiple files
        for i in 0..10 {
            fs::write(base.join(format!("file{}.txt", i)), "content").unwrap();
        }

        let tool = GlobTool;
        let ctx = ToolContext::new(base.to_path_buf());
        let args = json!({ "pattern": "*.txt", "limit": 5 });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("truncated"));
    }
}
