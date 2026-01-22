//! Context manager for explicit file/directory inclusion
//!
//! Manages files and directories to include as context in prompts.

use anyhow::{Context, Result};
use glob::glob;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::tokenizer::{count_tokens, Tokenizer};

/// Default include patterns for code files
pub const DEFAULT_INCLUDE: &[&str] = &[
    "**/*.rs",
    "**/*.py",
    "**/*.ts",
    "**/*.tsx",
    "**/*.js",
    "**/*.jsx",
    "**/*.go",
    "**/*.java",
    "**/*.c",
    "**/*.cpp",
    "**/*.h",
    "**/*.hpp",
    "**/*.toml",
    "**/*.yaml",
    "**/*.yml",
    "**/*.json",
    "**/*.md",
];

/// Default exclude patterns
pub const DEFAULT_EXCLUDE: &[&str] = &[
    "**/target/**",
    "**/node_modules/**",
    "**/.git/**",
    "**/dist/**",
    "**/build/**",
    "**/__pycache__/**",
    "**/*.pyc",
    "**/venv/**",
    "**/.venv/**",
    "**/vendor/**",
];

/// Project markers to detect project root
pub const PROJECT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    ".git",
];

/// Default maximum tokens for context
pub const DEFAULT_MAX_TOKENS: usize = 8000;

/// Configuration for context management
#[derive(Debug, Clone)]
pub struct ContextConfig {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    pub max_tokens: usize,
}

impl Default for ContextConfig {
    fn default() -> Self {
        Self {
            include: DEFAULT_INCLUDE.iter().map(|s| s.to_string()).collect(),
            exclude: DEFAULT_EXCLUDE.iter().map(|s| s.to_string()).collect(),
            max_tokens: DEFAULT_MAX_TOKENS,
        }
    }
}

/// Manages context files for prompt injection
pub struct ContextManager {
    /// Explicitly added files/directories
    files: HashSet<String>,
    /// Configuration
    config: ContextConfig,
    /// Path to context state file
    state_path: PathBuf,
    /// Tokenizer for accurate counting
    tokenizer: Tokenizer,
}

