//! OllamaBar - macOS menu bar app for local LLM management
//!
//! A native menu bar application that provides:
//! - One-click Ollama start/stop
//! - Model switching
//! - Tailscale network sharing
//! - Memory monitoring

#![cfg(target_os = "macos")]

mod actions;
mod app;
mod dialogs;
mod menu;
mod notifications;
mod state;

use anyhow::Result;
use tracing_subscriber::{fmt, prelude::*, EnvFilter};

fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::registry()
        .with(fmt::layer())
        .with(EnvFilter::from_default_env().add_directive("ollama_bar=debug".parse()?))
        .init();

    tracing::info!("Starting OllamaBar");

    // Run the app
    app::run()
}
