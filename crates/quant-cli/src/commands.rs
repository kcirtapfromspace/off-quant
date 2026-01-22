//! CLI commands implementation
//!
//! Ports the Python llm_ctl.py functionality to Rust.

use anyhow::{Context, Result};
use futures::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use llm_core::{ChatMessage, Config, OllamaClient, OllamaStatus};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use crate::agent::{AgentConfig, AgentLoop};
use crate::context::ContextManager;
use crate::tools::builtin::create_default_registry;
use crate::tools::router::ToolRouter;
use crate::tools::security::TerminalConfirmation;

// ANSI color codes
const GREEN: &str = "\x1b[92m";
const RED: &str = "\x1b[91m";
const YELLOW: &str = "\x1b[93m";
const BLUE: &str = "\x1b[94m";
const CYAN: &str = "\x1b[96m";
const BOLD: &str = "\x1b[1m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

fn print_status(ok: bool, msg: &str) {
    let icon = if ok {
        format!("{}✓{}", GREEN, RESET)
    } else {
        format!("{}✗{}", RED, RESET)
    };
    println!("  {} {}", icon, msg);
}

/// Show Ollama status and system info
pub async fn status() -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    println!("{}Ollama Status{}", BOLD, RESET);
    println!("  Endpoint: {}", config.ollama_url());

    let status = client.status().await;
    match status {
        OllamaStatus::Running => {
            print_status(true, "Ollama is running");
        }
        _ => {
            print_status(false, "Ollama is not running");
            println!("\n  Start with: {}ollama serve{}", BLUE, RESET);
            println!("  Or: {}quant serve start{}", BLUE, RESET);
            return Ok(());
        }
    }

    // Check models volume
    let vol_ok = config.ollama.models_path.exists();
    print_status(
        vol_ok,
        &format!("Models volume: {}", config.ollama.models_path.display()),
    );

    // List models
    match client.list_models().await {
        Ok(models) => {
            println!("\n{}Loaded Models ({}){}", BOLD, models.len(), RESET);
            if models.is_empty() {
                println!("  {}No models loaded{}", YELLOW, RESET);
                println!("  Run: {}quant import{}", BLUE, RESET);
            } else {
                let mut sorted = models.clone();
                sorted.sort_by(|a, b| a.name.cmp(&b.name));
                for m in sorted {
                    println!("  - {} ({})", m.name, m.size_human());
                }
            }
        }
        Err(e) => {
            println!("  {}Error listing models: {}{}", RED, e, RESET);
        }
    }

    // Show running models
    match client.list_running().await {
        Ok(running) if !running.is_empty() => {
            println!("\n{}Running Models{}", BOLD, RESET);
            for m in running {
                let vram_gb = m.size_vram as f64 / (1024.0 * 1024.0 * 1024.0);
                println!("  - {} (VRAM: {:.1} GB)", m.name, vram_gb);
            }
        }
        _ => {}
    }

    // System info
    println!("\n{}System{}", BOLD, RESET);
    match Config::system_ram_gb() {
        Ok(ram) => println!("  RAM: {} GB", ram),
        Err(_) => println!("  RAM: unknown"),
    }
    println!("  Arch: {}", std::env::consts::ARCH);

    Ok(())
}

/// Health check with retries
pub async fn health(timeout_secs: u64) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    let pb = ProgressBar::new(timeout_secs);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} Waiting for Ollama [{bar:30.cyan/dim}] {pos}/{len}s")
            .unwrap()
            .progress_chars("=>-"),
    );

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let interval = Duration::from_secs(1);

    while start.elapsed() < timeout {
        pb.set_position(start.elapsed().as_secs());

        if client.health_check().await.unwrap_or(false) {
            pb.finish_and_clear();
            println!("{}✓{} Ollama is ready", GREEN, RESET);
            return Ok(());
        }
        tokio::time::sleep(interval).await;
    }

    pb.finish_and_clear();
    println!("{}✗{} Ollama did not become ready within {}s", RED, RESET, timeout_secs);
    anyhow::bail!("Ollama did not become ready within timeout")
}

