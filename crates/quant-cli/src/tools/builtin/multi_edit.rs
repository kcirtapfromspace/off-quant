//! Multi-file atomic edit tool
//!
//! Provides transactional editing of multiple files - all edits succeed or all are rolled back.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, info, warn};

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Tool for atomically editing multiple files
pub struct MultiEditTool;

/// A single file edit operation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileEdit {
    /// Path to the file (absolute or relative)
    pub path: String,
    /// The old content to find and replace (if None, creates new file)
    pub old_content: Option<String>,
    /// The new content to write
    pub new_content: String,
    /// Whether to create the file if it doesn't exist
    #[serde(default)]
    pub create_if_missing: bool,
}

/// Backup of original file state for rollback
#[derive(Debug)]
struct FileBackup {
    path: PathBuf,
    original_content: Option<String>, // None if file didn't exist
    existed: bool,
}

impl FileBackup {
    fn capture(path: &PathBuf) -> Self {
        let existed = path.exists();
        let original_content = if existed {
            fs::read_to_string(path).ok()
        } else {
            None
        };

        Self {
            path: path.clone(),
            original_content,
            existed,
        }
    }

    fn restore(&self) -> Result<()> {
        if self.existed {
            if let Some(ref content) = self.original_content {
                fs::write(&self.path, content)?;
            }
        } else {
            // File didn't exist before, remove it
            if self.path.exists() {
                fs::remove_file(&self.path)?;
            }
        }
        Ok(())
    }
}

#[async_trait]
impl Tool for MultiEditTool {
    fn name(&self) -> &str {
        "multi_edit"
    }

    fn description(&self) -> &str {
        "Atomically edit multiple files. All edits succeed together or all are rolled back. Use for refactoring that spans multiple files."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Dangerous
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required(
                "edits",
                ParameterProperty::array("Array of file edits. Each edit has: path, old_content (optional), new_content, create_if_missing (optional)")
            )
            .with_property(
                "description",
                ParameterProperty::string("Description of what this batch edit accomplishes")
            )
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let edits_value = args.get("edits")
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: edits"))?;

        let edits: Vec<FileEdit> = serde_json::from_value(edits_value.clone())
            .map_err(|e| anyhow::anyhow!("Invalid edits format: {}", e))?;

        if edits.is_empty() {
            return Ok(ToolResult::error("No edits provided"));
        }

        let description = args.get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("Multi-file edit");

        info!(edit_count = edits.len(), description, "Starting atomic multi-file edit");

        // Phase 1: Validate all edits and capture backups
        let mut backups: Vec<FileBackup> = Vec::new();
        let mut resolved_edits: Vec<(PathBuf, &FileEdit)> = Vec::new();

        for edit in &edits {
            let path = if PathBuf::from(&edit.path).is_absolute() {
                PathBuf::from(&edit.path)
            } else {
                ctx.working_dir.join(&edit.path)
            };

            // Validate path is within working directory
            let canonical_ctx = ctx.working_dir.canonicalize()
                .map_err(|e| anyhow::anyhow!("Failed to resolve working directory: {}", e))?;

            if path.exists() {
                let canonical_path = path.canonicalize()
                    .map_err(|e| anyhow::anyhow!("Failed to resolve path {}: {}", edit.path, e))?;

                if !canonical_path.starts_with(&canonical_ctx) {
                    return Ok(ToolResult::error(format!(
                        "Path {} is outside working directory",
                        edit.path
                    )));
                }
            }

            // Check if file exists when old_content is specified
            if edit.old_content.is_some() && !path.exists() {
                return Ok(ToolResult::error(format!(
                    "File {} does not exist but old_content was specified",
                    edit.path
                )));
            }

            // Check if we need to create the file
            if !path.exists() && !edit.create_if_missing && edit.old_content.is_none() {
                return Ok(ToolResult::error(format!(
                    "File {} does not exist and create_if_missing is false",
                    edit.path
                )));
            }

            // Verify old_content matches if specified
            if let Some(ref old_content) = edit.old_content {
                let current = fs::read_to_string(&path)
                    .map_err(|e| anyhow::anyhow!("Failed to read {}: {}", edit.path, e))?;

                if !current.contains(old_content) {
                    return Ok(ToolResult::error(format!(
                        "File {} does not contain expected old_content. The file may have been modified.",
                        edit.path
                    )));
                }
            }

            // Capture backup
            backups.push(FileBackup::capture(&path));
            resolved_edits.push((path, edit));
        }

        debug!(backup_count = backups.len(), "Captured backups for rollback");

        // Phase 2: Apply all edits
        let mut applied_count = 0;
        let mut results: Vec<String> = Vec::new();

        for (path, edit) in &resolved_edits {
            let apply_result = apply_edit(path, edit);

            match apply_result {
                Ok(msg) => {
                    applied_count += 1;
                    results.push(msg);
                    debug!(path = %path.display(), "Applied edit");
                }
                Err(e) => {
                    warn!(path = %path.display(), error = %e, "Edit failed, rolling back");

                    // Phase 3: Rollback on failure
                    for backup in &backups {
                        if let Err(restore_err) = backup.restore() {
                            warn!(
                                path = %backup.path.display(),
                                error = %restore_err,
                                "Failed to restore backup during rollback"
                            );
                        }
                    }

                    return Ok(ToolResult::error(format!(
                        "Edit failed for {}: {}. All changes have been rolled back.",
                        path.display(),
                        e
                    )));
                }
            }
        }

        info!(
            applied_count,
            description,
            "Successfully completed atomic multi-file edit"
        );

        let summary = format!(
            "Successfully applied {} edit(s):\n{}",
            applied_count,
            results.join("\n")
        );

        Ok(ToolResult::success(summary))
    }
}

