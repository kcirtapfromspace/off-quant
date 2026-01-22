//! Smart context selection with keyword and semantic search
//!
//! Auto-includes relevant files based on query analysis using both
//! keyword matching and optional embedding-based semantic search.

use anyhow::Result;
use glob::glob;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use tracing::debug;

use super::index::FileIndex;
use super::manager::ContextConfig;
use super::tokenizer::{count_tokens, Tokenizer};

#[cfg(feature = "embeddings")]
use super::embeddings::EmbeddingEngine;

/// Smart context selector that auto-includes relevant files
pub struct SmartContextSelector {
    /// Project root directory
    project_root: PathBuf,
    /// Configuration for context limits
    config: ContextConfig,
    /// Keywords extracted from the query
    keywords: Vec<String>,
    /// File index for efficient metadata access
    file_index: Option<FileIndex>,
    /// Embedding engine for semantic search
    #[cfg(feature = "embeddings")]
    embedding_engine: Option<EmbeddingEngine>,
    /// Tokenizer for accurate counting
    tokenizer: Tokenizer,
}

impl SmartContextSelector {
    /// Create a new smart context selector
    pub fn new(project_root: PathBuf) -> Self {
        let file_index = FileIndex::new(project_root.clone()).ok();

        #[cfg(feature = "embeddings")]
        let embedding_engine = {
            let cache_dir = dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("quant");
            EmbeddingEngine::new(super::embeddings::DEFAULT_MODEL, &cache_dir).ok()
        };

        Self {
            project_root,
            config: ContextConfig::default(),
            keywords: Vec::new(),
            file_index,
            #[cfg(feature = "embeddings")]
            embedding_engine,
            tokenizer: Tokenizer::default(),
        }
    }

    /// Set the max tokens for context
    pub fn with_max_tokens(mut self, tokens: usize) -> Self {
        self.config.max_tokens = tokens;
        self
    }

    /// Set the tokenizer for a specific model
    pub fn with_model(mut self, model: &str) -> Self {
        self.tokenizer = Tokenizer::new(model);
        self
    }

    /// Analyze a query and select relevant files
    pub fn select_context(&mut self, query: &str) -> Result<SmartContext> {
        // Extract keywords from the query
        self.keywords = Self::extract_keywords(query);
        debug!(keywords = ?self.keywords, "Extracted keywords from query");

        let mut context = SmartContext::new();
        let max_tokens = self.config.max_tokens;

        // Priority 1: Find files by name matching keywords
        let name_matches = self.find_files_by_name()?;
        debug!(count = name_matches.len(), "Found files by name match");

        // Priority 2: Find files containing keywords (grep)
        let content_matches = self.find_files_by_content()?;
        debug!(count = content_matches.len(), "Found files by content match");

        // Priority 3: Semantic search using embeddings (if available)
        #[cfg(feature = "embeddings")]
        let semantic_matches = self.find_files_by_semantics(query)?;
        #[cfg(not(feature = "embeddings"))]
        let semantic_matches: HashMap<PathBuf, f32> = HashMap::new();

        debug!(count = semantic_matches.len(), "Found files by semantic match");

        // Merge and rank files
        let mut ranked_files = self.rank_files(name_matches, content_matches, semantic_matches);
        debug!(count = ranked_files.len(), "Ranked files for context");

        // Read file contents up to the token limit
        let mut current_tokens = 0;
        for (path, score) in ranked_files.drain(..) {
            if current_tokens >= max_tokens {
                break;
            }

            // Get file metadata from index if available
            let file_size = if let Some(ref index) = self.file_index {
                index.get(&path).map(|m| m.size).unwrap_or(0)
            } else {
                fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
            };

            // Skip files that are too large
            if file_size > 50_000 {
                continue;
            }

            if let Ok(content) = fs::read_to_string(&path) {
                let file_tokens = self.tokenizer.count_tokens(&content);

                // Check if we can fit this file
                if current_tokens + file_tokens + 50 > max_tokens {
                    // Try to fit truncated version if file is important (high score)
                    if score > 5.0 && current_tokens + 500 < max_tokens {
                        let available_tokens = max_tokens - current_tokens - 100;
                        let truncated = self
                            .tokenizer
                            .truncate_to_tokens(&content, available_tokens.min(500));
                        context.add_file(path.clone(), truncated, true);
                        current_tokens += self
                            .tokenizer
                            .count_tokens(context.files.last().map(|f| f.content.as_str()).unwrap_or(""));
                    }
                    continue;
                }

                context.add_file(path, content, false);
                current_tokens += file_tokens + 50; // Account for headers
            }
        }

        debug!(
            files = context.files.len(),
            tokens = current_tokens,
            "Built smart context"
        );

        Ok(context)
    }

