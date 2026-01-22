//! User configuration for quant CLI
//!
//! Configuration file: ~/.config/quant/config.toml (or platform equivalent)

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

/// User configuration for the quant CLI
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UserConfig {
    /// REPL configuration
    #[serde(default)]
    pub repl: ReplConfig,

    /// Default options for ask command
    #[serde(default)]
    pub ask: AskConfig,

    /// Aliases for commands/models
    #[serde(default)]
    pub aliases: AliasConfig,
}

/// REPL-specific configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplConfig {
    /// Default model for chat
    #[serde(default)]
    pub default_model: Option<String>,

    /// Default system prompt
    #[serde(default)]
    pub system_prompt: Option<String>,

    /// Auto-save conversations on exit
    #[serde(default)]
    pub auto_save: bool,

    /// Show timestamps in conversation history
    #[serde(default)]
    pub show_timestamps: bool,

    /// Maximum history entries to keep
    #[serde(default = "default_history_size")]
    pub history_size: usize,

    /// Color theme (light/dark/auto)
    #[serde(default = "default_theme")]
    pub theme: String,
}

/// Ask command configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AskConfig {
    /// Default model for one-shot queries
    #[serde(default)]
    pub default_model: Option<String>,

    /// Default temperature
    #[serde(default)]
    pub temperature: Option<f32>,

    /// Default max tokens
    #[serde(default)]
    pub max_tokens: Option<i32>,
}

/// Model and command aliases
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AliasConfig {
    /// Model aliases (e.g., "code" -> "deepseek-coder:6.7b")
    #[serde(default)]
    pub models: std::collections::HashMap<String, String>,
}

fn default_history_size() -> usize {
    1000
}

fn default_theme() -> String {
    "auto".to_string()
}

impl Default for ReplConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            system_prompt: None,
            auto_save: false,
            show_timestamps: false,
            history_size: default_history_size(),
            theme: default_theme(),
        }
    }
}

impl Default for AskConfig {
    fn default() -> Self {
        Self {
            default_model: None,
            temperature: None,
            max_tokens: None,
        }
    }
}

impl UserConfig {
    /// Load user configuration from default location
    pub fn load() -> Result<Self> {
        let path = Self::config_path()?;

        if !path.exists() {
            return Ok(Self::default());
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read config from {}", path.display()))?;

        toml::from_str(&content)
            .with_context(|| format!("Failed to parse config from {}", path.display()))
    }

    /// Save configuration to default location
    #[allow(dead_code)]
    pub fn save(&self) -> Result<PathBuf> {
        let path = Self::config_path()?;

        // Create directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let content = toml::to_string_pretty(self)?;
        fs::write(&path, content)?;

        Ok(path)
    }

    /// Get the configuration file path
    pub fn config_path() -> Result<PathBuf> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

        Ok(config_dir.join("quant").join("config.toml"))
    }

    /// Create a default configuration file with comments
    pub fn create_default() -> Result<PathBuf> {
        let path = Self::config_path()?;

        if path.exists() {
            anyhow::bail!("Config file already exists: {}", path.display());
        }

        // Create directory if needed
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }

        let default_config = r#"# quant CLI configuration
# Location: ~/.config/quant/config.toml

[repl]
# Default model for interactive chat (uses llm.toml chat model if not set)
# default_model = "deepseek-coder:6.7b"

# Default system prompt for all conversations
# system_prompt = "You are a helpful coding assistant."

# Auto-save conversations on exit
auto_save = false

# Show timestamps in conversation history
show_timestamps = false

# Maximum history entries to keep
history_size = 1000

# Color theme: "light", "dark", or "auto"
theme = "auto"

[ask]
# Default model for one-shot queries (uses llm.toml coding model if not set)
# default_model = "deepseek-coder:6.7b"

# Default temperature (0.0-2.0)
# temperature = 0.7

# Default max tokens
# max_tokens = 4096

[aliases.models]
# Model aliases for quick access
# code = "deepseek-coder:6.7b"
# chat = "glm4:9b"
"#;

        fs::write(&path, default_config)?;

        Ok(path)
    }

    /// Resolve a model name (check aliases first)
    #[allow(dead_code)]
    pub fn resolve_model(&self, name: &str) -> String {
        self.aliases
            .models
            .get(name)
            .cloned()
            .unwrap_or_else(|| name.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = UserConfig::default();
        assert!(!config.repl.auto_save);
        assert_eq!(config.repl.history_size, 1000);
    }

    #[test]
    fn test_parse_config() {
        let toml = r#"
[repl]
default_model = "test-model"
auto_save = true

[ask]
temperature = 0.8

[aliases.models]
code = "deepseek-coder:6.7b"
"#;

        let config: UserConfig = toml::from_str(toml).unwrap();
        assert_eq!(config.repl.default_model, Some("test-model".to_string()));
        assert!(config.repl.auto_save);
        assert_eq!(config.ask.temperature, Some(0.8));
        assert_eq!(
            config.resolve_model("code"),
            "deepseek-coder:6.7b".to_string()
        );
    }
}
