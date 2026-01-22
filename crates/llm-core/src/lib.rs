//! llm-core: Shared library for local LLM management
//!
//! Provides:
//! - Configuration loading (llm.toml)
//! - Ollama API client (with streaming support)
//! - Tailscale integration
//! - Process management

pub mod config;
pub mod ollama;
pub mod process;
pub mod tailscale;

pub use config::Config;
pub use ollama::{
    ChatChunk, ChatMessage, ChatOptions, ChatResponse, ChatStream, Model, OllamaClient,
    OllamaStatus, PullProgress, PullStream, RetryConfig, Role, RunningModel,
};
pub use tailscale::{TailscaleClient, TailscaleStatus};
