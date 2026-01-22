//! quant: Unified CLI for local LLM management
//!
//! Provides a Claude Code-like experience for local LLMs via Ollama.

mod agent;
mod commands;
mod config;
mod context;
mod conversation;
mod hooks;
mod mcp;
mod progress;
mod project;
mod repl;
mod session;
mod tools;

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

        /// Temperature (0.0-2.0, default: 0.7)
        #[arg(short, long)]
        temperature: Option<f32>,

        /// Max tokens to generate
        #[arg(long)]
        max_tokens: Option<i32>,

        /// Don't print newline after response
        #[arg(short = 'n', long)]
        no_newline: bool,
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

    /// Load/warm up a model (makes subsequent queries faster)
    Run {
        /// Model to load
        #[arg(short, long)]
        model: Option<String>,
    },

    /// Show detailed version and system info
    Info,

    /// Manage user configuration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },

    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        #[arg(value_enum)]
        shell: clap_complete::Shell,
    },

    /// Run agent with tools for autonomous task execution
    Agent {
        /// The task to perform
        task: Vec<String>,

        /// Model to use
        #[arg(short, long)]
        model: Option<String>,

        /// System prompt
        #[arg(short, long)]
        system: Option<String>,

        /// Auto-approve all tool executions (skip confirmations)
        #[arg(long)]
        auto: bool,

        /// Maximum iterations before stopping
        #[arg(long, default_value = "50")]
        max_iterations: usize,

        /// Quiet mode (less verbose output)
        #[arg(short, long)]
        quiet: bool,

        /// Resume a previous session
        #[arg(long)]
        resume: Option<String>,

        /// Don't save this session
        #[arg(long)]
        no_save: bool,
    },

    /// Manage conversation sessions
    Sessions {
        #[command(subcommand)]
        action: SessionAction,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigAction {
    /// Create default config file
    Init,
    /// Show current configuration
    Show,
    /// Print config file path
    Path,
    /// Edit config file (opens in $EDITOR)
    Edit,
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

#[derive(Debug, Subcommand)]
enum SessionAction {
    /// List saved sessions
    List {
        /// Show only sessions for current project
        #[arg(long)]
        project: bool,

        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Show details of a session
    Show {
        /// Session ID
        id: String,
    },
    /// Delete a session
    Rm {
        /// Session ID
        id: String,
    },
    /// Resume a session (alias for `agent --resume`)
    Resume {
        /// Session ID (or "latest" for most recent)
        id: String,

        /// Auto-approve all tool executions
        #[arg(long)]
        auto: bool,
    },
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
            temperature,
            max_tokens,
            no_newline,
        }) => {
            let prompt_text = prompt.join(" ");
            commands::ask(
                &prompt_text,
                model,
                stdin,
                context,
                json,
                system,
                temperature,
                max_tokens,
                no_newline,
            )
            .await
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
        Some(Commands::Run { model }) => commands::run(model).await,
        Some(Commands::Info) => commands::info().await,
        Some(Commands::Config { action }) => match action {
            ConfigAction::Init => commands::config_init().await,
            ConfigAction::Show => commands::config_show().await,
            ConfigAction::Path => commands::config_path().await,
            ConfigAction::Edit => commands::config_edit().await,
        },
        Some(Commands::Completions { shell }) => {
            use clap::CommandFactory;
            use clap_complete::generate;
            let mut cmd = Cli::command();
            let name = cmd.get_name().to_string();
            generate(shell, &mut cmd, name, &mut std::io::stdout());
            Ok(())
        }
        Some(Commands::Agent {
            task,
            model,
            system,
            auto,
            max_iterations,
            quiet,
            resume,
            no_save,
        }) => {
            let task_text = task.join(" ");
            commands::agent(&task_text, model, system, auto, max_iterations, quiet, resume, no_save).await
        }
        Some(Commands::Sessions { action }) => match action {
            SessionAction::List { project, json } => commands::sessions_list(project, json).await,
            SessionAction::Show { id } => commands::sessions_show(&id).await,
            SessionAction::Rm { id } => commands::sessions_rm(&id).await,
            SessionAction::Resume { id, auto } => commands::sessions_resume(&id, auto).await,
        }
        None => {
            // Default to chat REPL when no command specified
            repl::run(None, None, None).await
        }
    }
}
