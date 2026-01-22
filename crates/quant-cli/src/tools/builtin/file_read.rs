//! File read tool

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Tool for reading file contents
pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read the contents of a file. Returns the file content as text. For binary files, returns an error."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Safe
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("path", ParameterProperty::string("The path to the file to read (absolute or relative to working directory)"))
            .with_property("offset", ParameterProperty::number("Line number to start reading from (1-indexed, default: 1)").with_default(Value::Number(1.into())))
            .with_property("limit", ParameterProperty::number("Maximum number of lines to read (default: unlimited)"))
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path_str = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let offset = args.get("offset")
            .and_then(|v| v.as_u64())
            .map(|v| v.saturating_sub(1) as usize) // Convert to 0-indexed
            .unwrap_or(0);

        let limit = args.get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize);

        // Resolve path relative to working directory
        let path = if PathBuf::from(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            ctx.working_dir.join(path_str)
        };

        // Check if file exists
        if !path.exists() {
            return Ok(ToolResult::error(format!("File not found: {}", path.display())));
        }

        // Check if it's a file (not a directory)
        if !path.is_file() {
            return Ok(ToolResult::error(format!("Not a file: {}", path.display())));
        }

        // Read the file
        let content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) => {
                return Ok(ToolResult::error(format!("Failed to read file: {}", e)));
            }
        };

        // Apply offset and limit
        let lines: Vec<&str> = content.lines().collect();
        let total_lines = lines.len();

        let selected_lines: Vec<_> = lines
            .into_iter()
            .skip(offset)
            .take(limit.unwrap_or(usize::MAX))
            .enumerate()
            .map(|(i, line)| format!("{:>6}\t{}", offset + i + 1, line))
            .collect();

        let output = if selected_lines.is_empty() {
            format!("File is empty or offset {} exceeds file length ({} lines)", offset + 1, total_lines)
        } else {
            let header = format!("File: {} ({} lines total)\n", path.display(), total_lines);
            header + &selected_lines.join("\n")
        };

        // Truncate if too long (UTF-8 safe)
        let output = if output.len() > ctx.max_output_len {
            // Find a safe truncation point at a char boundary
            let safe_end = output
                .char_indices()
                .take_while(|(idx, _)| *idx < ctx.max_output_len)
                .last()
                .map(|(idx, c)| idx + c.len_utf8())
                .unwrap_or(0);
            format!(
                "{}\n\n[Output truncated at {} characters]",
                &output[..safe_end],
                safe_end
            )
        } else {
            output
        };

        Ok(ToolResult::success(output))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;
    use tempfile::NamedTempFile;

    #[tokio::test]
    async fn test_read_file() {
        let mut temp = NamedTempFile::new().unwrap();
        writeln!(temp, "line 1").unwrap();
        writeln!(temp, "line 2").unwrap();
        writeln!(temp, "line 3").unwrap();

        let tool = FileReadTool;
        let ctx = ToolContext::default();
        let args = json!({ "path": temp.path().to_str().unwrap() });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("line 1"));
        assert!(result.output.contains("line 2"));
        assert!(result.output.contains("line 3"));
    }

    #[tokio::test]
    async fn test_read_file_with_offset_limit() {
        let mut temp = NamedTempFile::new().unwrap();
        for i in 1..=10 {
            writeln!(temp, "line {}", i).unwrap();
        }

        let tool = FileReadTool;
        let ctx = ToolContext::default();
        let args = json!({
            "path": temp.path().to_str().unwrap(),
            "offset": 3,
            "limit": 2
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("line 3"));
        assert!(result.output.contains("line 4"));
        assert!(!result.output.contains("line 5"));
    }

    #[tokio::test]
    async fn test_read_nonexistent_file() {
        let tool = FileReadTool;
        let ctx = ToolContext::default();
        let args = json!({ "path": "/nonexistent/path/file.txt" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("File not found"));
    }
}
