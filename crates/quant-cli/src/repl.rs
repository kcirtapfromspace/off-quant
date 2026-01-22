//! Interactive REPL for chat interface
//!
//! Provides a Claude Code-like experience with:
//! - Readline-like input with history
//! - Streaming responses
//! - Slash commands for in-session control
//! - Conversation save/load

use anyhow::{Context, Result};
// crossterm is available for future terminal features
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use llm_core::{ChatMessage, Config, OllamaClient};
use rustyline::error::ReadlineError;
use rustyline::history::DefaultHistory;
use rustyline::{DefaultEditor, Editor};
use std::io::{stdout, Write};

use crate::context::ContextManager;
use crate::conversation::{Conversation, ConversationStore, InputHistory};

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
}

impl ReplState {
    async fn new(model: Option<String>, system: Option<String>) -> Result<Self> {
        let config = Config::load().context("Failed to load llm.toml")?;
        let client = OllamaClient::new(config.ollama_url());

        // Check Ollama is running
        if !client.health_check().await.unwrap_or(false) {
            anyhow::bail!(
                "Ollama is not running.\nStart with: {}quant serve start{}",
                BLUE,
                RESET
            );
        }

        let model = model.unwrap_or_else(|| config.models.chat.clone());
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
            auto_save: false,
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
    println!("  {}/exit{}, /quit, /q  Exit the REPL", CYAN, RESET);
    println!();
    println!("{}Tips:{}", DIM, RESET);
    println!("  - Press Ctrl+C to cancel current input");
    println!("  - Press Ctrl+D to exit");
    println!("  - Use arrow keys to navigate history");
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

    state.model = args.to_string();
    state.conversation.model = args.to_string();
    println!("Switched to model: {}{}{}", BLUE, args, RESET);

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

    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        if let Some(msg) = &chunk.message {
            print!("{}", msg.content);
            stdout().flush()?;
            response_content.push_str(&msg.content);
        }
    }

    print!("{}", RESET);
    println!();
    println!();

    // Add assistant response to conversation
    state
        .conversation
        .add_message(ChatMessage::assistant(response_content));

    Ok(())
}
