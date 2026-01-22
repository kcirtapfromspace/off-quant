//! MCP server configuration parsing
//!
//! Supports configuration from QUANT.md frontmatter and global config.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Configuration for an MCP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServerConfig {
    /// Unique name for this server (used in tool prefixes)
    pub name: String,
    /// Command to run the server
    pub command: String,
    /// Arguments to pass to the command
    #[serde(default)]
    pub args: Vec<String>,
    /// Environment variables (supports ${VAR} expansion)
    #[serde(default)]
    pub env: HashMap<String, String>,
    /// Working directory for the server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<PathBuf>,
    /// Security level override for all tools from this server
    #[serde(skip_serializing_if = "Option::is_none")]
    pub security_level: Option<String>,
    /// Whether to auto-start this server
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
    /// Timeout for server operations in seconds
    #[serde(default = "default_timeout")]
    pub timeout_secs: u64,
}

fn default_auto_start() -> bool {
    true
}

fn default_timeout() -> u64 {
    30
}

impl McpServerConfig {
    /// Create a new server config with just name and command
    pub fn new(name: impl Into<String>, command: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            command: command.into(),
            args: Vec::new(),
            env: HashMap::new(),
            cwd: None,
            security_level: None,
            auto_start: true,
            timeout_secs: 30,
        }
    }

    /// Add an argument
    pub fn with_arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Add arguments
    pub fn with_args(mut self, args: impl IntoIterator<Item = impl Into<String>>) -> Self {
        self.args.extend(args.into_iter().map(Into::into));
        self
    }

    /// Add an environment variable
    pub fn with_env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.env.insert(key.into(), value.into());
        self
    }

    /// Set working directory
    pub fn with_cwd(mut self, cwd: impl Into<PathBuf>) -> Self {
        self.cwd = Some(cwd.into());
        self
    }

    /// Expand environment variables in config values
    pub fn expand_env_vars(&mut self) -> Result<()> {
        // Expand in env values
        for value in self.env.values_mut() {
            *value = expand_env_string(value)?;
        }
        Ok(())
    }
}

/// Expand ${VAR} patterns in a string using environment variables
pub fn expand_env_string(s: &str) -> Result<String> {
    let mut result = s.to_string();
    let re = regex::Regex::new(r"\$\{([^}]+)\}").unwrap();

    for cap in re.captures_iter(s) {
        let var_name = &cap[1];
        let var_value = std::env::var(var_name)
            .with_context(|| format!("Environment variable {} not set", var_name))?;
        result = result.replace(&cap[0], &var_value);
    }

    Ok(result)
}

/// Global MCP configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct McpConfig {
    /// Default timeout for all servers
    #[serde(default = "default_timeout")]
    pub default_timeout_secs: u64,
    /// Whether to auto-start servers by default
    #[serde(default = "default_auto_start")]
    pub auto_start: bool,
    /// Global MCP servers (from config.toml)
    #[serde(default)]
    pub servers: Vec<McpServerConfig>,
}

impl McpConfig {
    /// Load from global config file
    pub fn load_global() -> Result<Self> {
        let config_path = dirs::config_dir()
            .map(|d| d.join("quant").join("config.toml"))
            .context("Could not determine config directory")?;

        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(&config_path)
            .with_context(|| format!("Failed to read config from {:?}", config_path))?;

        let config: toml::Value = toml::from_str(&content)
            .with_context(|| "Failed to parse config.toml")?;

        // Extract [mcp] section
        if let Some(mcp) = config.get("mcp") {
            let mcp_config: McpConfig = mcp.clone().try_into()
                .with_context(|| "Failed to parse [mcp] section")?;
            return Ok(mcp_config);
        }

        Ok(Self::default())
    }

    /// Merge project-level config with global config
    pub fn merge_with_project(&mut self, project_servers: Vec<McpServerConfig>) {
        // Project servers take precedence (add them first, skip duplicates)
        let project_names: std::collections::HashSet<_> =
            project_servers.iter().map(|s| s.name.clone()).collect();

        let mut merged = project_servers;

        // Add global servers that aren't overridden
        for server in &self.servers {
            if !project_names.contains(&server.name) {
                merged.push(server.clone());
            }
        }

        self.servers = merged;
    }
}

/// Parse MCP servers from QUANT.md frontmatter
pub fn parse_mcp_servers_from_yaml(yaml_str: &str) -> Result<Vec<McpServerConfig>> {
    let value: serde_yaml::Value = serde_yaml::from_str(yaml_str)
        .context("Failed to parse YAML frontmatter")?;

    let servers = value
        .get("mcp_servers")
        .cloned()
        .unwrap_or(serde_yaml::Value::Sequence(vec![]));

    let configs: Vec<McpServerConfig> = serde_yaml::from_value(servers)
        .context("Failed to parse mcp_servers configuration")?;

    Ok(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_env_string() {
        std::env::set_var("TEST_VAR", "hello");
        let result = expand_env_string("prefix_${TEST_VAR}_suffix").unwrap();
        assert_eq!(result, "prefix_hello_suffix");
    }

    #[test]
    fn test_parse_mcp_servers() {
        let yaml = r#"
mcp_servers:
  - name: "github"
    command: "npx"
    args: ["-y", "@modelcontextprotocol/server-github"]
    env:
      GITHUB_TOKEN: "test-token"
"#;
        let servers = parse_mcp_servers_from_yaml(yaml).unwrap();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "github");
        assert_eq!(servers[0].command, "npx");
        assert_eq!(servers[0].args, vec!["-y", "@modelcontextprotocol/server-github"]);
    }
}
