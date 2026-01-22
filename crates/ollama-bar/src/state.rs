//! Application state management

use llm_core::{Config, OllamaClient, OllamaStatus, TailscaleClient, TailscaleStatus};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    inner: Arc<Mutex<AppStateInner>>,
}

struct AppStateInner {
    config: Config,
    ollama_client: OllamaClient,
    tailscale_client: TailscaleClient,

    // Cached status
    ollama_status: OllamaStatus,
    tailscale_status: TailscaleStatus,
    current_model: Option<String>,
    available_models: Vec<String>,
    memory_used_gb: f64,
    memory_total_gb: f64,

    // Settings
    tailscale_sharing: bool,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let config = Config::load()?;
        let ollama_client = OllamaClient::new(config.ollama_url());
        let tailscale_client = TailscaleClient::new();

        let memory_total_gb = Config::system_ram_gb()? as f64;

        Ok(Self {
            inner: Arc::new(Mutex::new(AppStateInner {
                config,
                ollama_client,
                tailscale_client,
                ollama_status: OllamaStatus::Stopped,
                tailscale_status: TailscaleStatus::Disconnected,
                current_model: None,
                available_models: Vec::new(),
                memory_used_gb: 0.0,
                memory_total_gb,
                tailscale_sharing: false,
            })),
        })
    }

    /// Refresh all status information
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let (ollama_client, tailscale_client) = {
            let inner = self.inner.lock().unwrap();
            (inner.ollama_client.clone(), inner.tailscale_client.clone())
        };

        // Check Ollama status
        let ollama_status = ollama_client.status().await;
        let (current_model, available_models, memory_used) = if ollama_status == OllamaStatus::Running {
            let models = ollama_client.list_models().await.unwrap_or_default();
            let running = ollama_client.list_running().await.unwrap_or_default();

            let current = running.first().map(|m| m.name.clone());
            let names: Vec<String> = models.iter().map(|m| m.name.clone()).collect();
            let mem = running.first().map(|m| m.size as f64 / 1e9).unwrap_or(0.0);

            (current, names, mem)
        } else {
            (None, Vec::new(), 0.0)
        };

        // Check Tailscale status
        let tailscale_status = tailscale_client.status();

        // Update state
        {
            let mut inner = self.inner.lock().unwrap();
            inner.ollama_status = ollama_status;
            inner.tailscale_status = tailscale_status;
            inner.current_model = current_model;
            inner.available_models = available_models;
            inner.memory_used_gb = memory_used;
        }

        Ok(())
    }

    // Getters

    pub fn ollama_status(&self) -> OllamaStatus {
        self.inner.lock().unwrap().ollama_status
    }

    pub fn tailscale_status(&self) -> TailscaleStatus {
        self.inner.lock().unwrap().tailscale_status
    }

    pub fn current_model(&self) -> Option<String> {
        self.inner.lock().unwrap().current_model.clone()
    }

    pub fn available_models(&self) -> Vec<String> {
        self.inner.lock().unwrap().available_models.clone()
    }

    pub fn memory_info(&self) -> (f64, f64) {
        let inner = self.inner.lock().unwrap();
        (inner.memory_used_gb, inner.memory_total_gb)
    }

    pub fn tailscale_sharing(&self) -> bool {
        self.inner.lock().unwrap().tailscale_sharing
    }

    pub fn tailscale_ip(&self) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        if inner.tailscale_status == TailscaleStatus::Connected {
            inner.tailscale_client.get_ipv4().ok()
        } else {
            None
        }
    }

    pub fn ollama_url(&self) -> String {
        self.inner.lock().unwrap().config.ollama_url()
    }

    // Actions

    pub async fn start_ollama(&self) -> anyhow::Result<()> {
        use std::process::Command;

        let (host, port, ollama_home) = {
            let inner = self.inner.lock().unwrap();
            (
                inner.config.ollama.host.clone(),
                inner.config.ollama.port,
                inner.config.ollama.ollama_home.to_string_lossy().to_string(),
            )
        };

        tracing::info!("Starting Ollama at {}:{}", host, port);

        Command::new("ollama")
            .arg("serve")
            .env("OLLAMA_HOST", format!("{}:{}", host, port))
            .env("OLLAMA_HOME", &ollama_home)
            .spawn()?;

        // Wait for health
        let client = self.inner.lock().unwrap().ollama_client.clone();
        for _ in 0..30 {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if client.health_check().await.unwrap_or(false) {
                tracing::info!("Ollama started successfully");
                return Ok(());
            }
        }

        anyhow::bail!("Ollama failed to start within 30 seconds")
    }

    pub fn stop_ollama(&self) -> anyhow::Result<()> {
        use std::process::Command;

        tracing::info!("Stopping Ollama");

        // Find and kill ollama process
        let output = Command::new("pkill")
            .args(["-f", "ollama serve"])
            .output()?;

        if output.status.success() {
            tracing::info!("Ollama stopped");
        }

        Ok(())
    }

    pub async fn switch_model(&self, model: &str) -> anyhow::Result<()> {
        let client = self.inner.lock().unwrap().ollama_client.clone();

        tracing::info!("Loading model: {}", model);
        client.load_model(model).await?;
        tracing::info!("Model loaded: {}", model);

        Ok(())
    }

    pub fn toggle_tailscale_sharing(&self) -> anyhow::Result<()> {
        let mut inner = self.inner.lock().unwrap();

        if inner.tailscale_status != TailscaleStatus::Connected {
            anyhow::bail!("Tailscale is not connected");
        }

        inner.tailscale_sharing = !inner.tailscale_sharing;

        // TODO: Restart Ollama with different OLLAMA_HOST binding
        // For now, just toggle the flag

        tracing::info!("Tailscale sharing: {}", inner.tailscale_sharing);
        Ok(())
    }
}
