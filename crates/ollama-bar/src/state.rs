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

    // Remember last used model
    last_model: Option<String>,
}

impl AppState {
    pub fn new() -> anyhow::Result<Self> {
        let config = Config::load()?;
        let ollama_client = OllamaClient::new(config.ollama_url());
        let tailscale_client = TailscaleClient::new();

        let memory_total_gb = Config::system_ram_gb()? as f64;

        // Load last model from persistent storage
        let last_model = Self::load_last_model();

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
                last_model,
            })),
        })
    }

    /// Load last model from persistent storage
    fn load_last_model() -> Option<String> {
        let path = dirs::cache_dir()?.join("ollama-bar").join("last_model");
        std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
    }

    /// Save last model to persistent storage
    fn save_last_model(model: &str) {
        if let Some(cache_dir) = dirs::cache_dir() {
            let dir = cache_dir.join("ollama-bar");
            let _ = std::fs::create_dir_all(&dir);
            let path = dir.join("last_model");
            let _ = std::fs::write(path, model);
        }
    }

    /// Refresh all status information
    pub async fn refresh(&self) -> anyhow::Result<()> {
        let (ollama_client, tailscale_client) = {
            let inner = self.inner.lock().unwrap();
            (inner.ollama_client.clone(), inner.tailscale_client.clone())
        };

        // Check Ollama status
        let ollama_status = ollama_client.status().await;
        let (current_model, available_models, memory_used) =
            if ollama_status == OllamaStatus::Running {
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

        // Check if tailscale serve is actually active
        let tailscale_sharing = self.is_tailscale_serving();

        // Update state
        {
            let mut inner = self.inner.lock().unwrap();
            inner.ollama_status = ollama_status;
            inner.tailscale_status = tailscale_status;

            // Track last model - save when current model changes
            if let Some(ref model) = current_model {
                if inner.current_model.as_ref() != Some(model) {
                    inner.last_model = Some(model.clone());
                    Self::save_last_model(model);
                }
            }

            inner.current_model = current_model;
            inner.available_models = available_models;
            inner.memory_used_gb = memory_used;
            inner.tailscale_sharing = tailscale_sharing;
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

    pub fn last_model(&self) -> Option<String> {
        self.inner.lock().unwrap().last_model.clone()
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

    #[allow(dead_code)]
    pub fn tailscale_ip(&self) -> Option<String> {
        let inner = self.inner.lock().unwrap();
        if inner.tailscale_status == TailscaleStatus::Connected {
            inner.tailscale_client.get_ipv4().ok()
        } else {
            None
        }
    }

    #[allow(dead_code)]
    pub fn ollama_url(&self) -> String {
        self.inner.lock().unwrap().config.ollama_url()
    }

    // Actions

    /// Get the path to the Ollama log file
    pub fn ollama_log_path() -> std::path::PathBuf {
        std::path::PathBuf::from("/tmp/ollama.log")
    }

    pub async fn start_ollama(&self) -> anyhow::Result<()> {
        use std::fs::File;
        use std::process::{Command, Stdio};

        let (host, port, ollama_home) = {
            let inner = self.inner.lock().unwrap();
            (
                inner.config.ollama.host.clone(),
                inner.config.ollama.port,
                inner
                    .config
                    .ollama
                    .ollama_home
                    .to_string_lossy()
                    .to_string(),
            )
        };

        tracing::info!("Starting Ollama at {}:{}", host, port);

        // Create log file for Ollama output
        let log_path = Self::ollama_log_path();
        let log_file = File::create(&log_path)?;
        let log_file_err = log_file.try_clone()?;

        tracing::info!("Ollama logs will be written to: {:?}", log_path);

        Command::new("ollama")
            .arg("serve")
            .env("OLLAMA_HOST", format!("{}:{}", host, port))
            .env("OLLAMA_HOME", &ollama_home)
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(log_file_err))
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

    /// Start Ollama and load a specific model
    pub async fn start_ollama_with_model(&self, model: &str) -> anyhow::Result<()> {
        // First start Ollama
        self.start_ollama().await?;

        // Then load the model
        tracing::info!("Loading model after start: {}", model);
        self.switch_model(model).await?;

        Ok(())
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

    pub async fn pull_model(&self, model: &str) -> anyhow::Result<()> {
        let client = self.inner.lock().unwrap().ollama_client.clone();

        tracing::info!("Pulling model: {}", model);
        client.pull_model_blocking(model).await?;
        tracing::info!("Model pulled: {}", model);

        Ok(())
    }

    pub fn toggle_tailscale_sharing(&self) -> anyhow::Result<()> {
        use std::process::Command;

        let tailscale_status = {
            let inner = self.inner.lock().unwrap();
            inner.tailscale_status
        };

        if tailscale_status != TailscaleStatus::Connected {
            anyhow::bail!("Tailscale is not connected");
        }

        let currently_sharing = self.is_tailscale_serving();

        if currently_sharing {
            // Disable tailscale serve
            tracing::info!("Disabling Tailscale serve");
            let output = Command::new("tailscale")
                .args(["serve", "--https=443", "off"])
                .output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!("Failed to disable tailscale serve: {}", stderr);
                anyhow::bail!("Failed to disable tailscale serve");
            }

            self.inner.lock().unwrap().tailscale_sharing = false;
        } else {
            // Enable tailscale serve on port 11434
            tracing::info!("Enabling Tailscale serve on port 11434");
            let output = Command::new("tailscale")
                .args(["serve", "--bg", "11434"])
                .output()?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                tracing::error!("Failed to enable tailscale serve: {}", stderr);
                anyhow::bail!("Failed to enable tailscale serve");
            }

            self.inner.lock().unwrap().tailscale_sharing = true;
        }

        tracing::info!("Tailscale sharing: {}", !currently_sharing);
        Ok(())
    }

    /// Check if tailscale serve is currently active
    fn is_tailscale_serving(&self) -> bool {
        use std::process::Command;

        let output = Command::new("tailscale")
            .args(["serve", "status"])
            .output();

        match output {
            Ok(out) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                // If there's serve config, it will show proxy info
                stdout.contains("proxy") || stdout.contains("http")
            }
            Err(_) => false,
        }
    }

    /// Get the tailscale serve URL if active
    pub fn tailscale_serve_url(&self) -> Option<String> {
        use std::process::Command;

        let output = Command::new("tailscale")
            .args(["serve", "status", "--json"])
            .output()
            .ok()?;

        if output.status.success() {
            let _stdout = String::from_utf8_lossy(&output.stdout);
            // Parse the serve URL from status
            // For now, construct it from the hostname
            if let Ok(dns_output) = Command::new("tailscale").args(["status", "--json"]).output() {
                let dns_stdout = String::from_utf8_lossy(&dns_output.stdout);
                if let Ok(status) = serde_json::from_str::<serde_json::Value>(&dns_stdout) {
                    if let Some(dns_name) = status["Self"]["DNSName"].as_str() {
                        let dns_name = dns_name.trim_end_matches('.');
                        return Some(format!("https://{}", dns_name));
                    }
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ollama_log_path() {
        let path = AppState::ollama_log_path();
        assert_eq!(path.to_string_lossy(), "/tmp/ollama.log");
    }

    #[test]
    fn test_ollama_log_path_is_absolute() {
        let path = AppState::ollama_log_path();
        assert!(path.is_absolute());
    }
}