/// Apply a single edit to a file
fn apply_edit(path: &PathBuf, edit: &FileEdit) -> Result<String> {
    // Create parent directories if needed
    if let Some(parent) = path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)?;
        }
    }

    let new_content = if let Some(ref old_content) = edit.old_content {
        // Replace old content with new content
        let current = fs::read_to_string(path)?;
        current.replace(old_content, &edit.new_content)
    } else {
        // Write new content directly
        edit.new_content.clone()
    };

    fs::write(path, &new_content)?;

    let action = if edit.old_content.is_some() {
        "replaced content in"
    } else if edit.create_if_missing {
        "created"
    } else {
        "wrote"
    };

    Ok(format!("  - {} {} ({} bytes)", action, path.display(), new_content.len()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_multi_edit_create_files() {
        let temp_dir = TempDir::new().unwrap();
        let tool = MultiEditTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "description": "Create two test files",
            "edits": [
                {
                    "path": "file1.txt",
                    "new_content": "content of file 1",
                    "create_if_missing": true
                },
                {
                    "path": "file2.txt",
                    "new_content": "content of file 2",
                    "create_if_missing": true
                }
            ]
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success, "Expected success but got: {:?}", result.error);

        // Verify files were created
        let file1 = fs::read_to_string(temp_dir.path().join("file1.txt")).unwrap();
        let file2 = fs::read_to_string(temp_dir.path().join("file2.txt")).unwrap();
        assert_eq!(file1, "content of file 1");
        assert_eq!(file2, "content of file 2");
    }

    #[tokio::test]
    async fn test_multi_edit_replace_content() {
        let temp_dir = TempDir::new().unwrap();

        // Create initial files
        fs::write(temp_dir.path().join("a.rs"), "fn old_name() {}").unwrap();
        fs::write(temp_dir.path().join("b.rs"), "use crate::old_name;").unwrap();

        let tool = MultiEditTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "description": "Rename function across files",
            "edits": [
                {
                    "path": "a.rs",
                    "old_content": "old_name",
                    "new_content": "new_name"
                },
                {
                    "path": "b.rs",
                    "old_content": "old_name",
                    "new_content": "new_name"
                }
            ]
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success, "Expected success but got: {:?}", result.error);

        // Verify replacements
        let a = fs::read_to_string(temp_dir.path().join("a.rs")).unwrap();
        let b = fs::read_to_string(temp_dir.path().join("b.rs")).unwrap();
        assert_eq!(a, "fn new_name() {}");
        assert_eq!(b, "use crate::new_name;");
    }

    #[tokio::test]
    async fn test_multi_edit_rollback_on_failure() {
        let temp_dir = TempDir::new().unwrap();

        // Create first file, but not second
        fs::write(temp_dir.path().join("exists.txt"), "original content").unwrap();

        let tool = MultiEditTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "description": "Should fail and rollback",
            "edits": [
                {
                    "path": "exists.txt",
                    "old_content": "original",
                    "new_content": "modified"
                },
                {
                    "path": "does_not_exist.txt",
                    "old_content": "something",  // This will fail - file doesn't exist
                    "new_content": "new"
                }
            ]
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(!result.success, "Expected failure");

        // Verify first file was NOT modified (rollback worked)
        let content = fs::read_to_string(temp_dir.path().join("exists.txt")).unwrap();
        assert_eq!(content, "original content", "Rollback should have restored original content");
    }

    #[tokio::test]
    async fn test_multi_edit_nested_dirs() {
        let temp_dir = TempDir::new().unwrap();
        let tool = MultiEditTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "description": "Create files in nested directories",
            "edits": [
                {
                    "path": "a/b/c/deep.txt",
                    "new_content": "deep content",
                    "create_if_missing": true
                }
            ]
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success, "Expected success but got: {:?}", result.error);

        let content = fs::read_to_string(temp_dir.path().join("a/b/c/deep.txt")).unwrap();
        assert_eq!(content, "deep content");
    }

    #[tokio::test]
    async fn test_multi_edit_empty_edits() {
        let temp_dir = TempDir::new().unwrap();
        let tool = MultiEditTool;
        let ctx = ToolContext::new(temp_dir.path().to_path_buf());

        let args = json!({
            "edits": []
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("No edits"));
    }
}
