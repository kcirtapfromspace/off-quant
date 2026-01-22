//! quant: Unified CLI for local LLM management
//!
//! Provides a Claude Code-like experience for local LLMs via Ollama.

mod commands;
mod context;
mod conversation;
mod repl;

use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Parser)]
#[command(name = "quant")]
#[command(about = "Unified CLI for local LLM management", version)]
#[command(propagate_version = true)]
struct Cli {
    /// Enable verbose output
    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Start interactive chat REPL
    Chat {
        /// Model to use (overrides config)
        #[arg(short, long)]
        model: Option<String>,

        /// System prompt
        #[arg(short, long)]
        system: Option<String>,

        /// Load a saved conversation
        #[arg(long)]
        load: Option<String>,
    },

    /// One-shot query (non-interactive)
    Ask {
        /// The prompt to send
        prompt: Vec<String>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,

        /// Read input from stdin
        #[arg(long)]
        stdin: bool,

        /// Add context from directory
        #[arg(short, long)]
        context: Option<String>,

        /// Output as JSON
        #[arg(long)]
        json: bool,

        /// System prompt
        #[arg(short, long)]
        system: Option<String>,
    },

    /// Show Ollama status and system info
    Status,

    /// Manage models
    Models {
        #[command(subcommand)]
        action: ModelAction,
    },

    /// Manage Ollama service
    Serve {
        #[command(subcommand)]
        action: ServeAction,
    },

    /// Manage context/files for RAG
    Context {
        #[command(subcommand)]
        action: ContextAction,
    },

    /// Health check with retries
    Health {
        /// Timeout in seconds
        #[arg(short, long, default_value = "60")]
        timeout: u64,
    },

    /// Import local GGUF files into Ollama
    Import,

    /// Auto-select best model based on system RAM
    Select {
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },

    /// Generate .env.local for Aider
    Env {
        /// Output file path
        #[arg(short, long, default_value = ".env.local")]
        output: String,
    },
}

#[derive(Debug, Subcommand)]
enum ModelAction {
    /// List available models
    List,
    /// Pull a model from Ollama registry
    Pull {
        /// Model name to pull
        name: String,
    },
    /// Remove a model
    Rm {
        /// Model name to remove
        name: String,
    },
    /// Show running/loaded models
    Ps,
}

#[derive(Debug, Subcommand)]
enum ServeAction {
    /// Start Ollama server
    Start {
        /// Run in foreground
        #[arg(long)]
        foreground: bool,
    },
    /// Stop Ollama server
    Stop,
    /// Restart Ollama server
    Restart,
}

#[derive(Debug, Subcommand)]
enum ContextAction {
    /// Add files/directories to context
    Add {
        /// Paths to add
        paths: Vec<String>,
    },
    /// List current context files
    List,
    /// Remove files from context
    Rm {
        /// Paths to remove
        paths: Vec<String>,
    },
    /// Clear all context
    Clear,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Setup logging
    let filter = if cli.verbose {
        EnvFilter::new("debug")
    } else {
        EnvFilter::new("warn")
    };
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match cli.command {
        Some(Commands::Chat { model, system, load }) => {
            repl::run(model, system, load).await
        }
        Some(Commands::Ask {
            prompt,
            model,
            stdin,
            context,
            json,
            system,
        }) => {
            let prompt_text = prompt.join(" ");
            commands::ask(&prompt_text, model, stdin, context, json, system).await
        }
        Some(Commands::Status) => commands::status().await,
        Some(Commands::Models { action }) => match action {
            ModelAction::List => commands::models_list().await,
            ModelAction::Pull { name } => commands::models_pull(&name).await,
            ModelAction::Rm { name } => commands::models_rm(&name).await,
            ModelAction::Ps => commands::models_ps().await,
        },
        Some(Commands::Serve { action }) => match action {
            ServeAction::Start { foreground } => commands::serve_start(foreground).await,
            ServeAction::Stop => commands::serve_stop().await,
            ServeAction::Restart => commands::serve_restart().await,
        },
        Some(Commands::Context { action }) => match action {
            ContextAction::Add { paths } => commands::context_add(&paths).await,
            ContextAction::List => commands::context_list().await,
            ContextAction::Rm { paths } => commands::context_rm(&paths).await,
            ContextAction::Clear => commands::context_clear().await,
        },
        Some(Commands::Health { timeout }) => commands::health(timeout).await,
        Some(Commands::Import) => commands::import().await,
        Some(Commands::Select { json }) => commands::select(json).await,
        Some(Commands::Env { output }) => commands::env(&output).await,
        None => {
            // Default to chat REPL when no command specified
            repl::run(None, None, None).await
        }
    }
}