/// List available models
pub async fn models_list() -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    // Show local GGUF files
    println!("{}Local GGUF Files{}", BOLD, RESET);
    for (_, model) in &config.models.local {
        let path = config.ollama.models_path.join(&model.file);
        let exists = path.exists();
        let status = if exists {
            format!("{}exists{}", GREEN, RESET)
        } else {
            format!("{}missing{}", RED, RESET)
        };
        println!("  {}: {}", model.name, status);
    }

    // Check if Ollama is running
    if !client.health_check().await.unwrap_or(false) {
        println!(
            "\n{}Ollama not running - can't list imported models{}",
            YELLOW, RESET
        );
        return Ok(());
    }

    // Show imported models
    println!("\n{}Imported in Ollama{}", BOLD, RESET);
    let models = client.list_models().await?;
    let local_names: std::collections::HashSet<_> =
        config.models.local.values().map(|m| &m.name).collect();

    let mut sorted = models.clone();
    sorted.sort_by(|a, b| a.name.cmp(&b.name));

    for m in sorted {
        let tag = if local_names.contains(&m.name) {
            format!(" {}(local){}", BLUE, RESET)
        } else {
            String::new()
        };
        println!("  - {}{}", m.name, tag);
    }

    Ok(())
}

/// Pull a model from Ollama registry
pub async fn models_pull(name: &str) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    // Check Ollama is running
    if !client.health_check().await.unwrap_or(false) {
        anyhow::bail!("Ollama is not running. Start with: quant serve start");
    }

    println!("Pulling {}...", name);

    // Use streaming pull API for progress
    let mut stream = client
        .pull_model_stream(name)
        .await
        .context("Failed to start model pull")?;

    let pb = ProgressBar::new(100);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.cyan} {msg} [{bar:30.cyan/dim}] {percent}%")
            .unwrap()
            .progress_chars("=>-"),
    );
    pb.set_message(name.to_string());

    let mut last_status = String::new();

    while let Some(progress) = stream.next().await {
        let progress = progress?;

        // Update status message if changed
        if progress.status != last_status {
            last_status = progress.status.clone();
            pb.set_message(format!("{}: {}", name, progress.status));
        }

        // Update progress bar if we have total/completed info
        if progress.total > 0 {
            let percent = (progress.completed as f64 / progress.total as f64 * 100.0) as u64;
            pb.set_position(percent);
        }
    }

    pb.finish_and_clear();
    println!("{}✓{} Pulled {}", GREEN, RESET, name);

    Ok(())
}

/// Remove a model
pub async fn models_rm(name: &str) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    println!("Removing {}...", name);
    client.delete_model(name).await?;
    println!("{}Done!{}", GREEN, RESET);

    Ok(())
}

/// Show running/loaded models
pub async fn models_ps() -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    let running = client.list_running().await?;

    if running.is_empty() {
        println!("No models currently loaded");
        return Ok(());
    }

    println!("{}Running Models{}", BOLD, RESET);
    for m in running {
        let vram_gb = m.size_vram as f64 / (1024.0 * 1024.0 * 1024.0);
        println!("  {} ({:.1} GB VRAM, expires: {})", m.name, vram_gb, m.expires_at);
    }

    Ok(())
}

