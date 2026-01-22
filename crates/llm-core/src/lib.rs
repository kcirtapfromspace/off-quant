//! llm-core: Shared library for local LLM management
//!
//! Provides:
//! - Configuration loading (llm.toml)
//! - Ollama API client
//! - Tailscale integration
//! - Process management

pub mod config;
pub mod ollama;
pub mod tailscale;
pub mod process;

pub use config::Config;
pub use ollama::{OllamaClient, OllamaStatus, Model};
pub use tailscale::{TailscaleClient, TailscaleStatus};
