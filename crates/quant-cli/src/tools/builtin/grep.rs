//! Grep/search tool

use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use tracing::{debug, instrument, warn};
use walkdir::WalkDir;

use crate::tools::{ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolResult};

/// Tool for searching file contents
pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for a pattern in files. Supports regex patterns. Returns matching lines with file paths and line numbers."
    }

    fn security_level(&self) -> SecurityLevel {
        SecurityLevel::Safe
    }

    fn parameters_schema(&self) -> ParameterSchema {
        ParameterSchema::new()
            .with_required("pattern", ParameterProperty::string("Regex pattern to search for"))
            .with_property("path", ParameterProperty::string("File or directory to search in (default: working directory)"))
            .with_property("glob", ParameterProperty::string("File pattern to filter (e.g., '*.rs', '*.py')"))
            .with_property("case_insensitive", ParameterProperty::boolean("Case insensitive search (default: false)"))
            .with_property("limit", ParameterProperty::number("Maximum number of matches to return (default: 50)").with_default(Value::Number(50.into())))
    }

    #[instrument(skip(self, args, ctx), fields(pattern = tracing::field::Empty))]
    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        let pattern_str = args.get("pattern")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing required parameter: pattern"))?;

        // Record pattern in span (truncate for safety)
        tracing::Span::current().record("pattern", &pattern_str.chars().take(50).collect::<String>().as_str());

        let search_path = args.get("path")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let file_glob = args.get("glob")
            .and_then(|v| v.as_str());

        let case_insensitive = args.get("case_insensitive")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let limit = args.get("limit")
            .and_then(|v| v.as_u64())
            .map(|v| v as usize)
            .unwrap_or(50);

        debug!(path = %search_path.display(), glob = ?file_glob, case_insensitive, limit, "Grep parameters");

        // Compile regex
        let pattern = if case_insensitive {
            format!("(?i){}", pattern_str)
        } else {
            pattern_str.to_string()
        };

        let regex = match Regex::new(&pattern) {
            Ok(r) => r,
            Err(e) => {
                warn!(pattern = %pattern_str, error = %e, "Invalid regex pattern");
                return Ok(ToolResult::error(format!("Invalid regex pattern: {}", e)));
            }
        };

        // Compile file glob pattern if provided
        let glob_pattern = file_glob.map(|g| glob::Pattern::new(g));
        if let Some(Err(e)) = &glob_pattern {
            return Ok(ToolResult::error(format!("Invalid glob pattern: {}", e)));
        }
        let glob_pattern = glob_pattern.transpose().ok().flatten();

        let mut matches: Vec<String> = Vec::new();
        let mut files_searched = 0;

        // Determine if searching a single file or directory
        let search_path = if search_path.is_absolute() {
            search_path
        } else {
            ctx.working_dir.join(search_path)
        };

        if search_path.is_file() {
            // Search single file
            search_file(&search_path, &regex, &mut matches, limit, &ctx.working_dir)?;
            files_searched = 1;
        } else if search_path.is_dir() {
            // Walk directory
            for entry in WalkDir::new(&search_path)
                .follow_links(true)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
            {
                let path = entry.path();

                // Skip hidden files and common non-text directories
                let path_str = path.to_string_lossy();
                if path_str.contains("/.git/")
                    || path_str.contains("/node_modules/")
                    || path_str.contains("/target/")
                    || path_str.contains("/.venv/")
                {
                    continue;
                }

                // Apply glob filter
                if let Some(ref glob) = glob_pattern {
                    if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                        if !glob.matches(name) {
                            continue;
                        }
                    }
                }

                search_file(path, &regex, &mut matches, limit, &ctx.working_dir)?;
                files_searched += 1;

                if matches.len() >= limit {
                    break;
                }
            }
        } else {
            return Ok(ToolResult::error(format!("Path not found: {}", search_path.display())));
        }

        let output = if matches.is_empty() {
            format!(
                "No matches found for '{}' in {} files",
                pattern_str, files_searched
            )
        } else {
            let header = format!(
                "Found {} matches for '{}' in {} files:\n\n",
                matches.len(),
                pattern_str,
                files_searched
            );
            let results = matches.join("\n");
            let truncated = if matches.len() >= limit {
                format!("\n\n[Results truncated at {} matches]", limit)
            } else {
                String::new()
            };
            header + &results + &truncated
        };

        Ok(ToolResult::success(output))
    }
}

fn search_file(
    path: &std::path::Path,
    regex: &Regex,
    matches: &mut Vec<String>,
    limit: usize,
    working_dir: &PathBuf,
) -> Result<()> {
    // Try to read as text
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Ok(()), // Skip binary files
    };

    let display_path = path
        .strip_prefix(working_dir)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| path.to_path_buf());

    for (line_num, line) in content.lines().enumerate() {
        if regex.is_match(line) {
            matches.push(format!(
                "{}:{}:{}",
                display_path.display(),
                line_num + 1,
                line.trim()
            ));

            if matches.len() >= limit {
                return Ok(());
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[tokio::test]
    async fn test_grep_basic() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        fs::write(
            base.join("test.rs"),
            "fn main() {\n    println!(\"Hello\");\n}\n",
        )
        .unwrap();

        let tool = GrepTool;
        let ctx = ToolContext::new(base.to_path_buf());
        let args = json!({ "pattern": "println" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("println"));
        assert!(result.output.contains("test.rs:2"));
    }

    #[tokio::test]
    async fn test_grep_case_insensitive() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        fs::write(base.join("test.txt"), "Hello World\nhello world\n").unwrap();

        let tool = GrepTool;
        let ctx = ToolContext::new(base.to_path_buf());
        let args = json!({
            "pattern": "hello",
            "case_insensitive": true
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("Found 2 matches"));
    }

    #[tokio::test]
    async fn test_grep_with_glob() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        fs::write(base.join("main.rs"), "fn main() {}\n").unwrap();
        fs::write(base.join("lib.rs"), "fn lib() {}\n").unwrap();
        fs::write(base.join("test.txt"), "fn test() {}\n").unwrap();

        let tool = GrepTool;
        let ctx = ToolContext::new(base.to_path_buf());
        let args = json!({
            "pattern": "fn",
            "glob": "*.rs"
        });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("main.rs"));
        assert!(result.output.contains("lib.rs"));
        assert!(!result.output.contains("test.txt"));
    }

    #[tokio::test]
    async fn test_grep_no_matches() {
        let temp_dir = TempDir::new().unwrap();
        let base = temp_dir.path();

        fs::write(base.join("test.txt"), "hello world\n").unwrap();

        let tool = GrepTool;
        let ctx = ToolContext::new(base.to_path_buf());
        let args = json!({ "pattern": "xyz123" });

        let result = tool.execute(&args, &ctx).await.unwrap();
        assert!(result.success);
        assert!(result.output.contains("No matches found"));
    }
}
