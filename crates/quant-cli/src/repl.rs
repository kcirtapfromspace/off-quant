//! Interactive REPL for chat interface
//!
//! Provides a Claude Code-like experience with:
//! - Readline-like input with history
//! - Streaming responses
//! - Slash commands for in-session control
//! - Conversation save/load
//! - Agent mode with tool execution

use anyhow::Result;
// crossterm is available for future terminal features
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use llm_core::{ChatMessage, Config, OllamaClient};
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{DefaultEditor, Editor};
use std::io::{stdout, Write};
use std::path::PathBuf;

use crate::agent::{AgentConfig, AgentLoop};
use crate::config::UserConfig;
use crate::context::ContextManager;
use crate::conversation::{Conversation, ConversationStore, InputHistory};
use crate::tools::builtin::create_default_registry;
use crate::tools::router::ToolRouter;
use crate::tools::security::TerminalConfirmation;

// ANSI colors
const GREEN: &str = "\x1b[92m";
const BLUE: &str = "\x1b[94m";
const YELLOW: &str = "\x1b[93m";
const CYAN: &str = "\x1b[96m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

/// REPL state
#[allow(dead_code)]
struct ReplState {
    /// Ollama client
    client: OllamaClient,
    /// Configuration
    config: Config,
    /// Current model
    model: String,
    /// Current conversation
    conversation: Conversation,
    /// Context manager
    context: ContextManager,
    /// Conversation store
    store: ConversationStore,
    /// Whether to auto-save
    auto_save: bool,
    /// Whether agent mode is enabled
    agent_mode: bool,
}

impl ReplState {
    async fn new(model: Option<String>, system: Option<String>) -> Result<Self> {
        // Try to load config, fall back to defaults if missing
        let (config, config_warning) = match Config::try_load() {
            Some(cfg) => (cfg, None),
            None => {
                let warning = format!(
                    "{}Warning:{} llm.toml not found, using defaults (localhost:11434)",
                    YELLOW, RESET
                );
                (Config::default_minimal(), Some(warning))
            }
        };

        let user_config = UserConfig::load().unwrap_or_default();
        let client = OllamaClient::new(config.ollama_url());

        // Check Ollama is running
        if !client.health_check().await.unwrap_or(false) {
            anyhow::bail!(
                "Ollama is not running.\nStart with: {}quant serve start{}",
                BLUE,
                RESET
            );
        }

        // Print config warning if applicable
        if let Some(warning) = config_warning {
            eprintln!("{}", warning);
        }

        // Determine model: CLI arg > user config > llm.toml > first available
        let model = if let Some(m) = model {
            m
        } else if let Some(m) = user_config.repl.default_model.clone() {
            m
        } else if !config.models.chat.is_empty() {
            config.models.chat.clone()
        } else {
            // No config available, try to get first available model from Ollama
            match client.list_models().await {
                Ok(models) if !models.is_empty() => {
                    let first_model = models[0].name.clone();
                    eprintln!(
                        "{}Info:{} No default model configured, using: {}",
                        DIM, RESET, first_model
                    );
                    first_model
                }
                _ => {
                    anyhow::bail!(
                        "No models available. Pull a model with: {}quant models pull <name>{}",
                        BLUE, RESET
                    );
                }
            }
        };

        // Use system prompt from: CLI arg > user config
        let system = system.or_else(|| user_config.repl.system_prompt.clone());

        let conversation = Conversation::new(model.clone(), system);
        let context = ContextManager::new()?;
        let store = ConversationStore::new()?;

        Ok(Self {
            client,
            config,
            model,
            conversation,
            context,
            store,
            auto_save: user_config.repl.auto_save,
            agent_mode: false,
        })
    }

    async fn load_conversation(&mut self, name: &str) -> Result<()> {
        self.conversation = self.store.load_by_name(name)?;
        self.model = self.conversation.model.clone();
        println!(
            "{}Loaded:{} {} ({} messages)",
            GREEN,
            RESET,
            self.conversation.title,
            self.conversation.len()
        );
        Ok(())
    }
}

