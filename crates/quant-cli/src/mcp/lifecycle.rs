//! MCP server lifecycle management
//!
//! Handles starting, stopping, and monitoring MCP server processes.

use super::client::McpClient;
use super::config::McpServerConfig;
use super::tools::PrefixedMcpTool;
use super::transport::StdioTransport;
use anyhow::{bail, Context, Result};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::{debug, error, info, warn};

/// State of an MCP server
#[derive(Debug, Clone, PartialEq)]
pub enum ServerState {
    /// Server is not running
    Stopped,
    /// Server is starting up
    Starting,
    /// Server is running and initialized
    Running,
    /// Server failed to start or crashed
    Failed(String),
    /// Server is shutting down
    ShuttingDown,
}

/// Information about a running MCP server
pub struct McpServerHandle {
    /// Server configuration
    pub config: McpServerConfig,
    /// MCP client for this server
    pub client: Arc<Mutex<McpClient>>,
    /// Current state
    pub state: ServerState,
    /// Number of restart attempts
    pub restart_count: u32,
    /// Last error message
    pub last_error: Option<String>,
}

impl McpServerHandle {
    /// Create a new server handle
    fn new(config: McpServerConfig, client: McpClient) -> Self {
        Self {
            config,
            client: Arc::new(Mutex::new(client)),
            state: ServerState::Stopped,
            restart_count: 0,
            last_error: None,
        }
    }
}

/// Manager for MCP server lifecycle
pub struct McpManager {
    /// Running server handles by name
    servers: HashMap<String, McpServerHandle>,
    /// Maximum restart attempts before giving up
    max_restarts: u32,
    /// Initialization timeout
    init_timeout: Duration,
}

impl McpManager {
    /// Create a new MCP manager
    pub fn new() -> Self {
        Self {
            servers: HashMap::new(),
            max_restarts: 3,
            init_timeout: Duration::from_secs(30),
        }
    }

    /// Set maximum restart attempts
    pub fn with_max_restarts(mut self, max: u32) -> Self {
        self.max_restarts = max;
        self
    }

    /// Set initialization timeout
    pub fn with_init_timeout(mut self, timeout: Duration) -> Self {
        self.init_timeout = timeout;
        self
    }

    /// Start a single MCP server
    pub async fn start_server(&mut self, mut config: McpServerConfig) -> Result<()> {
        let name = config.name.clone();
        info!("Starting MCP server: {}", name);

        // Expand environment variables
        config.expand_env_vars().with_context(|| {
            format!("Failed to expand environment variables for MCP server: {}", name)
        })?;

        // Spawn the transport
        let transport = StdioTransport::spawn(
            &config.command,
            &config.args,
            &config.env,
            config.cwd.as_deref(),
        )
        .await
        .with_context(|| format!("Failed to spawn MCP server: {}", name))?;

        // Create client
        let mut client = McpClient::new(Box::new(transport));

        // Initialize with timeout
        let init_timeout = Duration::from_secs(config.timeout_secs);
        match timeout(init_timeout, client.initialize()).await {
            Ok(Ok(result)) => {
                info!(
                    "MCP server {} initialized: {} v{}",
                    name,
                    result.server_info.name,
                    result.server_info.version.as_deref().unwrap_or("unknown")
                );
            }
            Ok(Err(e)) => {
                error!("Failed to initialize MCP server {}: {}", name, e);
                bail!("Failed to initialize MCP server {}: {}", name, e);
            }
            Err(_) => {
                error!("MCP server {} initialization timed out", name);
                bail!("MCP server {} initialization timed out", name);
            }
        }

        // Create handle and store
        let mut handle = McpServerHandle::new(config, client);
        handle.state = ServerState::Running;

        self.servers.insert(name, handle);

        Ok(())
    }

    /// Start all configured servers
    pub async fn start_all(&mut self, configs: Vec<McpServerConfig>) -> Vec<String> {
        let mut failures = Vec::new();

        for config in configs {
            if !config.auto_start {
                debug!("Skipping MCP server {} (auto_start=false)", config.name);
                continue;
            }

            if let Err(e) = self.start_server(config.clone()).await {
                warn!("Failed to start MCP server {}: {}", config.name, e);
                failures.push(config.name);
            }
        }

        failures
    }

    /// Stop a single server
    pub async fn stop_server(&mut self, name: &str) -> Result<()> {
        if let Some(mut handle) = self.servers.remove(name) {
            info!("Stopping MCP server: {}", name);
            handle.state = ServerState::ShuttingDown;

            let mut client = handle.client.lock().await;
            if let Err(e) = client.close().await {
                warn!("Error closing MCP server {}: {}", name, e);
            }
        }

        Ok(())
    }

    /// Stop all servers
    pub async fn stop_all(&mut self) {
        let names: Vec<_> = self.servers.keys().cloned().collect();
        for name in names {
            if let Err(e) = self.stop_server(&name).await {
                warn!("Error stopping MCP server {}: {}", name, e);
            }
        }
    }

    /// Restart a server
    pub async fn restart_server(&mut self, name: &str) -> Result<()> {
        if let Some(handle) = self.servers.get(name) {
            let config = handle.config.clone();
            self.stop_server(name).await?;
            self.start_server(config).await?;
        }
        Ok(())
    }

    /// Get all running server names
    pub fn running_servers(&self) -> Vec<&str> {
        self.servers
            .iter()
            .filter(|(_, h)| h.state == ServerState::Running)
            .map(|(name, _)| name.as_str())
            .collect()
    }

