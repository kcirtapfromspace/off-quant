//! llm-core: Shared library for local LLM management
//!
//! Provides:
//! - Configuration loading (llm.toml)
//! - Ollama API client
//! - Tailscale integration
//! - Process management

pub mod config;
pub mod ollama;
pub mod process;
pub mod tailscale;

pub use config::Config;
pub use ollama::{Model, OllamaClient, OllamaStatus};
pub use tailscale::{TailscaleClient, TailscaleStatus};