/// Run the interactive REPL
pub async fn run(
    model: Option<String>,
    system: Option<String>,
    load: Option<String>,
) -> Result<()> {
    let mut state = ReplState::new(model, system).await?;

    // Load existing conversation if specified
    if let Some(name) = load {
        state.load_conversation(&name).await?;
    }

    // Setup readline
    let history = InputHistory::new()?;
    let mut rl: Editor<(), DefaultHistory> = DefaultEditor::new()?;
    let _ = rl.load_history(history.path());

    // Print welcome message
    print_welcome(&state);

    // Main REPL loop
    loop {
        let prompt = format!("{}quant>{} ", CYAN, RESET);

        match rl.readline(&prompt) {
            Ok(line) => {
                let line = line.trim();

                if line.is_empty() {
                    continue;
                }

                // Add to history
                let _ = rl.add_history_entry(line);

                // Handle slash commands
                if line.starts_with('/') {
                    match handle_slash_command(&mut state, line).await {
                        Ok(true) => break, // Exit requested
                        Ok(false) => continue,
                        Err(e) => {
                            eprintln!("{}Error:{} {}", YELLOW, RESET, e);
                            continue;
                        }
                    }
                }

                // Send message
                if let Err(e) = send_message(&mut state, line).await {
                    eprintln!("{}Error:{} {}", YELLOW, RESET, e);
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("{}^C{}", DIM, RESET);
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("{}Goodbye!{}", DIM, RESET);
                break;
            }
            Err(e) => {
                eprintln!("{}Error:{} {}", YELLOW, RESET, e);
                break;
            }
        }
    }

    // Save history
    let _ = rl.save_history(history.path());

    // Auto-save conversation if enabled and has messages
    if state.auto_save && !state.conversation.is_empty() {
        let path = state.store.save(&state.conversation)?;
        println!(
            "{}Saved:{} {}",
            DIM,
            RESET,
            path.file_name().unwrap().to_string_lossy()
        );
    }

    Ok(())
}

fn print_welcome(state: &ReplState) {
    println!();
    println!("{}╭─────────────────────────────────────────╮{}", DIM, RESET);
    println!(
        "{}│{} {}quant{} - Local LLM Chat                  {}│{}",
        DIM, RESET, BOLD, RESET, DIM, RESET
    );
    println!(
        "{}│{} Model: {}{}{}                      {}│{}",
        DIM,
        RESET,
        BLUE,
        truncate(&state.model, 25),
        RESET,
        DIM,
        RESET
    );
    println!(
        "{}│{} Type {}/help{} for commands                  {}│{}",
        DIM, RESET, CYAN, RESET, DIM, RESET
    );
    println!("{}╰─────────────────────────────────────────╯{}", DIM, RESET);
    println!();
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        format!("{:width$}", s, width = max)
    } else {
        format!("{}...", &s[..max - 3])
    }
}

