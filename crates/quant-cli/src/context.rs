//! Context management for RAG (Retrieval-Augmented Generation)
//!
//! Manages files and directories to include as context in prompts.
//! Includes smart context selection based on query analysis.

#![allow(dead_code)]

use anyhow::{Context, Result};
use glob::glob;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

/// Default include patterns for code files
const DEFAULT_INCLUDE: &[&str] = &[
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
const DEFAULT_EXCLUDE: &[&str] = &[
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
const PROJECT_MARKERS: &[&str] = &[
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "pom.xml",
    "build.gradle",
    ".git",
];

/// Maximum tokens for context (rough estimate: 4 chars per token)
const DEFAULT_MAX_TOKENS: usize = 8000;
const CHARS_PER_TOKEN: usize = 4;

/// Configuration for context
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
        })
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
        let max_chars = self.config.max_tokens * CHARS_PER_TOKEN;

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

        // Add file contents (with truncation)
        for file in all_files {
            if context.len() >= max_chars {
                context.push_str("\n... (truncated due to context limit)\n");
                break;
            }

            if let Ok(content) = fs::read_to_string(&file) {
                let remaining = max_chars.saturating_sub(context.len());
                let file_header = format!("## {}\n\n```\n", file.display());

                if remaining < file_header.len() + 100 {
                    continue; // Skip if not enough room
                }

                context.push_str(&file_header);

                if content.len() > remaining - file_header.len() - 10 {
                    let truncated = &content[..remaining - file_header.len() - 50];
                    context.push_str(truncated);
                    context.push_str("\n... (truncated)\n");
                } else {
                    context.push_str(&content);
                }
                context.push_str("\n```\n\n");
            }
        }

        Ok(context)
    }

    /// Build context from a specific path (for --context flag)
    pub fn build_context_from_path(&self, path: &str) -> Result<String> {
        let mut context = String::new();
        let max_chars = self.config.max_tokens * CHARS_PER_TOKEN;
        let p = Path::new(path);

        let mut all_files: Vec<PathBuf> = Vec::new();

        if p.is_dir() {
            self.collect_files_from_dir(p, &mut all_files)?;
        } else if p.is_file() {
            all_files.push(p.to_path_buf());
        }

        // Sort files
        all_files.sort();

        // Build file tree
        if !all_files.is_empty() {
            context.push_str("## Context Files\n\n");
            context.push_str("```\n");
            for f in &all_files {
                context.push_str(&format!("{}\n", f.display()));
            }
            context.push_str("```\n\n");
        }

        // Add file contents
        for file in all_files {
            if context.len() >= max_chars {
                context.push_str("\n... (truncated due to context limit)\n");
                break;
            }

            if let Ok(content) = fs::read_to_string(&file) {
                let remaining = max_chars.saturating_sub(context.len());
                let file_header = format!("## {}\n\n```\n", file.display());

                if remaining < file_header.len() + 100 {
                    continue;
                }

                context.push_str(&file_header);

                if content.len() > remaining - file_header.len() - 10 {
                    let truncated = &content[..remaining - file_header.len() - 50];
                    context.push_str(truncated);
                    context.push_str("\n... (truncated)\n");
                } else {
                    context.push_str(&content);
                }
                context.push_str("\n```\n\n");
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
        Ok(context.len() / CHARS_PER_TOKEN)
    }

    /// Get token count and warning status
    /// Returns (estimated_tokens, max_tokens, is_truncated)
    pub fn token_status(&self) -> Result<(usize, usize, bool)> {
        let context = self.build_context()?;
        let estimated = context.len() / CHARS_PER_TOKEN;
        let is_truncated = context.contains("(truncated");
        Ok((estimated, self.config.max_tokens, is_truncated))
    }

    /// Check if context is approaching or exceeding limits
    /// Returns a warning message if applicable
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
                    // Check excludes
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

/// Smart context selector that auto-includes relevant files based on query analysis
pub struct SmartContextSelector {
    /// Project root directory
    project_root: PathBuf,
    /// Configuration for context limits
    config: ContextConfig,
    /// Keywords extracted from the query
    keywords: Vec<String>,
}

impl SmartContextSelector {
    /// Create a new smart context selector
    pub fn new(project_root: PathBuf) -> Self {
        Self {
            project_root,
            config: ContextConfig::default(),
            keywords: Vec::new(),
        }
    }

    /// Set the max tokens for context
    pub fn with_max_tokens(mut self, tokens: usize) -> Self {
        self.config.max_tokens = tokens;
        self
    }

    /// Analyze a query and select relevant files
    pub fn select_context(&mut self, query: &str) -> Result<SmartContext> {
        // Extract keywords from the query
        self.keywords = Self::extract_keywords(query);
        debug!(keywords = ?self.keywords, "Extracted keywords from query");

        let mut context = SmartContext::new();
        let max_chars = self.config.max_tokens * CHARS_PER_TOKEN;

        // Priority 1: Find files by name matching keywords
        let name_matches = self.find_files_by_name()?;
        debug!(count = name_matches.len(), "Found files by name match");

        // Priority 2: Find files containing keywords (grep)
        let content_matches = self.find_files_by_content()?;
        debug!(count = content_matches.len(), "Found files by content match");

        // Merge and rank files
        let mut ranked_files = self.rank_files(name_matches, content_matches);
        debug!(count = ranked_files.len(), "Ranked files for context");

        // Read file contents up to the token limit
        let mut current_chars = 0;
        for (path, score) in ranked_files.drain(..) {
            if current_chars >= max_chars {
                break;
            }

            // Skip if file is too large
            if let Ok(metadata) = fs::metadata(&path) {
                if metadata.len() > 50_000 {
                    // Skip files > 50KB
                    continue;
                }
            }

            if let Ok(content) = fs::read_to_string(&path) {
                let file_chars = content.len();

                // Check if we can fit this file
                if current_chars + file_chars + 200 > max_chars {
                    // Try to fit truncated version if file is important (high score)
                    if score > 5.0 && current_chars + 500 < max_chars {
                        let truncated_len = (max_chars - current_chars - 200).min(2000);
                        let truncated = &content[..truncated_len.min(content.len())];
                        context.add_file(path.clone(), truncated.to_string(), true);
                        current_chars += truncated_len + 200;
                    }
                    continue;
                }

                context.add_file(path, content, false);
                current_chars += file_chars + 200; // Account for headers
            }
        }

        debug!(
            files = context.files.len(),
            chars = current_chars,
            "Built smart context"
        );

        Ok(context)
    }

    /// Extract keywords from a query
    fn extract_keywords(query: &str) -> Vec<String> {
        // Common stop words to filter out
        let stop_words: HashSet<&str> = [
            "a", "an", "the", "is", "are", "was", "were", "be", "been", "being",
            "have", "has", "had", "do", "does", "did", "will", "would", "could",
            "should", "may", "might", "must", "shall", "can", "need", "dare",
            "to", "of", "in", "for", "on", "with", "at", "by", "from", "as",
            "into", "through", "during", "before", "after", "above", "below",
            "between", "under", "again", "further", "then", "once", "here",
            "there", "when", "where", "why", "how", "all", "each", "few", "more",
            "most", "other", "some", "such", "no", "nor", "not", "only", "own",
            "same", "so", "than", "too", "very", "just", "and", "but", "if",
            "or", "because", "until", "while", "this", "that", "these", "those",
            "i", "me", "my", "we", "our", "you", "your", "he", "him", "his",
            "she", "her", "it", "its", "they", "them", "their", "what", "which",
            "who", "whom", "file", "files", "code", "function", "functions",
            "find", "search", "look", "show", "list", "create", "add", "remove",
            "delete", "update", "change", "modify", "help", "please", "want",
        ]
        .into_iter()
        .collect();

        // Extract words, filter stop words, and return unique keywords
        let mut keywords: Vec<String> = query
            .to_lowercase()
            .split(|c: char| !c.is_alphanumeric() && c != '_' && c != '-')
            .filter(|s| s.len() >= 3 && !stop_words.contains(*s))
            .map(|s| s.to_string())
            .collect();

        // Deduplicate while preserving order
        let mut seen = HashSet::new();
        keywords.retain(|k| seen.insert(k.clone()));

        // Limit to most important keywords
        keywords.truncate(15);
        keywords
    }

    /// Find files by name matching keywords
    fn find_files_by_name(&self) -> Result<HashMap<PathBuf, f32>> {
        let mut matches: HashMap<PathBuf, f32> = HashMap::new();

        for keyword in &self.keywords {
            // Try glob patterns for this keyword
            let patterns = [
                format!("{}/**/*{}*.rs", self.project_root.display(), keyword),
                format!("{}/**/*{}*.py", self.project_root.display(), keyword),
                format!("{}/**/*{}*.ts", self.project_root.display(), keyword),
                format!("{}/**/*{}*.js", self.project_root.display(), keyword),
                format!("{}/**/*{}*.go", self.project_root.display(), keyword),
                format!("{}/**/*{}*.java", self.project_root.display(), keyword),
                format!("{}/**/*{}*.toml", self.project_root.display(), keyword),
                format!("{}/**/*{}*.yaml", self.project_root.display(), keyword),
                format!("{}/**/*{}*.yml", self.project_root.display(), keyword),
                format!("{}/**/*{}*.md", self.project_root.display(), keyword),
            ];

            for pattern in &patterns {
                if let Ok(paths) = glob(pattern) {
                    for entry in paths.filter_map(|e| e.ok()) {
                        // Skip excluded directories
                        let path_str = entry.to_string_lossy();
                        if self.is_excluded(&path_str) {
                            continue;
                        }

                        // Score based on how well the filename matches
                        let filename = entry
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();

                        let score = if filename.to_lowercase() == *keyword {
                            10.0 // Exact match
                        } else if filename.to_lowercase().starts_with(keyword) {
                            8.0 // Starts with
                        } else if filename.to_lowercase().ends_with(&format!("{}.rs", keyword))
                            || filename.to_lowercase().ends_with(&format!("{}.py", keyword))
                        {
                            7.0 // Ends with (module name)
                        } else {
                            5.0 // Contains
                        };

                        *matches.entry(entry).or_insert(0.0) += score;
                    }
                }
            }
        }

        Ok(matches)
    }

    /// Find files containing keywords in their content
    fn find_files_by_content(&self) -> Result<HashMap<PathBuf, f32>> {
        let mut matches: HashMap<PathBuf, f32> = HashMap::new();

        // Use ripgrep-like search for each keyword
        for keyword in &self.keywords {
            // Search for the keyword in code files
            let code_extensions = ["rs", "py", "ts", "js", "go", "java", "c", "cpp", "h"];

            for ext in &code_extensions {
                let pattern = format!("{}/**/*.{}", self.project_root.display(), ext);
                if let Ok(paths) = glob(&pattern) {
                    for entry in paths.filter_map(|e| e.ok()) {
                        let path_str = entry.to_string_lossy();
                        if self.is_excluded(&path_str) {
                            continue;
                        }

                        // Read and search file
                        if let Ok(content) = fs::read_to_string(&entry) {
                            let content_lower = content.to_lowercase();

                            // Count occurrences and determine relevance
                            let count = content_lower.matches(keyword).count();
                            if count > 0 {
                                // Score based on occurrence count and context
                                let base_score = (count as f32).sqrt(); // Diminishing returns

                                // Bonus for definition-like patterns
                                let def_patterns = [
                                    format!("fn {}", keyword),
                                    format!("def {}", keyword),
                                    format!("function {}", keyword),
                                    format!("class {}", keyword),
                                    format!("struct {}", keyword),
                                    format!("enum {}", keyword),
                                    format!("trait {}", keyword),
                                    format!("impl {}", keyword),
                                    format!("type {}", keyword),
                                    format!("const {}", keyword),
                                ];

                                let def_bonus: f32 = def_patterns
                                    .iter()
                                    .filter(|p| content_lower.contains(*p))
                                    .count() as f32
                                    * 3.0;

                                *matches.entry(entry).or_insert(0.0) += base_score + def_bonus;
                            }
                        }
                    }
                }
            }
        }

        Ok(matches)
    }

    /// Check if a path should be excluded
    fn is_excluded(&self, path: &str) -> bool {
        let excludes = [
            "/target/",
            "/node_modules/",
            "/.git/",
            "/dist/",
            "/build/",
            "/__pycache__/",
            "/venv/",
            "/.venv/",
            "/vendor/",
            "/.idea/",
            "/.vscode/",
        ];
        excludes.iter().any(|e| path.contains(e))
    }

    /// Rank files by combining name and content match scores
    fn rank_files(
        &self,
        name_matches: HashMap<PathBuf, f32>,
        content_matches: HashMap<PathBuf, f32>,
    ) -> Vec<(PathBuf, f32)> {
        let mut combined: HashMap<PathBuf, f32> = HashMap::new();

        // Name matches get higher base weight
        for (path, score) in name_matches {
            *combined.entry(path).or_insert(0.0) += score * 1.5;
        }

        // Content matches add to score
        for (path, score) in content_matches {
            *combined.entry(path).or_insert(0.0) += score;
        }

        // Convert to vec and sort by score descending
        let mut ranked: Vec<(PathBuf, f32)> = combined.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Limit to reasonable number
        ranked.truncate(20);
        ranked
    }
}

/// Container for smart context results
#[derive(Debug, Clone)]
pub struct SmartContext {
    /// Files selected for context
    pub files: Vec<SmartContextFile>,
}

impl SmartContext {
    pub fn new() -> Self {
        Self { files: Vec::new() }
    }

    pub fn add_file(&mut self, path: PathBuf, content: String, truncated: bool) {
        self.files.push(SmartContextFile {
            path,
            content,
            truncated,
        });
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    /// Format context for inclusion in system prompt
    pub fn to_context_string(&self) -> String {
        if self.files.is_empty() {
            return String::new();
        }

        let mut context = String::new();
        context.push_str("## Relevant Files (Auto-selected)\n\n");

        for file in &self.files {
            let rel_path = file.path.to_string_lossy();
            context.push_str(&format!("### {}\n\n", rel_path));
            context.push_str("```\n");
            context.push_str(&file.content);
            if file.truncated {
                context.push_str("\n... (truncated)");
            }
            context.push_str("\n```\n\n");
        }

        context
    }

    /// Get total character count
    pub fn char_count(&self) -> usize {
        self.files.iter().map(|f| f.content.len()).sum()
    }
}

impl Default for SmartContext {
    fn default() -> Self {
        Self::new()
    }
}

/// A file selected by smart context
#[derive(Debug, Clone)]
pub struct SmartContextFile {
    pub path: PathBuf,
    pub content: String,
    pub truncated: bool,
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

    #[test]
    fn test_extract_keywords() {
        let keywords = SmartContextSelector::extract_keywords("Find all functions related to session persistence");
        assert!(keywords.contains(&"session".to_string()));
        assert!(keywords.contains(&"persistence".to_string()));
        // Stop words should be filtered
        assert!(!keywords.contains(&"find".to_string()));
        assert!(!keywords.contains(&"all".to_string()));
        assert!(!keywords.contains(&"the".to_string()));
    }

    #[test]
    fn test_extract_keywords_code_terms() {
        let keywords = SmartContextSelector::extract_keywords("implement the agent_loop with tool_router");
        assert!(keywords.contains(&"implement".to_string()));
        assert!(keywords.contains(&"agent_loop".to_string()));
        assert!(keywords.contains(&"tool_router".to_string()));
    }

    #[test]
    fn test_smart_context_empty() {
        let ctx = SmartContext::new();
        assert!(ctx.is_empty());
        assert_eq!(ctx.char_count(), 0);
        assert!(ctx.to_context_string().is_empty());
    }

    #[test]
    fn test_smart_context_with_file() {
        let mut ctx = SmartContext::new();
        ctx.add_file(
            PathBuf::from("src/test.rs"),
            "fn main() {}".to_string(),
            false,
        );
        assert!(!ctx.is_empty());
        assert_eq!(ctx.files.len(), 1);
        let output = ctx.to_context_string();
        assert!(output.contains("src/test.rs"));
        assert!(output.contains("fn main()"));
    }
}
