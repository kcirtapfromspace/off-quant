//! Ollama API client

use anyhow::{Context, Result};
use futures::Stream;
use serde::{Deserialize, Serialize};
use std::pin::Pin;
use std::time::Duration;

/// Configuration for retry behavior
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retry attempts
    pub max_retries: u32,
    /// Initial delay between retries
    pub initial_delay: Duration,
    /// Maximum delay between retries
    pub max_delay: Duration,
    /// Multiplier for exponential backoff (e.g., 2.0 doubles delay each retry)
    pub backoff_multiplier: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries: 3,
            initial_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(5),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryConfig {
    /// Create a config with no retries (single attempt)
    pub fn no_retry() -> Self {
        Self {
            max_retries: 0,
            ..Default::default()
        }
    }

    /// Create a config for aggressive retrying (good for health checks)
    pub fn aggressive() -> Self {
        Self {
            max_retries: 5,
            initial_delay: Duration::from_millis(50),
            max_delay: Duration::from_secs(2),
            backoff_multiplier: 1.5,
        }
    }
}

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

/// Chat message role
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
}

/// A single chat message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: Role,
    pub content: String,
}

impl ChatMessage {
    pub fn system(content: impl Into<String>) -> Self {
        Self {
            role: Role::System,
            content: content.into(),
        }
    }

    pub fn user(content: impl Into<String>) -> Self {
        Self {
            role: Role::User,
            content: content.into(),
        }
    }

    pub fn assistant(content: impl Into<String>) -> Self {
        Self {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    options: Option<ChatOptions>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChatOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stop: Option<Vec<String>>,
}

/// Response from non-streaming chat
#[derive(Debug, Clone, Deserialize)]
pub struct ChatResponse {
    pub model: String,
    pub message: ChatMessage,
    pub done: bool,
    #[serde(default)]
    pub total_duration: u64,
    #[serde(default)]
    pub load_duration: u64,
    #[serde(default)]
    pub prompt_eval_count: u32,
    #[serde(default)]
    pub prompt_eval_duration: u64,
    #[serde(default)]
    pub eval_count: u32,
    #[serde(default)]
    pub eval_duration: u64,
}

/// Chunk from streaming chat response
#[derive(Debug, Clone, Deserialize)]
pub struct ChatChunk {
    pub model: String,
    #[serde(default)]
    pub message: Option<ChatChunkMessage>,
    pub done: bool,
    #[serde(default)]
    pub total_duration: Option<u64>,
    #[serde(default)]
    pub eval_count: Option<u32>,
    #[serde(default)]
    pub eval_duration: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChatChunkMessage {
    pub role: Role,
    pub content: String,
}

/// Type alias for the stream of chat chunks
pub type ChatStream = Pin<Box<dyn Stream<Item = Result<ChatChunk>> + Send>>;

/// Type alias for the stream of pull progress
pub type PullStream = Pin<Box<dyn Stream<Item = Result<PullProgress>> + Send>>;

#[derive(Debug, Clone, Deserialize)]
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
            Err(e) => {
                tracing::debug!("Health check failed: {}", e);
                Ok(false)
            }
        }
    }