/// Start Ollama server
pub async fn serve_start(foreground: bool) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;

    // Check if already running
    let client = OllamaClient::new(config.ollama_url());
    if client.health_check().await.unwrap_or(false) {
        println!("Ollama is already running");
        return Ok(());
    }

    println!("Starting Ollama...");
    println!("  OLLAMA_HOME={}", config.ollama.ollama_home.display());
    println!(
        "  OLLAMA_HOST={}:{}",
        config.ollama.host, config.ollama.port
    );

    let mut cmd = Command::new("ollama");
    cmd.arg("serve")
        .env(
            "OLLAMA_HOST",
            format!("{}:{}", config.ollama.host, config.ollama.port),
        )
        .env("OLLAMA_HOME", &config.ollama.ollama_home);

    if foreground {
        // Run in foreground
        let status = cmd.status().context("Failed to start Ollama")?;
        if !status.success() {
            anyhow::bail!("Ollama exited with error");
        }
    } else {
        // Run in background
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .context("Failed to start Ollama")?;

        // Wait for it to be ready
        tokio::time::sleep(Duration::from_secs(2)).await;

        if client.health_check().await.unwrap_or(false) {
            println!("{}Ollama started successfully{}", GREEN, RESET);
        } else {
            println!(
                "{}Ollama started but not yet responding - check logs{}",
                YELLOW, RESET
            );
        }
    }

    Ok(())
}

/// Stop Ollama server
pub async fn serve_stop() -> Result<()> {
    // Try to find and kill ollama process
    #[cfg(unix)]
    {
        let output = Command::new("pkill")
            .arg("-f")
            .arg("ollama serve")
            .output()
            .context("Failed to run pkill")?;

        if output.status.success() {
            println!("{}Ollama stopped{}", GREEN, RESET);
        } else {
            println!("Ollama was not running");
        }
    }

    #[cfg(not(unix))]
    {
        anyhow::bail!("serve stop is only supported on Unix systems");
    }

    Ok(())
}

/// Restart Ollama server
pub async fn serve_restart() -> Result<()> {
    serve_stop().await?;
    tokio::time::sleep(Duration::from_secs(1)).await;
    serve_start(false).await
}

/// Import local GGUF files into Ollama
pub async fn import() -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    if !client.health_check().await.unwrap_or(false) {
        println!("{}Ollama is not running{}", RED, RESET);
        return Ok(());
    }

    if !config.ollama.models_path.exists() {
        println!(
            "{}Models volume not mounted: {}{}",
            RED,
            config.ollama.models_path.display(),
            RESET
        );
        return Ok(());
    }

    let existing: std::collections::HashSet<_> = client
        .list_models()
        .await?
        .into_iter()
        .map(|m| m.name)
        .collect();

    let mut imported = 0;

    for (_, model) in &config.models.local {
        let name = &model.name;
        let gguf_path = config.ollama.models_path.join(&model.file);
        let modelfile_path = Path::new(&model.modelfile);

        if existing.contains(name) {
            println!("  {}skip{} {} (already exists)", YELLOW, RESET, name);
            continue;
        }

        if !gguf_path.exists() {
            println!(
                "  {}skip{} {} (GGUF not found: {})",
                RED,
                RESET,
                name,
                gguf_path.display()
            );
            continue;
        }

        if !modelfile_path.exists() {
            println!(
                "  {}skip{} {} (Modelfile not found: {})",
                RED,
                RESET,
                name,
                modelfile_path.display()
            );
            continue;
        }

        print!("  {}importing{} {}...", BLUE, RESET, name);
        io::stdout().flush()?;

        let result = Command::new("ollama")
            .arg("create")
            .arg(name)
            .arg("-f")
            .arg(modelfile_path)
            .output();

        match result {
            Ok(output) if output.status.success() => {
                println!(" {}OK{}", GREEN, RESET);
                imported += 1;
            }
            Ok(output) => {
                println!(" {}FAILED{}", RED, RESET);
                let stderr = String::from_utf8_lossy(&output.stderr);
                println!("    {}", stderr.trim());
            }
            Err(e) => {
                println!(" {}FAILED{}", RED, RESET);
                println!("    {}", e);
            }
        }
    }

    println!("\nImported {} model(s)", imported);
    Ok(())
}

