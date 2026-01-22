//! Ollama API client

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::time::Duration;

/// Ollama service status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OllamaStatus {
    /// Service is running and ready
    Running,
    /// Service is starting up
    Starting,
    /// Service is stopped
    Stopped,
    /// Service encountered an error
    Error,
}

/// Model information from Ollama API
#[derive(Debug, Clone, Deserialize)]
pub struct Model {
    pub name: String,
    pub size: u64,
    pub digest: String,
    pub modified_at: String,
    #[serde(default)]
    pub details: ModelDetails,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ModelDetails {
    pub format: Option<String>,
    pub family: Option<String>,
    pub parameter_size: Option<String>,
    pub quantization_level: Option<String>,
}

/// Running model information
#[derive(Debug, Clone, Deserialize)]
pub struct RunningModel {
    pub name: String,
    pub size: u64,
    pub digest: String,
    pub expires_at: String,
    pub size_vram: u64,
}

#[derive(Debug, Deserialize)]
struct TagsResponse {
    models: Vec<Model>,
}

#[derive(Debug, Deserialize)]
struct PsResponse {
    models: Vec<RunningModel>,
}

#[derive(Debug, Serialize)]
struct GenerateRequest {
    model: String,
    prompt: String,
    stream: bool,
}

#[derive(Debug, Serialize)]
struct PullRequest {
    name: String,
    stream: bool,
}

#[derive(Debug, Deserialize)]
pub struct PullProgress {
    pub status: String,
    #[serde(default)]
    pub digest: String,
    #[serde(default)]
    pub total: u64,
    #[serde(default)]
    pub completed: u64,
}

/// Ollama API client
#[derive(Debug, Clone)]
pub struct OllamaClient {
    base_url: String,
    client: reqwest::Client,
}

impl OllamaClient {
    /// Create a new client with default timeout
    pub fn new(base_url: impl Into<String>) -> Self {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(30))
            .build()
            .expect("Failed to create HTTP client");

        Self {
            base_url: base_url.into(),
            client,
        }
    }

    /// Check if Ollama is running
    pub async fn health_check(&self) -> Result<bool> {
        let url = format!("{}/api/tags", self.base_url);

        match self
            .client
            .get(&url)
            .timeout(Duration::from_secs(5))
            .send()
            .await
        {
            Ok(resp) => Ok(resp.status().is_success()),
            Err(_) => Ok(false),
        }
    }

    /// Get current status
    pub async fn status(&self) -> OllamaStatus {
        if self.health_check().await.unwrap_or(false) {
            OllamaStatus::Running
        } else {
            OllamaStatus::Stopped
        }
    }

    /// List all available models
    pub async fn list_models(&self) -> Result<Vec<Model>> {
        let url = format!("{}/api/tags", self.base_url);

        let resp: TagsResponse = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Ollama")?
            .json()
            .await
            .context("Failed to parse models response")?;

        Ok(resp.models)
    }

    /// List currently running/loaded models
    pub async fn list_running(&self) -> Result<Vec<RunningModel>> {
        let url = format!("{}/api/ps", self.base_url);

        let resp: PsResponse = self
            .client
            .get(&url)
            .send()
            .await
            .context("Failed to connect to Ollama")?
            .json()
            .await
            .context("Failed to parse running models response")?;

        Ok(resp.models)
    }

    /// Get the currently loaded model (if any)
    pub async fn current_model(&self) -> Result<Option<String>> {
        let running = self.list_running().await?;
        Ok(running.first().map(|m| m.name.clone()))
    }

    /// Load a model (by running a minimal generate request)
    pub async fn load_model(&self, model: &str) -> Result<()> {
        let url = format!("{}/api/generate", self.base_url);

        let req = GenerateRequest {
            model: model.to_string(),
            prompt: String::new(),
            stream: false,
        };

        self.client
            .post(&url)
            .json(&req)
            .timeout(Duration::from_secs(300)) // Models can take a while to load
            .send()
            .await
            .context("Failed to load model")?
            .error_for_status()
            .context("Model load failed")?;

        Ok(())
    }

    /// Pull a model (blocking, no progress)
    pub async fn pull_model_blocking(&self, name: &str) -> Result<()> {
        let url = format!("{}/api/pull", self.base_url);

        let req = PullRequest {
            name: name.to_string(),
            stream: false,
        };

        self.client
            .post(&url)
            .json(&req)
            .timeout(Duration::from_secs(3600)) // 1 hour timeout for large models
            .send()
            .await
            .context("Failed to pull model")?
            .error_for_status()
            .context("Model pull failed")?;

        Ok(())
    }

    /// Delete a model
    pub async fn delete_model(&self, name: &str) -> Result<()> {
        let url = format!("{}/api/delete", self.base_url);

        #[derive(Serialize)]
        struct DeleteRequest {
            name: String,
        }

        self.client
            .delete(&url)
            .json(&DeleteRequest {
                name: name.to_string(),
            })
            .send()
            .await
            .context("Failed to delete model")?
            .error_for_status()
            .context("Model delete failed")?;

        Ok(())
    }

    /// Create a model from a Modelfile
    pub async fn create_model(&self, name: &str, modelfile_content: &str) -> Result<()> {
        let url = format!("{}/api/create", self.base_url);

        #[derive(Serialize)]
        struct CreateRequest {
            name: String,
            modelfile: String,
            stream: bool,
        }

        self.client
            .post(&url)
            .json(&CreateRequest {
                name: name.to_string(),
                modelfile: modelfile_content.to_string(),
                stream: false,
            })
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .context("Failed to create model")?
            .error_for_status()
            .context("Model creation failed")?;

        Ok(())
    }
}

impl Model {
    /// Get human-readable size
    pub fn size_human(&self) -> String {
        let gb = self.size as f64 / (1024.0 * 1024.0 * 1024.0);
        format!("{:.1} GB", gb)
    }
}
