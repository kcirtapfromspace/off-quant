//! CLI commands implementation
//!
//! Ports the Python llm_ctl.py functionality to Rust.

use anyhow::{Context, Result};
use futures::StreamExt;
use llm_core::{ChatMessage, Config, OllamaClient, OllamaStatus};
use std::io::{self, Read, Write};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use crate::context::ContextManager;

// ANSI color codes
const GREEN: &str = "\x1b[92m";
const RED: &str = "\x1b[91m";
const YELLOW: &str = "\x1b[93m";
const BLUE: &str = "\x1b[94m";
const BOLD: &str = "\x1b[1m";
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

    print!("Waiting for Ollama (timeout: {}s)...", timeout_secs);
    io::stdout().flush()?;

    let start = std::time::Instant::now();
    let timeout = Duration::from_secs(timeout_secs);
    let interval = Duration::from_secs(2);

    while start.elapsed() < timeout {
        if client.health_check().await.unwrap_or(false) {
            println!(" {}OK{}", GREEN, RESET);
            return Ok(());
        }
        tokio::time::sleep(interval).await;
    }

    println!(" {}FAILED{}", RED, RESET);
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
    println!("Pulling {}...", name);

    // Use ollama CLI for progress display
    let status = Command::new("ollama")
        .arg("pull")
        .arg(name)
        .status()
        .context("Failed to run ollama pull")?;

    if status.success() {
        println!("{}Done!{}", GREEN, RESET);
        Ok(())
    } else {
        anyhow::bail!("Failed to pull model")
    }
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
) -> Result<()> {
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

    if json_output {
        // Non-streaming for JSON output
        let response = client.chat(&model, &messages, None).await?;
        let output = serde_json::json!({
            "model": response.model,
            "response": response.message.content,
            "eval_count": response.eval_count,
            "eval_duration_ms": response.eval_duration / 1_000_000,
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        // Streaming output
        let mut stream = client.chat_stream(&model, &messages, None).await?;

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if let Some(msg) = &chunk.message {
                print!("{}", msg.content);
                io::stdout().flush()?;
            }
        }
        println!();
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
    for file in files {
        println!("  {}", file);
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