/// Auto-select best model based on system RAM
pub async fn select(json: bool) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let ram = Config::system_ram_gb()?;

    let model = config.auto_select_model()?;

    if json {
        let output = serde_json::json!({
            "ram_gb": ram,
            "model": model
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("RAM: {} GB", ram);
        println!("Selected: {}", model);
    }

    Ok(())
}

/// Generate .env.local for Aider
pub async fn env(output_path: &str) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let ram = Config::system_ram_gb().unwrap_or(0);
    let model = config.auto_select_model().unwrap_or_else(|_| config.models.coding.clone());

    let lines = vec![
        format!("OLLAMA_MODEL={}", model),
        format!("AIDER_MODEL=ollama/{}", model),
        format!("OLLAMA_API_BASE={}", config.ollama_url()),
        "AIDER_AUTO_COMMITS=1".to_string(),
        "AIDER_LOG_FILE=.aider/aider.log".to_string(),
        format!("HOST_RAM_GB={}", ram),
        format!("HOST_ARCH={}", std::env::consts::ARCH),
    ];

    std::fs::write(output_path, lines.join("\n") + "\n")?;
    println!("Wrote: {}", output_path);
    println!("Model: {}", model);

    Ok(())
}

/// One-shot query
pub async fn ask(
    prompt: &str,
    model: Option<String>,
    stdin: bool,
    context_path: Option<String>,
    json_output: bool,
    system: Option<String>,
    temperature: Option<f32>,
    max_tokens: Option<i32>,
    no_newline: bool,
) -> Result<()> {
    use llm_core::ChatOptions;

    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    // Check Ollama is running
    if !client.health_check().await.unwrap_or(false) {
        anyhow::bail!("Ollama is not running. Start with: quant serve start");
    }

    // Select model
    let model = model.unwrap_or_else(|| config.models.coding.clone());

    // Build prompt
    let mut full_prompt = String::new();

    // Add context if provided
    if let Some(ctx_path) = context_path {
        let ctx_manager = ContextManager::new()?;
        let ctx_content = ctx_manager.build_context_from_path(&ctx_path)?;
        if !ctx_content.is_empty() {
            full_prompt.push_str(&ctx_content);
            full_prompt.push_str("\n\n");
        }
    }

    // Add stdin content if requested
    if stdin {
        let mut stdin_content = String::new();
        io::stdin().read_to_string(&mut stdin_content)?;
        if !stdin_content.is_empty() {
            full_prompt.push_str("```\n");
            full_prompt.push_str(&stdin_content);
            full_prompt.push_str("\n```\n\n");
        }
    }

    // Add the actual prompt
    full_prompt.push_str(prompt);

    // Build messages
    let mut messages = Vec::new();
    if let Some(sys) = system {
        messages.push(ChatMessage::system(sys));
    }
    messages.push(ChatMessage::user(full_prompt));

    // Build options
    let options = if temperature.is_some() || max_tokens.is_some() {
        Some(ChatOptions {
            temperature,
            num_predict: max_tokens,
            ..Default::default()
        })
    } else {
        None
    };

    if json_output {
        // Non-streaming for JSON output (with timeout)
        let response = tokio::time::timeout(
            Duration::from_secs(300),
            client.chat(&model, &messages, options),
        )
        .await
        .context("Request timed out after 5 minutes")??;

        let output = serde_json::json!({
            "model": response.model,
            "response": response.message.content,
            "eval_count": response.eval_count,
            "eval_duration_ms": response.eval_duration / 1_000_000,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Streaming output (with timeout on initial connection)
        let mut stream = tokio::time::timeout(
            Duration::from_secs(60),
            client.chat_stream(&model, &messages, options),
        )
        .await
        .context("Connection timed out after 60 seconds")??;

        let stream_timeout = Duration::from_secs(120); // 2 min between chunks
        while let Ok(Some(chunk)) =
            tokio::time::timeout(stream_timeout, stream.next()).await
        {
            let chunk = chunk?;
            if let Some(msg) = &chunk.message {
                print!("{}", msg.content);
                io::stdout().flush()?;
            }
        }
        if !no_newline {
            println!();
        }
    }

    Ok(())
}

// Context management commands

/// Add files/directories to context
pub async fn context_add(paths: &[String]) -> Result<()> {
    let mut ctx_manager = ContextManager::new()?;

    for path in paths {
        ctx_manager.add(path)?;
        println!("Added: {}", path);
    }

    ctx_manager.save()?;
    Ok(())
}

/// List current context files
pub async fn context_list() -> Result<()> {
    let ctx_manager = ContextManager::new()?;
    let files = ctx_manager.list();

    if files.is_empty() {
        println!("No files in context");
        println!("Add files with: quant context add <path>");
        return Ok(());
    }

    println!("{}Context Files{}", BOLD, RESET);
    for file in &files {
        println!("  {}", file);
    }

    // Show token usage
    if let Ok((tokens, max_tokens, is_truncated)) = ctx_manager.token_status() {
        println!();
        let usage_pct = (tokens as f64 / max_tokens as f64 * 100.0) as u32;
        let status = if is_truncated {
            format!("{}(truncated){}", RED, RESET)
        } else if usage_pct > 80 {
            format!("{}(approaching limit){}", YELLOW, RESET)
        } else {
            String::new()
        };
        println!(
            "{}Tokens:{} ~{} / {} ({}%) {}",
            BOLD, RESET, tokens, max_tokens, usage_pct, status
        );
    }

    Ok(())
}

/// Remove files from context
pub async fn context_rm(paths: &[String]) -> Result<()> {
    let mut ctx_manager = ContextManager::new()?;

    for path in paths {
        ctx_manager.remove(path)?;
        println!("Removed: {}", path);
    }

    ctx_manager.save()?;
    Ok(())
}

/// Clear all context
pub async fn context_clear() -> Result<()> {
    let mut ctx_manager = ContextManager::new()?;
    ctx_manager.clear();
    ctx_manager.save()?;
    println!("Context cleared");
    Ok(())
}

/// Load/warm up a model
pub async fn run(model: Option<String>) -> Result<()> {
    let config = Config::load().context("Failed to load llm.toml")?;
    let client = OllamaClient::new(config.ollama_url());

    // Check Ollama is running
    if !client.health_check().await.unwrap_or(false) {
        anyhow::bail!("Ollama is not running. Start with: quant serve start");
    }

    // Select model
    let model = model.unwrap_or_else(|| config.models.coding.clone());

    // Check if already loaded
    let running = client.list_running().await.unwrap_or_default();
    if running.iter().any(|m| m.name == model) {
        println!("Model {} is already loaded", model);
        return Ok(());
    }

    // Show loading spinner
    let spinner = ProgressBar::new_spinner();
    spinner.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.cyan} {msg}")
            .unwrap(),
    );
    spinner.set_message(format!("Loading {}...", model));
    spinner.enable_steady_tick(std::time::Duration::from_millis(100));

    // Load the model by sending a minimal request
    client.load_model(&model).await?;

    spinner.finish_with_message(format!("{}✓{} Model {} loaded", GREEN, RESET, model));

    // Show VRAM usage
    if let Ok(running) = client.list_running().await {
        for m in running {
            if m.name == model {
                let vram_gb = m.size_vram as f64 / (1024.0 * 1024.0 * 1024.0);
                println!("  VRAM: {:.1} GB", vram_gb);
            }
        }
    }

    Ok(())
}

