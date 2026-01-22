//! Context management for RAG (Retrieval-Augmented Generation)
//!
//! This module provides:
//! - **ContextManager**: Explicit file/directory management for prompts
//! - **SmartContextSelector**: Auto-selects relevant files based on query analysis
//! - **Tokenizer**: Accurate token counting using tiktoken
//! - **FileIndex**: Cached file metadata for efficient access
//! - **EmbeddingEngine**: Semantic search using embeddings (optional)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                  SmartContextSelector                    │
//! │  - Keyword extraction from query                        │
//! │  - Name-based file matching                             │
//! │  - Content-based file matching (grep)                   │
//! │  - Semantic matching (embeddings, optional)             │
//! │  - Ranking and token-aware truncation                   │
//! └─────────────┬───────────────┬───────────────────────────┘
//!               │               │
//!               ▼               ▼
//! ┌─────────────────┐ ┌─────────────────┐
//! │   FileIndex     │ │  Tokenizer      │
//! │  - Metadata     │ │  - tiktoken     │
//! │  - Caching      │ │  - Token count  │
//! │  - Hashing      │ │  - Truncation   │
//! └─────────────────┘ └─────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use quant_cli::context::{SmartContextSelector, SmartContext};
//!
//! let mut selector = SmartContextSelector::new(project_root)
//!     .with_max_tokens(8000)
//!     .with_model("gpt-4");
//!
//! let context = selector.select_context("implement authentication")?;
//!
//! if !context.is_empty() {
//!     println!("Selected {} files ({} tokens)",
//!         context.files.len(),
//!         context.token_count());
//! }
//! ```
//!
//! # Features
//!
//! - `embeddings`: Enables semantic search using fastembed

pub mod manager;
pub mod smart;
pub mod tokenizer;
pub mod index;

#[cfg(feature = "embeddings")]
pub mod embeddings;

// Re-exports
pub use manager::{ContextConfig, ContextManager, DEFAULT_MAX_TOKENS};
pub use smart::{SmartContext, SmartContextFile, SmartContextSelector};
pub use tokenizer::{count_tokens, count_tokens_for_model, truncate_to_tokens, Tokenizer, TokenizerType};
pub use index::{FileIndex, FileMetadata, IndexStats};

#[cfg(feature = "embeddings")]
pub use embeddings::{EmbeddingEngine, SemanticSearchResult};

/// Model-specific context limits
pub struct ModelLimits {
    /// Maximum context window tokens
    pub context_window: usize,
    /// Recommended tokens for system prompt
    pub system_reserve: usize,
    /// Recommended tokens for response
    pub response_reserve: usize,
}

impl ModelLimits {
    /// Get limits for a model by name
    pub fn for_model(model: &str) -> Self {
        let model_lower = model.to_lowercase();

        // GPT-4 variants
        if model_lower.contains("gpt-4-turbo") || model_lower.contains("gpt-4o") {
            return Self {
                context_window: 128000,
                system_reserve: 4000,
                response_reserve: 4000,
            };
        }

        if model_lower.contains("gpt-4") {
            return Self {
                context_window: 8192,
                system_reserve: 2000,
                response_reserve: 2000,
            };
        }

        // GPT-3.5
        if model_lower.contains("gpt-3.5") {
            return Self {
                context_window: 16384,
                system_reserve: 2000,
                response_reserve: 2000,
            };
        }

        // Claude models
        if model_lower.contains("claude-3-opus") || model_lower.contains("claude-3-sonnet") {
            return Self {
                context_window: 200000,
                system_reserve: 8000,
                response_reserve: 4000,
            };
        }

        if model_lower.contains("claude") {
            return Self {
                context_window: 100000,
                system_reserve: 4000,
                response_reserve: 4000,
            };
        }

        // Llama models (local via Ollama)
        if model_lower.contains("llama3") {
            return Self {
                context_window: 8192,
                system_reserve: 2000,
                response_reserve: 2000,
            };
        }

        if model_lower.contains("llama") {
            return Self {
                context_window: 4096,
                system_reserve: 1000,
                response_reserve: 1000,
            };
        }

        // Qwen models
        if model_lower.contains("qwen") {
            return Self {
                context_window: 32768,
                system_reserve: 4000,
                response_reserve: 4000,
            };
        }

        // Mistral models
        if model_lower.contains("mistral") {
            return Self {
                context_window: 32768,
                system_reserve: 4000,
                response_reserve: 4000,
            };
        }

        // Default conservative limits
        Self {
            context_window: 4096,
            system_reserve: 1000,
            response_reserve: 1000,
        }
    }

    /// Get available tokens for context (excluding reserves)
    pub fn available_for_context(&self) -> usize {
        self.context_window
            .saturating_sub(self.system_reserve)
            .saturating_sub(self.response_reserve)
    }
}

/// Adaptive context configuration
pub struct AdaptiveContext {
    /// Model-specific limits
    limits: ModelLimits,
    /// Current token usage
    used_tokens: usize,
}

impl AdaptiveContext {
    /// Create for a specific model
    pub fn for_model(model: &str) -> Self {
        Self {
            limits: ModelLimits::for_model(model),
            used_tokens: 0,
        }
    }

    /// Get remaining available tokens
    pub fn remaining(&self) -> usize {
        self.limits.available_for_context().saturating_sub(self.used_tokens)
    }

    /// Add tokens to usage
    pub fn add_usage(&mut self, tokens: usize) {
        self.used_tokens += tokens;
    }

    /// Check if we can fit more content
    pub fn can_fit(&self, tokens: usize) -> bool {
        self.remaining() >= tokens
    }

    /// Get usage percentage
    pub fn usage_percent(&self) -> f32 {
        let available = self.limits.available_for_context();
        if available == 0 {
            return 100.0;
        }
        (self.used_tokens as f32 / available as f32) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_model_limits() {
        let gpt4 = ModelLimits::for_model("gpt-4");
        assert_eq!(gpt4.context_window, 8192);

        let gpt4_turbo = ModelLimits::for_model("gpt-4-turbo");
        assert_eq!(gpt4_turbo.context_window, 128000);

        let claude = ModelLimits::for_model("claude-3-sonnet");
        assert_eq!(claude.context_window, 200000);

        let llama = ModelLimits::for_model("llama3.2");
        assert_eq!(llama.context_window, 8192);
    }

    #[test]
    fn test_adaptive_context() {
        let mut ctx = AdaptiveContext::for_model("gpt-4");

        assert!(ctx.can_fit(1000));
        ctx.add_usage(1000);
        assert!(ctx.remaining() < ctx.limits.available_for_context());
    }
}