/// Handle slash commands
async fn handle_slash_command(state: &mut ReplState, input: &str) -> Result<bool> {
    let parts: Vec<&str> = input.splitn(2, ' ').collect();
    let cmd = parts[0].to_lowercase();
    let args = parts.get(1).copied().unwrap_or("");

    match cmd.as_str() {
        "/help" | "/h" | "/?" => {
            print_help();
            Ok(false)
        }
        "/exit" | "/quit" | "/q" => {
            println!("{}Goodbye!{}", DIM, RESET);
            Ok(true)
        }
        "/model" | "/m" => {
            handle_model_command(state, args).await?;
            Ok(false)
        }
        "/models" => {
            handle_models_list(state).await?;
            Ok(false)
        }
        "/context" | "/ctx" => {
            handle_context_command(state, args)?;
            Ok(false)
        }
        "/clear" => {
            state.conversation.clear();
            println!("{}Conversation cleared{}", DIM, RESET);
            Ok(false)
        }
        "/save" => {
            let path = state.store.save(&state.conversation)?;
            println!(
                "{}Saved:{} {}",
                GREEN,
                RESET,
                path.file_name().unwrap().to_string_lossy()
            );
            Ok(false)
        }
        "/load" => {
            if args.is_empty() {
                // List conversations
                let convs = state.store.list()?;
                if convs.is_empty() {
                    println!("No saved conversations");
                } else {
                    println!("{}Saved Conversations:{}", BOLD, RESET);
                    for c in convs.iter().take(10) {
                        println!(
                            "  {} - {} ({} messages)",
                            &c.id[..8],
                            c.title,
                            c.message_count
                        );
                    }
                    println!("\nUse: /load <id-prefix>");
                }
            } else {
                state.load_conversation(args).await?;
            }
            Ok(false)
        }
        "/system" | "/sys" => {
            if args.is_empty() {
                if let Some(ref sys) = state.conversation.system_prompt {
                    println!("{}System prompt:{}", DIM, RESET);
                    println!("{}", sys);
                } else {
                    println!("No system prompt set");
                }
            } else {
                state.conversation.system_prompt = Some(args.to_string());
                println!("{}System prompt updated{}", DIM, RESET);
            }
            Ok(false)
        }
        "/history" | "/hist" => {
            if state.conversation.is_empty() {
                println!("No messages in conversation");
            } else {
                println!("{}Conversation History:{}", BOLD, RESET);
                for (i, msg) in state.conversation.messages.iter().enumerate() {
                    let role_color = match msg.role {
                        llm_core::Role::User => CYAN,
                        llm_core::Role::Assistant => GREEN,
                        llm_core::Role::System => YELLOW,
                        llm_core::Role::Tool => BLUE,
                    };
                    let preview = if msg.content.len() > 60 {
                        format!("{}...", &msg.content[..60])
                    } else {
                        msg.content.clone()
                    };
                    println!(
                        "  {}[{}]{} {}{:?}:{} {}",
                        DIM,
                        i + 1,
                        RESET,
                        role_color,
                        msg.role,
                        RESET,
                        preview.replace('\n', " ")
                    );
                }
            }
            Ok(false)
        }
        "/status" => {
            crate::commands::status().await?;
            Ok(false)
        }
        "/autosave" => {
            state.auto_save = !state.auto_save;
            println!(
                "Auto-save: {}",
                if state.auto_save { "enabled" } else { "disabled" }
            );
            Ok(false)
        }
        "/agent" => {
            state.agent_mode = !state.agent_mode;
            if state.agent_mode {
                println!(
                    "{}Agent mode: enabled{} (tools active)",
                    GREEN, RESET
                );
                println!("Messages will be processed with tool calling.");
            } else {
                println!(
                    "{}Agent mode: disabled{}",
                    YELLOW, RESET
                );
            }
            Ok(false)
        }
        _ => {
            println!("{}Unknown command:{} {}", YELLOW, RESET, cmd);
            println!("Type {}/help{} for available commands", CYAN, RESET);
            Ok(false)
        }
    }
}

fn print_help() {
    println!();
    println!("{}Commands:{}", BOLD, RESET);
    println!("  {}/help{}, /h, /?      Show this help", CYAN, RESET);
    println!("  {}/model{} <name>     Switch to a different model", CYAN, RESET);
    println!("  {}/models{}           List available models", CYAN, RESET);
    println!(
        "  {}/context{} <cmd>    Manage context files (add/list/rm/clear)",
        CYAN, RESET
    );
    println!("  {}/system{} <prompt>  Set system prompt", CYAN, RESET);
    println!("  {}/clear{}            Clear conversation history", CYAN, RESET);
    println!("  {}/save{}             Save conversation", CYAN, RESET);
    println!("  {}/load{} [id]        Load conversation (or list saved)", CYAN, RESET);
    println!("  {}/history{}          Show conversation history", CYAN, RESET);
    println!("  {}/status{}           Show Ollama status", CYAN, RESET);
    println!("  {}/autosave{}         Toggle auto-save on exit", CYAN, RESET);
    println!("  {}/agent{}            Toggle agent mode (tool execution)", CYAN, RESET);
    println!("  {}/exit{}, /quit, /q  Exit the REPL", CYAN, RESET);
    println!();
    println!("{}Tips:{}", DIM, RESET);
    println!("  - Press Ctrl+C to cancel current input");
    println!("  - Press Ctrl+D to exit");
    println!("  - Use arrow keys to navigate history");
    println!("  - Use /agent to enable tool calling");
    println!();
}