/// Show detailed version and system info
pub async fn info() -> Result<()> {
    let config = Config::load().ok();

    println!("{}quant{} - Unified CLI for local LLM management", BOLD, RESET);
    println!("Version: {}", env!("CARGO_PKG_VERSION"));
    println!();

    // System info
    println!("{}System{}", BOLD, RESET);
    match Config::system_ram_gb() {
        Ok(ram) => println!("  RAM: {} GB", ram),
        Err(_) => println!("  RAM: unknown"),
    }
    println!("  Arch: {}", std::env::consts::ARCH);
    println!("  OS: {}", std::env::consts::OS);
    println!();

    // Config info
    if let Some(cfg) = config {
        println!("{}Configuration{}", BOLD, RESET);
        println!("  Ollama: {}", cfg.ollama_url());
        println!("  Models path: {}", cfg.ollama.models_path.display());
        println!("  Default coding model: {}", cfg.models.coding);
        println!("  Default chat model: {}", cfg.models.chat);
    } else {
        println!("{}Configuration{}", BOLD, RESET);
        println!("  {}llm.toml not found{}", YELLOW, RESET);
    }
    println!();

    // Data directories
    println!("{}Data Directories{}", BOLD, RESET);
    if let Some(data_dir) = dirs::data_dir() {
        let quant_dir = data_dir.join("quant");
        println!("  Data: {}", quant_dir.display());
        println!("  Conversations: {}", quant_dir.join("conversations").display());
        println!("  History: {}", quant_dir.join("history").display());
    }

    Ok(())
}