impl ContextManager {
    /// Create a new context manager
    pub fn new() -> Result<Self> {
        let state_dir = dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("quant");
        fs::create_dir_all(&state_dir)?;

        let state_path = state_dir.join("context.json");
        let files = if state_path.exists() {
            let content = fs::read_to_string(&state_path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            HashSet::new()
        };

        Ok(Self {
            files,
            config: ContextConfig::default(),
            state_path,
            tokenizer: Tokenizer::default(),
        })
    }

    /// Create with a specific model for tokenization
    pub fn with_model(model: &str) -> Result<Self> {
        let mut manager = Self::new()?;
        manager.tokenizer = Tokenizer::new(model);
        Ok(manager)
    }

    /// Set the max tokens for context
    pub fn set_max_tokens(&mut self, max_tokens: usize) {
        self.config.max_tokens = max_tokens;
    }

    /// Add a file or directory to the context
    pub fn add(&mut self, path: &str) -> Result<()> {
        let path = self.normalize_path(path)?;
        self.files.insert(path);
        Ok(())
    }

    /// Remove a file or directory from context
    pub fn remove(&mut self, path: &str) -> Result<()> {
        let path = self.normalize_path(path)?;
        self.files.remove(&path);
        Ok(())
    }

    /// Clear all context
    pub fn clear(&mut self) {
        self.files.clear();
    }

    /// List all context files
    pub fn list(&self) -> Vec<String> {
        let mut files: Vec<_> = self.files.iter().cloned().collect();
        files.sort();
        files
    }

    /// Save context state
    pub fn save(&self) -> Result<()> {
        let content = serde_json::to_string_pretty(&self.files)?;
        fs::write(&self.state_path, content)?;
        Ok(())
    }

    /// Build context string from current files
    pub fn build_context(&self) -> Result<String> {
        let mut context = String::new();
        let max_tokens = self.config.max_tokens;

        // Collect all files
        let mut all_files: Vec<PathBuf> = Vec::new();

        for path in &self.files {
            let p = Path::new(path);
            if p.is_dir() {
                self.collect_files_from_dir(p, &mut all_files)?;
            } else if p.is_file() {
                all_files.push(p.to_path_buf());
            }
        }

        // Deduplicate and sort
        all_files.sort();
        all_files.dedup();

        // Build file tree
        if !all_files.is_empty() {
            context.push_str("## Project Files\n\n");
            context.push_str("```\n");
            for f in &all_files {
                context.push_str(&format!("{}\n", f.display()));
            }
            context.push_str("```\n\n");
        }

        // Add file contents (with token-aware truncation)
        let mut current_tokens = self.tokenizer.count_tokens(&context);

        for file in all_files {
            if current_tokens >= max_tokens {
                context.push_str("\n... (truncated due to context limit)\n");
                break;
            }

            if let Ok(content) = fs::read_to_string(&file) {
                let file_header = format!("## {}\n\n```\n", file.display());
                let file_footer = "\n```\n\n";

                let header_tokens = self.tokenizer.count_tokens(&file_header);
                let footer_tokens = self.tokenizer.count_tokens(file_footer);
                let content_tokens = self.tokenizer.count_tokens(&content);

                let remaining_tokens = max_tokens.saturating_sub(current_tokens);

                if header_tokens + footer_tokens + 10 > remaining_tokens {
                    continue; // Not enough room for anything meaningful
                }

                context.push_str(&file_header);

                let available_for_content = remaining_tokens - header_tokens - footer_tokens;

                if content_tokens > available_for_content {
                    let truncated = self
                        .tokenizer
                        .truncate_to_tokens(&content, available_for_content - 10);
                    context.push_str(&truncated);
                    context.push_str("\n... (truncated)\n");
                } else {
                    context.push_str(&content);
                }
                context.push_str(file_footer);

                current_tokens = self.tokenizer.count_tokens(&context);
            }
        }

        Ok(context)
    }

    /// Build context from a specific path (for --context flag)
    pub fn build_context_from_path(&self, path: &str) -> Result<String> {
        let mut context = String::new();
        let max_tokens = self.config.max_tokens;
        let p = Path::new(path);

        let mut all_files: Vec<PathBuf> = Vec::new();

        if p.is_dir() {
            self.collect_files_from_dir(p, &mut all_files)?;
        } else if p.is_file() {
            all_files.push(p.to_path_buf());
        }

        all_files.sort();

        if !all_files.is_empty() {
            context.push_str("## Context Files\n\n");
            context.push_str("```\n");
            for f in &all_files {
                context.push_str(&format!("{}\n", f.display()));
            }
            context.push_str("```\n\n");
        }

        let mut current_tokens = self.tokenizer.count_tokens(&context);

        for file in all_files {
            if current_tokens >= max_tokens {
                context.push_str("\n... (truncated due to context limit)\n");
                break;
            }

            if let Ok(content) = fs::read_to_string(&file) {
                let file_header = format!("## {}\n\n```\n", file.display());
                let content_tokens = self.tokenizer.count_tokens(&content);
                let header_tokens = self.tokenizer.count_tokens(&file_header);
                let remaining = max_tokens.saturating_sub(current_tokens);

                if header_tokens + 20 > remaining {
                    continue;
                }

                context.push_str(&file_header);

                if content_tokens + header_tokens + 10 > remaining {
                    let available = remaining - header_tokens - 20;
                    let truncated = self.tokenizer.truncate_to_tokens(&content, available);
                    context.push_str(&truncated);
                    context.push_str("\n... (truncated)\n");
                } else {
                    context.push_str(&content);
                }
                context.push_str("\n```\n\n");

                current_tokens = self.tokenizer.count_tokens(&context);
            }
        }

        Ok(context)
    }

    /// Find project root by looking for marker files
    pub fn find_project_root() -> Option<PathBuf> {
        let mut current = std::env::current_dir().ok()?;

        for _ in 0..10 {
            for marker in PROJECT_MARKERS {
                if current.join(marker).exists() {
                    return Some(current);
                }
            }
            if !current.pop() {
                break;
            }
        }

        None
    }

    /// Get estimated token count
    pub fn token_count(&self) -> Result<usize> {
        let context = self.build_context()?;
        Ok(self.tokenizer.count_tokens(&context))
    }

    /// Get token count and warning status
    pub fn token_status(&self) -> Result<(usize, usize, bool)> {
        let context = self.build_context()?;
        let estimated = self.tokenizer.count_tokens(&context);
        let is_truncated = context.contains("(truncated");
        Ok((estimated, self.config.max_tokens, is_truncated))
    }

    /// Check if context is approaching or exceeding limits
    pub fn check_limits(&self) -> Result<Option<String>> {
        let (tokens, max_tokens, is_truncated) = self.token_status()?;

        if is_truncated {
            return Ok(Some(format!(
                "Context truncated: ~{} tokens (max: {}). Some files were omitted.",
                tokens, max_tokens
            )));
        }

        let threshold = (max_tokens as f64 * 0.8) as usize;
        if tokens > threshold {
            return Ok(Some(format!(
                "Context approaching limit: ~{} tokens (max: {})",
                tokens, max_tokens
            )));
        }

        Ok(None)
    }

    // Private helpers

    fn normalize_path(&self, path: &str) -> Result<String> {
        let p = Path::new(path);
        let absolute = if p.is_absolute() {
            p.to_path_buf()
        } else {
            std::env::current_dir()?.join(p)
        };

        absolute
            .canonicalize()
            .map(|p| p.to_string_lossy().to_string())
            .context("Failed to resolve path")
    }

    fn collect_files_from_dir(&self, dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
        let dir_str = dir.to_string_lossy();

        for pattern in &self.config.include {
            let full_pattern = format!("{}/{}", dir_str, pattern);

            for entry in glob(&full_pattern).context("Invalid glob pattern")? {
                if let Ok(path) = entry {
                    let _path_str = path.to_string_lossy();
                    let excluded = self.config.exclude.iter().any(|exc| {
                        let exc_pattern = format!("{}/{}", dir_str, exc);
                        glob(&exc_pattern)
                            .ok()
                            .map(|mut g| g.any(|e| e.ok().map(|p| p == path).unwrap_or(false)))
                            .unwrap_or(false)
                    });

                    if !excluded && path.is_file() {
                        files.push(path);
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = ContextConfig::default();
        assert!(!config.include.is_empty());
        assert!(!config.exclude.is_empty());
        assert_eq!(config.max_tokens, DEFAULT_MAX_TOKENS);
    }
}