async fn handle_model_command(state: &mut ReplState, args: &str) -> Result<()> {
    if args.is_empty() {
        println!("Current model: {}{}{}", BLUE, state.model, RESET);
        println!("Usage: /model <model-name>");
        return Ok(());
    }

    // Check if model exists
    let models = state.client.list_models().await?;
    let model_names: Vec<_> = models.iter().map(|m| &m.name).collect();

    if !model_names.contains(&&args.to_string()) {
        println!(
            "{}Warning:{} Model '{}' not found locally",
            YELLOW, RESET, args
        );
        println!("Available models:");
        for name in model_names {
            println!("  - {}", name);
        }
        return Ok(());
    }

    // Check if model is already loaded
    let running = state.client.list_running().await.unwrap_or_default();
    let already_loaded = running.iter().any(|m| m.name == args);

    state.model = args.to_string();
    state.conversation.model = args.to_string();

    if already_loaded {
        println!("Switched to model: {}{}{}", BLUE, args, RESET);
    } else {
        // Warm up the model to avoid latency on first message
        let spinner = ProgressBar::new_spinner();
        spinner.set_style(
            ProgressStyle::default_spinner()
                .template("{spinner:.cyan} {msg}")
                .unwrap(),
        );
        spinner.set_message(format!("Loading {}...", args));
        spinner.enable_steady_tick(std::time::Duration::from_millis(100));

        match state.client.load_model(args).await {
            Ok(()) => {
                spinner.finish_and_clear();
                println!("{}✓{} Switched to model: {}{}{}", GREEN, RESET, BLUE, args, RESET);
            }
            Err(e) => {
                spinner.finish_and_clear();
                println!(
                    "{}Warning:{} Model set but failed to pre-load: {}",
                    YELLOW, RESET, e
                );
                println!("First message may be slow while model loads.");
            }
        }
    }

    Ok(())
}

async fn handle_models_list(state: &mut ReplState) -> Result<()> {
    let models = state.client.list_models().await?;

    println!("{}Available Models:{}", BOLD, RESET);
    for m in models {
        let current = if m.name == state.model { " (current)" } else { "" };
        println!("  {}{}{}{}", m.name, DIM, current, RESET);
    }

    Ok(())
}

fn handle_context_command(state: &mut ReplState, args: &str) -> Result<()> {
    let parts: Vec<&str> = args.splitn(2, ' ').collect();
    let subcmd = parts.first().copied().unwrap_or("");
    let subargs = parts.get(1).copied().unwrap_or("");

    match subcmd {
        "" | "list" => {
            let files = state.context.list();
            if files.is_empty() {
                println!("No files in context");
                println!("Add with: /context add <path>");
            } else {
                println!("{}Context Files:{}", BOLD, RESET);
                for f in files {
                    println!("  {}", f);
                }
            }
        }
        "add" => {
            if subargs.is_empty() {
                println!("Usage: /context add <path>");
            } else {
                state.context.add(subargs)?;
                state.context.save()?;
                println!("Added: {}", subargs);
            }
        }
        "rm" | "remove" => {
            if subargs.is_empty() {
                println!("Usage: /context rm <path>");
            } else {
                state.context.remove(subargs)?;
                state.context.save()?;
                println!("Removed: {}", subargs);
            }
        }
        "clear" => {
            state.context.clear();
            state.context.save()?;
            println!("Context cleared");
        }
        _ => {
            println!("{}Unknown context command:{} {}", YELLOW, RESET, subcmd);
            println!("Usage: /context [add|list|rm|clear] [path]");
        }
    }

    Ok(())
}

