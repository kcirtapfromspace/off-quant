//! Proper tokenization using tiktoken
//!
//! Replaces the rough "4 chars per token" estimate with actual tokenization.

use once_cell::sync::Lazy;
use parking_lot::Mutex;
use tiktoken_rs::{cl100k_base, CoreBPE};

/// Default fallback estimate when tokenizer unavailable
const FALLBACK_CHARS_PER_TOKEN: usize = 4;

/// Global tokenizer (lazy initialized)
static CL100K_TOKENIZER: Lazy<Mutex<Option<CoreBPE>>> = Lazy::new(|| {
    Mutex::new(cl100k_base().ok())
});

/// Tokenizer type for different models
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TokenizerType {
    /// GPT-4, GPT-3.5-turbo, Claude (uses cl100k_base)
    Cl100kBase,
    /// Fallback for unknown models
    Fallback,
}

impl TokenizerType {
    /// Determine tokenizer type from model name
    pub fn from_model_name(model: &str) -> Self {
        let model_lower = model.to_lowercase();

        // Models that use cl100k_base (OpenAI GPT-4, GPT-3.5)
        if model_lower.contains("gpt-4")
            || model_lower.contains("gpt-3.5")
            || model_lower.contains("claude")
            || model_lower.contains("text-embedding")
        {
            return Self::Cl100kBase;
        }

        // For local models (Llama, Mistral, etc.), use cl100k as approximation
        // This is close enough for context management purposes
        if model_lower.contains("llama")
            || model_lower.contains("mistral")
            || model_lower.contains("qwen")
            || model_lower.contains("codellama")
            || model_lower.contains("deepseek")
            || model_lower.contains("phi")
        {
            return Self::Cl100kBase;
        }

        Self::Fallback
    }
}

/// Tokenizer for counting tokens in text
pub struct Tokenizer {
    tokenizer_type: TokenizerType,
}

impl Tokenizer {
    /// Create a new tokenizer for the given model
    pub fn new(model: &str) -> Self {
        Self {
            tokenizer_type: TokenizerType::from_model_name(model),
        }
    }

    /// Create a tokenizer with a specific type
    pub fn with_type(tokenizer_type: TokenizerType) -> Self {
        Self { tokenizer_type }
    }

    /// Count tokens in the given text
    pub fn count_tokens(&self, text: &str) -> usize {
        match self.tokenizer_type {
            TokenizerType::Cl100kBase => {
                let guard = CL100K_TOKENIZER.lock();
                if let Some(ref bpe) = *guard {
                    bpe.encode_with_special_tokens(text).len()
                } else {
                    // Fallback if tokenizer creation fails
                    text.len() / FALLBACK_CHARS_PER_TOKEN
                }
            }
            TokenizerType::Fallback => {
                text.len() / FALLBACK_CHARS_PER_TOKEN
            }
        }
    }

    /// Truncate text to fit within a token limit
    pub fn truncate_to_tokens(&self, text: &str, max_tokens: usize) -> String {
        match self.tokenizer_type {
            TokenizerType::Cl100kBase => {
                let guard = CL100K_TOKENIZER.lock();
                if let Some(ref bpe) = *guard {
                    let tokens = bpe.encode_with_special_tokens(text);
                    if tokens.len() <= max_tokens {
                        return text.to_string();
                    }

                    // Decode truncated tokens
                    let truncated_tokens = &tokens[..max_tokens];
                    match bpe.decode(truncated_tokens.to_vec()) {
                        Ok(decoded) => decoded,
                        Err(_) => {
                            // Fallback to character-based truncation
                            let char_limit = max_tokens * FALLBACK_CHARS_PER_TOKEN;
                            text.chars().take(char_limit).collect()
                        }
                    }
                } else {
                    // Fallback
                    let char_limit = max_tokens * FALLBACK_CHARS_PER_TOKEN;
                    text.chars().take(char_limit).collect()
                }
            }
            TokenizerType::Fallback => {
                let char_limit = max_tokens * FALLBACK_CHARS_PER_TOKEN;
                text.chars().take(char_limit).collect()
            }
        }
    }

    /// Get estimated tokens per character for this tokenizer
    pub fn avg_chars_per_token(&self) -> f32 {
        match self.tokenizer_type {
            TokenizerType::Cl100kBase => 4.0, // Rough average for English text
            TokenizerType::Fallback => FALLBACK_CHARS_PER_TOKEN as f32,
        }
    }
}

impl Default for Tokenizer {
    fn default() -> Self {
        Self::with_type(TokenizerType::Cl100kBase)
    }
}

/// Count tokens in text using the default tokenizer
pub fn count_tokens(text: &str) -> usize {
    Tokenizer::default().count_tokens(text)
}

/// Truncate text to fit within a token limit using the default tokenizer
pub fn truncate_to_tokens(text: &str, max_tokens: usize) -> String {
    Tokenizer::default().truncate_to_tokens(text, max_tokens)
}

/// Count tokens for a specific model
pub fn count_tokens_for_model(text: &str, model: &str) -> usize {
    Tokenizer::new(model).count_tokens(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tokenizer_type_detection() {
        assert_eq!(
            TokenizerType::from_model_name("gpt-4"),
            TokenizerType::Cl100kBase
        );
        assert_eq!(
            TokenizerType::from_model_name("gpt-3.5-turbo"),
            TokenizerType::Cl100kBase
        );
        assert_eq!(
            TokenizerType::from_model_name("claude-3"),
            TokenizerType::Cl100kBase
        );
        assert_eq!(
            TokenizerType::from_model_name("llama3.2"),
            TokenizerType::Cl100kBase
        );
        assert_eq!(
            TokenizerType::from_model_name("unknown-model"),
            TokenizerType::Fallback
        );
    }

    #[test]
    fn test_count_tokens() {
        let tokenizer = Tokenizer::default();
        let text = "Hello, world! This is a test.";
        let count = tokenizer.count_tokens(text);

        // Should be reasonable (7-10 tokens for this text)
        assert!(count > 0);
        assert!(count < 20);
    }

    #[test]
    fn test_truncate_to_tokens() {
        let tokenizer = Tokenizer::default();
        let text = "This is a long text that should be truncated to fit within the token limit.";

        // Truncate to 5 tokens
        let truncated = tokenizer.truncate_to_tokens(text, 5);
        let truncated_count = tokenizer.count_tokens(&truncated);

        // Should be at most 5 tokens
        assert!(truncated_count <= 5);
    }

    #[test]
    fn test_fallback_tokenizer() {
        let tokenizer = Tokenizer::with_type(TokenizerType::Fallback);
        let text = "Hello world"; // 11 chars

        let count = tokenizer.count_tokens(text);
        // Should be 11 / 4 = 2 (integer division)
        assert_eq!(count, 2);
    }

    #[test]
    fn test_global_functions() {
        let text = "Test text";
        let count = count_tokens(text);
        assert!(count > 0);

        let truncated = truncate_to_tokens(text, 2);
        assert!(count_tokens(&truncated) <= 2);
    }
}