    /// Check if Ollama is running with retry logic
    pub async fn health_check_with_retry(&self, config: &RetryConfig) -> Result<bool> {
        let mut attempt = 0;
        let mut delay = config.initial_delay;

        loop {
            match self.health_check().await {
                Ok(true) => return Ok(true),
                Ok(false) if attempt >= config.max_retries => return Ok(false),
                Err(e) if attempt >= config.max_retries => return Err(e),
                _ => {
                    attempt += 1;
                    tracing::debug!(
                        "Health check attempt {} failed, retrying in {:?}",
                        attempt,
                        delay
                    );
                    tokio::time::sleep(delay).await;
                    delay = Duration::from_secs_f64(
                        (delay.as_secs_f64() * config.backoff_multiplier).min(config.max_delay.as_secs_f64()),
                    );
                }
            }
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

    /// Get current status with retry logic
    pub async fn status_with_retry(&self, config: &RetryConfig) -> OllamaStatus {
        if self.health_check_with_retry(config).await.unwrap_or(false) {
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

    /// Pull a model with streaming progress updates
    pub async fn pull_model_stream(&self, name: &str) -> Result<PullStream> {
        let url = format!("{}/api/pull", self.base_url);

        let req = PullRequest {
            name: name.to_string(),
            stream: true,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to start model pull")?
            .error_for_status()
            .context("Model pull request failed")?;

        let stream = async_stream::try_stream! {
            use futures::StreamExt as FuturesStreamExt;

            let mut byte_stream = resp.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = FuturesStreamExt::next(&mut byte_stream).await {
                let chunk: bytes::Bytes = chunk_result.context("Error reading stream")?;
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                // Process complete lines
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let progress: PullProgress = serde_json::from_str(&line)
                        .with_context(|| format!("Failed to parse progress: {}", line))?;

                    yield progress;
                }
            }

            // Process any remaining content in buffer
            if !buffer.trim().is_empty() {
                let progress: PullProgress = serde_json::from_str(buffer.trim())
                    .with_context(|| format!("Failed to parse final progress: {}", buffer))?;
                yield progress;
            }
        };

        Ok(Box::pin(stream))
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

    /// Send a chat message (non-streaming)
    pub async fn chat(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: Option<ChatOptions>,
    ) -> Result<ChatResponse> {
        let url = format!("{}/api/chat", self.base_url);

        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: false,
            options,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .timeout(Duration::from_secs(300))
            .send()
            .await
            .context("Failed to send chat request")?
            .error_for_status()
            .context("Chat request failed")?;

        resp.json().await.context("Failed to parse chat response")
    }

    /// Send a chat message with streaming response
    pub async fn chat_stream(
        &self,
        model: &str,
        messages: &[ChatMessage],
        options: Option<ChatOptions>,
    ) -> Result<ChatStream> {
        let url = format!("{}/api/chat", self.base_url);

        let req = ChatRequest {
            model: model.to_string(),
            messages: messages.to_vec(),
            stream: true,
            options,
        };

        let resp = self
            .client
            .post(&url)
            .json(&req)
            .send()
            .await
            .context("Failed to send chat request")?
            .error_for_status()
            .context("Chat request failed")?;

        let stream = async_stream::try_stream! {
            use futures::StreamExt as FuturesStreamExt;

            let mut byte_stream = resp.bytes_stream();
            let mut buffer = String::new();

            while let Some(chunk_result) = FuturesStreamExt::next(&mut byte_stream).await {
                let chunk: bytes::Bytes = chunk_result.context("Error reading stream")?;
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                // Process complete lines
                while let Some(newline_pos) = buffer.find('\n') {
                    let line = buffer[..newline_pos].trim().to_string();
                    buffer = buffer[newline_pos + 1..].to_string();

                    if line.is_empty() {
                        continue;
                    }

                    let chat_chunk: ChatChunk = serde_json::from_str(&line)
                        .with_context(|| format!("Failed to parse chunk: {}", line))?;

                    yield chat_chunk;
                }
            }

            // Process any remaining content in buffer
            if !buffer.trim().is_empty() {
                let chat_chunk: ChatChunk = serde_json::from_str(buffer.trim())
                    .with_context(|| format!("Failed to parse final chunk: {}", buffer))?;
                yield chat_chunk;
            }
        };

        Ok(Box::pin(stream))
    }

    /// Get the base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }
}

impl Model {
    /// Get human-readable size
    pub fn size_human(&self) -> String {
        let gb = self.size as f64 / (1024.0 * 1024.0 * 1024.0);
        format!("{:.1} GB", gb)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_chat_message_constructors() {
        let system = ChatMessage::system("You are helpful");
        assert_eq!(system.role, Role::System);
        assert_eq!(system.content, "You are helpful");

        let user = ChatMessage::user("Hello");
        assert_eq!(user.role, Role::User);
        assert_eq!(user.content, "Hello");

        let assistant = ChatMessage::assistant("Hi there!");
        assert_eq!(assistant.role, Role::Assistant);
        assert_eq!(assistant.content, "Hi there!");
    }

    #[test]
    fn test_model_size_human() {
        let model = Model {
            name: "test".to_string(),
            size: 4 * 1024 * 1024 * 1024, // 4 GB
            digest: "abc123".to_string(),
            modified_at: "2024-01-01".to_string(),
            details: ModelDetails::default(),
        };
        assert_eq!(model.size_human(), "4.0 GB");
    }

    #[test]
    fn test_chat_options_default() {
        let opts = ChatOptions::default();
        assert!(opts.temperature.is_none());
        assert!(opts.top_p.is_none());
        assert!(opts.num_predict.is_none());
        assert!(opts.stop.is_none());
    }

    #[test]
    fn test_ollama_client_new() {
        let client = OllamaClient::new("http://localhost:11434");
        assert_eq!(client.base_url(), "http://localhost:11434");
    }

    #[test]
    fn test_role_serialization() {
        let user = Role::User;
        let serialized = serde_json::to_string(&user).unwrap();
        assert_eq!(serialized, r#""user""#);

        let deserialized: Role = serde_json::from_str(r#""assistant""#).unwrap();
        assert_eq!(deserialized, Role::Assistant);
    }

    #[test]
    fn test_chat_message_serialization() {
        let msg = ChatMessage::user("Hello");
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""role":"user""#));
        assert!(json.contains(r#""content":"Hello""#));
    }

    #[test]
    fn test_retry_config_default() {
        let config = RetryConfig::default();
        assert_eq!(config.max_retries, 3);
        assert_eq!(config.initial_delay, Duration::from_millis(100));
        assert_eq!(config.max_delay, Duration::from_secs(5));
        assert!((config.backoff_multiplier - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_retry_config_no_retry() {
        let config = RetryConfig::no_retry();
        assert_eq!(config.max_retries, 0);
    }

    #[test]
    fn test_retry_config_aggressive() {
        let config = RetryConfig::aggressive();
        assert_eq!(config.max_retries, 5);
        assert_eq!(config.initial_delay, Duration::from_millis(50));
    }

    // Integration tests (require Ollama to be running)
    #[cfg(feature = "integration_tests")]
    mod integration {
        use super::*;

        #[tokio::test]
        async fn test_health_check() {
            let client = OllamaClient::new("http://localhost:11434");
            // This test will pass if Ollama is running, fail gracefully if not
            let _result = client.health_check().await;
        }

        #[tokio::test]
        async fn test_list_models() {
            let client = OllamaClient::new("http://localhost:11434");
            if client.health_check().await.unwrap_or(false) {
                let models = client.list_models().await;
                assert!(models.is_ok());
            }
        }
    }
}