    /// Extract keywords from a query
    pub fn extract_keywords(query: &str) -> Vec<String> {
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
                        let path_str = entry.to_string_lossy();
                        if self.is_excluded(&path_str) {
                            continue;
                        }

                        let filename = entry
                            .file_name()
                            .map(|n| n.to_string_lossy().to_string())
                            .unwrap_or_default();

                        let score = if filename.to_lowercase() == *keyword {
                            10.0
                        } else if filename.to_lowercase().starts_with(keyword) {
                            8.0
                        } else if filename.to_lowercase().ends_with(&format!("{}.rs", keyword))
                            || filename.to_lowercase().ends_with(&format!("{}.py", keyword))
                        {
                            7.0
                        } else {
                            5.0
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

        for keyword in &self.keywords {
            let code_extensions = ["rs", "py", "ts", "js", "go", "java", "c", "cpp", "h"];

            for ext in &code_extensions {
                let pattern = format!("{}/**/*.{}", self.project_root.display(), ext);
                if let Ok(paths) = glob(&pattern) {
                    for entry in paths.filter_map(|e| e.ok()) {
                        let path_str = entry.to_string_lossy();
                        if self.is_excluded(&path_str) {
                            continue;
                        }

                        if let Ok(content) = fs::read_to_string(&entry) {
                            let content_lower = content.to_lowercase();

                            let count = content_lower.matches(keyword).count();
                            if count > 0 {
                                let base_score = (count as f32).sqrt();

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

    /// Find files using semantic search (embedding similarity)
    #[cfg(feature = "embeddings")]
    fn find_files_by_semantics(&self, query: &str) -> Result<HashMap<PathBuf, f32>> {
        let mut matches: HashMap<PathBuf, f32> = HashMap::new();

        if let Some(ref engine) = self.embedding_engine {
            if engine.is_available() {
                // Generate query embedding
                if let Ok(query_embedding) = engine.embed(query) {
                    // Search for similar files
                    let results = engine.search(&query_embedding, 10);

                    for (path, similarity) in results {
                        if similarity > 0.3 {
                            // Only include if similarity is meaningful
                            *matches.entry(path).or_insert(0.0) += similarity * 5.0;
                        }
                    }
                }
            }
        }

        Ok(matches)
    }

    #[cfg(not(feature = "embeddings"))]
    fn find_files_by_semantics(&self, _query: &str) -> Result<HashMap<PathBuf, f32>> {
        Ok(HashMap::new())
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

    /// Rank files by combining name, content, and semantic match scores
    fn rank_files(
        &self,
        name_matches: HashMap<PathBuf, f32>,
        content_matches: HashMap<PathBuf, f32>,
        semantic_matches: HashMap<PathBuf, f32>,
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

        // Semantic matches add to score
        for (path, score) in semantic_matches {
            *combined.entry(path).or_insert(0.0) += score;
        }

        // Convert to vec and sort by score descending
        let mut ranked: Vec<(PathBuf, f32)> = combined.into_iter().collect();
        ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        // Limit to reasonable number
        ranked.truncate(20);
        ranked
    }

    /// Index files for faster subsequent searches
    pub fn index_files(&self) -> Result<usize> {
        if let Some(ref index) = self.file_index {
            let code_extensions = ["rs", "py", "ts", "js", "go", "java", "c", "cpp", "h", "md"];
            let mut count = 0;

            for ext in &code_extensions {
                let pattern = format!("{}/**/*.{}", self.project_root.display(), ext);
                if let Ok(paths) = glob(&pattern) {
                    for entry in paths.filter_map(|e| e.ok()) {
                        let path_str = entry.to_string_lossy();
                        if !self.is_excluded(&path_str) {
                            index.get(&entry);
                            count += 1;
                        }
                    }
                }
            }

            index.save()?;
            Ok(count)
        } else {
            Ok(0)
        }
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

    /// Get total token count using proper tokenization
    pub fn token_count(&self) -> usize {
        self.files
            .iter()
            .map(|f| count_tokens(&f.content))
            .sum()
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
    fn test_extract_keywords() {
        let keywords =
            SmartContextSelector::extract_keywords("Find all functions related to session persistence");
        assert!(keywords.contains(&"session".to_string()));
        assert!(keywords.contains(&"persistence".to_string()));
        assert!(!keywords.contains(&"find".to_string()));
        assert!(!keywords.contains(&"all".to_string()));
    }

    #[test]
    fn test_extract_keywords_code_terms() {
        let keywords =
            SmartContextSelector::extract_keywords("implement the agent_loop with tool_router");
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