    /// Get server state
    pub fn server_state(&self, name: &str) -> Option<&ServerState> {
        self.servers.get(name).map(|h| &h.state)
    }

    /// Check if a server is running
    pub fn is_running(&self, name: &str) -> bool {
        self.servers
            .get(name)
            .map(|h| h.state == ServerState::Running)
            .unwrap_or(false)
    }

    /// Discover all tools from running servers
    pub async fn discover_tools(&self) -> Result<Vec<PrefixedMcpTool>> {
        let mut all_tools = Vec::new();

        for (name, handle) in &self.servers {
            if handle.state != ServerState::Running {
                continue;
            }

            let client = handle.client.lock().await;
            match client.list_tools().await {
                Ok(tools) => {
                    debug!("Discovered {} tools from MCP server {}", tools.len(), name);

                    // Parse security level from config
                    let security_level = handle
                        .config
                        .security_level
                        .as_ref()
                        .and_then(|s| super::tools::parse_security_level(s))
                        .unwrap_or(crate::tools::SecurityLevel::Moderate);

                    for tool_info in tools {
                        let tool = PrefixedMcpTool::new(
                            name.clone(),
                            tool_info,
                            Arc::clone(&handle.client),
                        )
                        .with_security_level(security_level);
                        all_tools.push(tool);
                    }
                }
                Err(e) => {
                    warn!("Failed to list tools from MCP server {}: {}", name, e);
                }
            }
        }

        Ok(all_tools)
    }

    /// Get a client for a specific server
    pub fn get_client(&self, name: &str) -> Option<Arc<Mutex<McpClient>>> {
        self.servers
            .get(name)
            .filter(|h| h.state == ServerState::Running)
            .map(|h| Arc::clone(&h.client))
    }

    /// Health check all servers
    pub async fn health_check(&mut self) -> HashMap<String, bool> {
        let mut results = HashMap::new();

        for (name, handle) in &mut self.servers {
            if handle.state != ServerState::Running {
                results.insert(name.clone(), false);
                continue;
            }

            let client = handle.client.lock().await;
            match timeout(Duration::from_secs(5), client.ping()).await {
                Ok(Ok(())) => {
                    results.insert(name.clone(), true);
                }
                Ok(Err(e)) => {
                    warn!("MCP server {} health check failed: {}", name, e);
                    handle.state = ServerState::Failed(e.to_string());
                    handle.last_error = Some(e.to_string());
                    results.insert(name.clone(), false);
                }
                Err(_) => {
                    warn!("MCP server {} health check timed out", name);
                    handle.state = ServerState::Failed("Health check timed out".to_string());
                    handle.last_error = Some("Health check timed out".to_string());
                    results.insert(name.clone(), false);
                }
            }
        }

        results
    }

    /// Get summary of all servers
    pub fn summary(&self) -> Vec<ServerSummary> {
        self.servers
            .iter()
            .map(|(name, handle)| ServerSummary {
                name: name.clone(),
                command: handle.config.command.clone(),
                state: format!("{:?}", handle.state),
                restart_count: handle.restart_count,
                last_error: handle.last_error.clone(),
            })
            .collect()
    }

    /// Discover all resources from running servers
    pub async fn discover_resources(&self) -> Vec<McpResourceInfo> {
        let mut all_resources = Vec::new();

        for (name, handle) in &self.servers {
            if handle.state != ServerState::Running {
                continue;
            }

            let client = handle.client.lock().await;
            match client.list_resources().await {
                Ok(resources) => {
                    for resource in resources {
                        all_resources.push(McpResourceInfo {
                            server: name.clone(),
                            uri: resource.uri,
                            name: resource.name,
                            description: resource.description,
                            mime_type: resource.mime_type,
                        });
                    }
                }
                Err(e) => {
                    warn!("Failed to list resources from MCP server {}: {}", name, e);
                }
            }
        }

        all_resources
    }

    /// Read a resource by URI from a specific server
    pub async fn read_resource(&self, server_name: &str, uri: &str) -> Result<String> {
        let handle = self.servers.get(server_name)
            .ok_or_else(|| anyhow::anyhow!("Server not found: {}", server_name))?;

        if handle.state != ServerState::Running {
            anyhow::bail!("Server {} is not running", server_name);
        }

        let client = handle.client.lock().await;
        let result = client.read_resource(uri).await?;

        // Combine all content into a string
        let mut content = String::new();
        for item in result.contents {
            if let Some(text) = item.text {
                content.push_str(&text);
            }
        }

        Ok(content)
    }
}

/// Information about an MCP resource
#[derive(Debug, Clone)]
pub struct McpResourceInfo {
    pub server: String,
    pub uri: String,
    pub name: String,
    pub description: Option<String>,
    pub mime_type: Option<String>,
}

impl Default for McpManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for McpManager {
    fn drop(&mut self) {
        // Note: Can't do async cleanup in drop
        // Servers will be killed via kill_on_drop in the process
        if !self.servers.is_empty() {
            debug!("McpManager dropping with {} servers", self.servers.len());
        }
    }
}

/// Summary of a server's status
#[derive(Debug, Clone)]
pub struct ServerSummary {
    pub name: String,
    pub command: String,
    pub state: String,
    pub restart_count: u32,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_manager_creation() {
        let manager = McpManager::new();
        assert!(manager.servers.is_empty());
        assert_eq!(manager.running_servers().len(), 0);
    }

    #[test]
    fn test_server_state_equality() {
        assert_eq!(ServerState::Running, ServerState::Running);
        assert_ne!(ServerState::Running, ServerState::Stopped);
    }
}
