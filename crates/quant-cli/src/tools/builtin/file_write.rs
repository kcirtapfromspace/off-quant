//! File write tool

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Tool for writing file contents
pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file. Creates the file if it doesn't exist, overwrites if it does. Creates parent directories as needed."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Dangerous
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("path", ParameterProperty::string("The path to write to (absolute or relative)"))
            .with_required("content", ParameterProperty::string("The content to write to the file"))
            .with_property("append", ParameterProperty::boolean("Append to file instead of overwriting (default: false)"))
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let path_str = args.get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: path"))?;

        let content = args.get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: content"))?;

        let append = args.get("append")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // Resolve path relative to working directory
        let path = if PathBuf::from(path_str).is_absolute() {
            PathBuf::from(path_str)
        } else {
            ctx.working_dir.join(path_str)
        };

        // Create parent directories if needed
        if let Some(parent) = path.parent() {
            if !parent.exists() {
                if let Err(e) = fs::create_dir_all(parent) {
                    return Ok(ToolResult::error(format!("Failed to create directories: {}", e)));
                }
            }
        }

        // Write the file
        let result = if append {
            use std::io::Write;
            let file = fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path);

            match file {
                Ok(mut f) => f.write_all(content.as_bytes()),
                Err(e) => Err(e),
            }
        } else {
            fs::write(&path, content)
        };

        match result {
            Ok(()) => {
                let mode = if append { "appended to" } else { "written to" };
                let bytes = content.len();
                Ok(ToolResult::success(format!(
                    "Successfully {} {} ({} bytes)",
                    mode,
                    path.display(),
                    bytes
                )))
            }
            Err(e) => Ok(ToolResult::error(format!("Failed to write file: {}", e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_write_file() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("test.txt");

        let tool = FileWriteTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());
        let args = json!({
            "path": file_path.to_str().unwrap(),
            "content": "Hello, World!"
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "Hello, World!");
    }

    #[tokio::test]
    async fn test_write_file_creates_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("a/b/c/test.txt");

        let tool = FileWriteTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());
        let args = json!({
            "path": file_path.to_str().unwrap(),
            "content": "nested content"
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(file_path.exists());
    }

    #[tokio::test]
    async fn test_write_file_append() {
        let temp_dir = TempDir::new().unwrap();
        let file_path = temp_dir.path().join("append.txt");

        // Write initial content
        fs::write(&file_path, "line1\n").unwrap();

        let tool = FileWriteTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());
        let args = json!({
            "path": file_path.to_str().unwrap(),
            "content": "line2\n",
            "append": true
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "line1\nline2\n");
    }
}