/// Send a message and stream the response
async fn send_message(state: &mut ReplState, input: &str) -> Result<()> {
    // Check if agent mode is enabled
    if state.agent_mode {
        return send_message_agent(state, input).await;
    }

    // Build the user message with context
    let mut full_message = String::new();

    // Add context if available
    let context_content = state.context.build_context()?;
    if !context_content.is_empty() {
        full_message.push_str(&context_content);
        full_message.push_str("\n---\n\n");
    }

    full_message.push_str(input);

    // Add to conversation
    state
        .conversation
        .add_message(ChatMessage::user(full_message.clone()));

    // Get messages for API
    let messages = state.conversation.messages_with_system();

    // Show thinking indicator
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message("Thinking...");
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    // Start timing
    let start_time = std::time::Instant::now();

    // Start streaming
    let mut stream = state
        .client
        .chat_stream(&state.model, &messages, None)
        .await?;

    // Clear spinner and start output
    spinner.finish_and_clear();
    print!("{}", GREEN);
    stdout().flush()?;

    let mut response_content = String::new();
    let mut first_token_time: Option<std::time::Duration> = None;
    let mut token_count = 0u32;
    let mut eval_duration: Option<u64> = None;

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(msg) = &chunk.message {
            // Track time to first token
            if first_token_time.is_none() && !msg.content.is_empty() {
                first_token_time = Some(start_time.elapsed());
            }
            print!("{}", msg.content);
            stdout().flush()?;
            response_content.push_str(&msg.content);
        }
        // Capture final stats from the done message
        if chunk.done {
            if let Some(count) = chunk.eval_count {
                token_count = count;
            }
            if let Some(duration) = chunk.eval_duration {
                eval_duration = Some(duration);
            }
        }
    }

    let total_time = start_time.elapsed();

    print!("{}", RESET);
    println!();

    // Show timing metrics (subtle, dimmed)
    let ttft = first_token_time
        .map(|d| format!("{:.1}s", d.as_secs_f64()))
        .unwrap_or_else(|| "?".to_string());
    let tokens_per_sec = if let Some(eval_ns) = eval_duration {
        if eval_ns > 0 && token_count > 0 {
            let tps = token_count as f64 / (eval_ns as f64 / 1_000_000_000.0);
            format!("{:.1} tok/s", tps)
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    tracing::debug!(
        model = %state.model,
        tokens = token_count,
        ttft_ms = first_token_time.map(|d| d.as_millis() as u64).unwrap_or(0),
        total_ms = total_time.as_millis() as u64,
        "chat_complete"
    );

    // Only show metrics if we have meaningful data
    if token_count > 0 {
        println!(
            "{}[{} tokens | TTFT: {} | {}]{}\n",
            DIM, token_count, ttft, tokens_per_sec, RESET
        );
    } else {
        println!();
    }

    // Add assistant response to conversation
    state
        .conversation
        .add_message(ChatMessage::assistant(response_content));

    Ok(())
}

/// Send a message in agent mode with tool execution
async fn send_message_agent(state: &mut ReplState, input: &str) -> Result<()> {
    // Build the user message with context
    let mut full_message = String::new();

    // Add context if available
    let context_content = state.context.build_context()?;
    if !context_content.is_empty() {
        full_message.push_str(&context_content);
        full_message.push_str("\n---\n\n");
    }

    full_message.push_str(input);

    // Create tool registry and router
    let registry = create_default_registry();
    let confirmation = TerminalConfirmation::new();
    let router = ToolRouter::new(registry, confirmation);

    // Configure the agent
    let agent_config = AgentConfig::new(&state.model)
        .with_max_iterations(50)
        .with_working_dir(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .with_auto_mode(false)
        .with_verbose(true);

    // Add system prompt if set
    let agent_config = if let Some(ref sys) = state.conversation.system_prompt {
        agent_config.with_system_prompt(sys.clone())
    } else {
        agent_config
    };

    // Create and run the agent
    let agent = AgentLoop::new(state.client.clone(), router, agent_config);
    let agent_state = agent.run(&full_message).await?;

    // Add user message to conversation history
    state
        .conversation
        .add_message(ChatMessage::user(full_message));

    // Add final response to conversation history
    if let Some(ref response) = agent_state.final_response {
        println!();
        println!("{}Response:{}", GREEN, RESET);
        println!("{}", response);
        state
            .conversation
            .add_message(ChatMessage::assistant(response.clone()));
    }

    if let Some(ref error) = agent_state.error {
        println!();
        println!("{}Error:{} {}", YELLOW, RESET, error);
    }

    println!();
    println!(
        "{}[Agent completed in {} iterations]{}",
        DIM, agent_state.iteration, RESET
    );

    Ok(())
}