// Config management commands

/// Create default config file
pub async fn config_init() -> Result<()> {
    use crate::config::UserConfig;

    match UserConfig::create_default() {
        Ok(path) => {
            println!("{}Created:{} {}", GREEN, RESET, path.display());
            println!("\nEdit this file to customize your quant experience.");
        }
        Err(e) => {
            if e.to_string().contains("already exists") {
                let path = UserConfig::config_path()?;
                println!("Config file already exists: {}", path.display());
                println!("Use 'quant config edit' to modify it.");
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

/// Show current configuration
pub async fn config_show() -> Result<()> {
    use crate::config::UserConfig;

    let path = UserConfig::config_path()?;

    if !path.exists() {
        println!("No config file found at: {}", path.display());
        println!("Run 'quant config init' to create one.");
        return Ok(());
    }

    let config = UserConfig::load()?;

    println!("{}User Configuration{}", BOLD, RESET);
    println!("  Path: {}", path.display());
    println!();

    println!("{}[repl]{}", BLUE, RESET);
    if let Some(ref model) = config.repl.default_model {
        println!("  default_model = \"{}\"", model);
    }
    if let Some(ref sys) = config.repl.system_prompt {
        println!("  system_prompt = \"{}\"", sys);
    }
    println!("  auto_save = {}", config.repl.auto_save);
    println!("  history_size = {}", config.repl.history_size);
    println!("  theme = \"{}\"", config.repl.theme);
    println!();

    println!("{}[ask]{}", BLUE, RESET);
    if let Some(ref model) = config.ask.default_model {
        println!("  default_model = \"{}\"", model);
    }
    if let Some(temp) = config.ask.temperature {
        println!("  temperature = {}", temp);
    }
    if let Some(max) = config.ask.max_tokens {
        println!("  max_tokens = {}", max);
    }
    println!();

    if !config.aliases.models.is_empty() {
        println!("{}[aliases.models]{}", BLUE, RESET);
        for (alias, model) in &config.aliases.models {
            println!("  {} = \"{}\"", alias, model);
        }
    }

    Ok(())
}

/// Print config file path
pub async fn config_path() -> Result<()> {
    use crate::config::UserConfig;

    let path = UserConfig::config_path()?;
    println!("{}", path.display());

    Ok(())
}

/// Edit config file
pub async fn config_edit() -> Result<()> {
    use crate::config::UserConfig;

    let path = UserConfig::config_path()?;

    // Create default if doesn't exist
    if !path.exists() {
        UserConfig::create_default()?;
        println!("Created default config at: {}", path.display());
    }

    // Get editor from environment
    let editor = std::env::var("EDITOR")
        .or_else(|_| std::env::var("VISUAL"))
        .unwrap_or_else(|_| {
            if cfg!(target_os = "macos") {
                "open -e".to_string()
            } else {
                "nano".to_string()
            }
        });

    // Open editor
    let parts: Vec<&str> = editor.split_whitespace().collect();
    let (cmd, args) = parts.split_first().context("Invalid editor command")?;

    let mut command = Command::new(cmd);
    command.args(args.iter());
    command.arg(&path);

    let status = command.status().context("Failed to open editor")?;

    if !status.success() {
        anyhow::bail!("Editor exited with error");
    }

    Ok(())
}

/// Run agent with autonomous task execution
pub async fn agent(
    task: &str,
    model: Option<String>,
    system: Option<String>,
    auto: bool,
    max_iterations: usize,
    quiet: bool,
    resume: Option<String>,
    no_save: bool,
) -> Result<()> {
    use crate::session::{Session, SessionStore};

    // Load config, fall back to defaults
    let (config, _) = match Config::try_load() {
        Some(cfg) => (cfg, None),
        None => (Config::default_minimal(), Some("Using default config")),
    };

    let client = OllamaClient::new(config.ollama_url());

    // Check Ollama is running
    if !client.health_check().await.unwrap_or(false) {
        anyhow::bail!(
            "Ollama is not running.\nStart with: {}quant serve start{}",
            BLUE,
            RESET
        );
    }

    // Determine model
    let model = model.unwrap_or_else(|| {
        if !config.models.coding.is_empty() {
            config.models.coding.clone()
        } else {
            "llama3.2".to_string()
        }
    });

    // Handle session resume
    let session_store = SessionStore::new()?;
    let mut session = if let Some(ref session_id) = resume {
        if !quiet {
            println!("{}Resuming session:{} {}", DIM, RESET, session_id);
        }
        session_store.load(session_id)?
    } else {
        let working_dir = std::env::current_dir().ok();
        Session::new(&model, working_dir)
    };

    // Create tool registry and router
    let registry = create_default_registry();
    let confirmation = if auto {
        TerminalConfirmation::auto()
    } else {
        TerminalConfirmation::new()
    };
    let router = ToolRouter::new(registry, confirmation);

    // Configure the agent
    let agent_config = AgentConfig::new(&model)
        .with_max_iterations(max_iterations)
        .with_working_dir(std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .with_auto_mode(auto)
        .with_verbose(!quiet);

    let agent_config = if let Some(sys) = system {
        agent_config.with_system_prompt(sys)
    } else {
        agent_config
    };

    // Create and run the agent
    let agent = AgentLoop::new(client, router, agent_config);

    if !quiet {
        println!("{}Agent Mode{}", BOLD, RESET);
        println!("  Model: {}", model);
        println!("  Task: {}", task);
        println!("  Auto mode: {}", if auto { "yes" } else { "no" });
        if resume.is_some() {
            println!("  Session: {}", session.id);
        }
        println!();
    }

    let state = agent.run(task).await?;

    // Save session messages
    for msg in &state.messages {
        session.add_message(msg.clone());
    }

    // Generate a summary from the final response
    if let Some(ref response) = state.final_response {
        let summary = if response.len() > 100 {
            format!("{}...", &response[..97])
        } else {
            response.clone()
        };
        session.set_summary(summary);
    }

    // Save session (unless --no-save)
    if !no_save {
        session_store.save(&session)?;
        if !quiet {
            println!("{}Session saved:{} {}", DIM, RESET, session.id);
        }
    }

    // Print results
    if let Some(response) = state.final_response {
        println!();
        println!("{}Final Response:{}", BOLD, RESET);
        println!("{}", response);
    }

    if let Some(error) = state.error {
        println!();
        println!("{}Error:{} {}", RED, RESET, error);
    }

    if !quiet {
        println!();
        println!(
            "{}Completed in {} iterations{}",
            GREEN, state.iteration, RESET
        );
    }

    Ok(())
}

/// List saved sessions
pub async fn sessions_list(project_only: bool, json: bool) -> Result<()> {
    use crate::session::SessionStore;

    let store = SessionStore::new()?;

    let sessions = if project_only {
        let cwd = std::env::current_dir()?;
        store.find_by_project(&cwd)?
    } else {
        store.list()?
    };

    if json {
        println!("{}", serde_json::to_string_pretty(&sessions)?);
        return Ok(());
    }

    if sessions.is_empty() {
        println!("No saved sessions found.");
        if project_only {
            println!("{}Tip:{} Use `quant sessions list` to see all sessions.", DIM, RESET);
        }
        return Ok(());
    }

    println!("{}Saved Sessions:{}", BOLD, RESET);
    println!();

    for s in sessions {
        let project = s.project_root
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| "-".to_string());

        println!(
            "  {}{}{}  {} msgs  {}  {}",
            CYAN, s.id, RESET,
            s.message_count,
            s.model,
            project
        );
        if let Some(summary) = &s.summary {
            let truncated = if summary.len() > 60 {
                format!("{}...", &summary[..57])
            } else {
                summary.clone()
            };
            println!("    {}{}{}", DIM, truncated, RESET);
        }
    }

    println!();
    println!("{}Resume with:{} quant sessions resume <id>", DIM, RESET);

    Ok(())
}

/// Show details of a session
pub async fn sessions_show(id: &str) -> Result<()> {
    use crate::session::SessionStore;

    let store = SessionStore::new()?;
    let session = store.load(id)?;

    println!("{}Session:{} {}", BOLD, RESET, session.id);
    println!("  Name: {}", session.name);
    println!("  Model: {}", session.model);
    println!("  Created: {}", session.created_at.format("%Y-%m-%d %H:%M:%S"));
    println!("  Updated: {}", session.updated_at.format("%Y-%m-%d %H:%M:%S"));
    if let Some(ref root) = session.project_root {
        println!("  Project: {}", root.display());
    }
    println!("  Messages: {}", session.message_count());

    if let Some(ref summary) = session.summary {
        println!();
        println!("{}Summary:{}", BOLD, RESET);
        println!("  {}", summary);
    }

    println!();
    println!("{}Messages:{}", BOLD, RESET);
    for (i, msg) in session.messages.iter().enumerate() {
        let role = format!("{:?}", msg.role).to_lowercase();
        let content = if msg.content.len() > 100 {
            format!("{}...", &msg.content[..97])
        } else {
            msg.content.clone()
        };
        println!("  {}. [{}] {}", i + 1, role, content);
    }

    Ok(())
}

/// Delete a session
pub async fn sessions_rm(id: &str) -> Result<()> {
    use crate::session::SessionStore;

    let store = SessionStore::new()?;
    store.delete(id)?;
    println!("{}Deleted session:{} {}", GREEN, RESET, id);
    Ok(())
}

/// Resume a session
pub async fn sessions_resume(id: &str, auto: bool) -> Result<()> {
    use crate::session::SessionStore;

    let store = SessionStore::new()?;

    // Handle "latest" as alias for most recent session
    let session_id = if id == "latest" {
        let sessions = store.list()?;
        sessions.first()
            .map(|s| s.id.clone())
            .ok_or_else(|| anyhow::anyhow!("No sessions found"))?
    } else {
        id.to_string()
    };

    let session = store.load(&session_id)?;

    println!("{}Resuming session:{} {}", BOLD, RESET, session.id);
    println!("  Model: {}", session.model);
    println!("  Messages: {}", session.message_count());
    println!();
    println!("Enter your next task or question:");

    // Read task from stdin
    let mut task = String::new();
    std::io::stdin().read_line(&mut task)?;
    let task = task.trim();

    if task.is_empty() {
        println!("{}No task provided, exiting.{}", YELLOW, RESET);
        return Ok(());
    }

    // Run agent with resumed session
    agent(
        task,
        Some(session.model.clone()),
        None,
        auto,
        50,
        false,
        Some(session_id),
        false,
    ).await
}
