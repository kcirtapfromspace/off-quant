//! Configuration management for llm.toml

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub ollama: OllamaConfig,
    pub network: NetworkConfig,
    pub models: ModelsConfig,
    pub aider: Option<AiderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct OllamaConfig {
    pub host: String,
    pub port: u16,
    pub models_path: PathBuf,
    pub ollama_home: PathBuf,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NetworkConfig {
    pub expose_port: u16,
    pub auth_user: String,
    pub auth_password_hash: String,
    pub cors_origins: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelsConfig {
    pub coding: String,
    pub chat: String,
    pub auto_select: AutoSelectConfig,
    pub local: std::collections::HashMap<String, LocalModelConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AutoSelectConfig {
    pub threshold_high: u64,
    pub threshold_medium: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalModelConfig {
    pub name: String,
    pub file: String,
    pub modelfile: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AiderConfig {
    pub model: String,
    pub auto_commits: bool,
    pub log_file: String,
}

impl Config {
    /// Load configuration from llm.toml
    pub fn load() -> Result<Self> {
        Self::load_from(Self::find_config_path()?)
    }

    /// Try to load configuration, returning None if not found
    pub fn try_load() -> Option<Self> {
        Self::load().ok()
    }

    /// Create a minimal default configuration for when llm.toml is missing
    pub fn default_minimal() -> Self {
        Self {
            ollama: OllamaConfig {
                host: "127.0.0.1".to_string(),
                port: 11434,
                models_path: std::path::PathBuf::from("/tmp/ollama/models"),
                ollama_home: std::path::PathBuf::from("/tmp/ollama"),
            },
            network: NetworkConfig {
                expose_port: 8080,
                auth_user: String::new(),
                auth_password_hash: String::new(),
                cors_origins: "*".to_string(),
            },
            models: ModelsConfig {
                coding: String::new(),
                chat: String::new(),
                auto_select: AutoSelectConfig {
                    threshold_high: 64,
                    threshold_medium: 32,
                },
                local: std::collections::HashMap::new(),
            },
            aider: None,
        }
    }

    /// Load configuration from a specific path
    pub fn load_from(path: impl AsRef<Path>) -> Result<Self> {
        let content = std::fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read {}", path.as_ref().display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse {}", path.as_ref().display()))
    }

    /// Find llm.toml by searching current directory and parents
    pub fn find_config_path() -> Result<PathBuf> {
        let mut current = std::env::current_dir()?;

        for _ in 0..10 {
            let candidate = current.join("llm.toml");
            if candidate.exists() {
                return Ok(candidate);
            }
            if !current.pop() {
                break;
            }
        }

        anyhow::bail!("llm.toml not found in current directory or parents")
    }

    /// Get Ollama base URL
    pub fn ollama_url(&self) -> String {
        format!("http://{}:{}", self.ollama.host, self.ollama.port)
    }

    /// Get system RAM in GB (macOS)
    #[cfg(target_os = "macos")]
    pub fn system_ram_gb() -> Result<u64> {
        use std::process::Command;

        let output = Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .context("Failed to run sysctl")?;

        let bytes: u64 = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .context("Failed to parse memory size")?;

        Ok(bytes / (1024 * 1024 * 1024))
    }

    #[cfg(not(target_os = "macos"))]
    pub fn system_ram_gb() -> Result<u64> {
        anyhow::bail!("system_ram_gb not implemented for this platform")
    }

    /// Auto-select best model based on RAM
    pub fn auto_select_model(&self) -> Result<String> {
        let ram = Self::system_ram_gb()?;

        if ram >= self.models.auto_select.threshold_high {
            Ok("local/qwen2.5-coder-7b-q4km".to_string())
        } else if ram >= self.models.auto_select.threshold_medium {
            Ok("local/deepseek-coder-6.7b-q4km".to_string())
        } else {
            Ok("local/starcoder2-7b-q4km".to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_config() {
        let toml = r#"
[ollama]
host = "127.0.0.1"
port = 11434
models_path = "/Volumes/models"
ollama_home = "/Volumes/models/ollama"

[network]
expose_port = 8080
auth_user = "llm"
auth_password_hash = "$2a$14$..."
cors_origins = "*"

[models]
coding = "local/qwen2.5-coder-7b-q4km"
chat = "local/glm-4-9b-chat-q4k"

[models.auto_select]
threshold_high = 64
threshold_medium = 32

[models.local.qwen]
name = "local/qwen2.5-coder-7b-q4km"
file = "qwen2.5-coder-7b-instruct-q4_k_m.gguf"
modelfile = "modelfiles/qwen2.5-coder-7b-instruct-q4km"
"#;

        let config: Config = toml::from_str(toml).unwrap();
        assert_eq!(config.ollama.port, 11434);
        assert_eq!(config.models.coding, "local/qwen2.5-coder-7b-q4km");
    }
}
