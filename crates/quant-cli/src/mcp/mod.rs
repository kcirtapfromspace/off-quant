//! MCP (Model Context Protocol) client support
//!
//! This module provides integration with MCP servers, enabling the use of
//! external tools through the Model Context Protocol.
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                     McpManager                          │
//! │  - Orchestrates server lifecycle                        │
//! │  - Discovers and registers tools                        │
//! │  - Handles restarts and health checks                   │
//! └─────────────────┬───────────────────────────────────────┘
//!                   │
//!          ┌────────┴────────┐
//!          │                 │
//!          ▼                 ▼
//! ┌─────────────────┐ ┌─────────────────┐
//! │  McpClient      │ │  McpClient      │
//! │  (github)       │ │  (filesystem)   │
//! └────────┬────────┘ └────────┬────────┘
//!          │                   │
//!          ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐
//! │  StdioTransport │ │  StdioTransport │
//! └────────┬────────┘ └────────┬────────┘
//!          │                   │
//!          ▼                   ▼
//! ┌─────────────────┐ ┌─────────────────┐
//! │  MCP Server     │ │  MCP Server     │
//! │  (npx github)   │ │  (npx fs)       │
//! └─────────────────┘ └─────────────────┘
//! ```
//!
//! # Usage
//!
//! ```rust,ignore
//! use quant_cli::mcp::{McpManager, McpServerConfig};
//!
//! // Create manager
//! let mut manager = McpManager::new();
//!
//! // Start servers from config
//! let config = McpServerConfig::new("github", "npx")
//!     .with_args(["-y", "@modelcontextprotocol/server-github"])
//!     .with_env("GITHUB_TOKEN", std::env::var("GITHUB_TOKEN")?);
//!
//! manager.start_server(config).await?;
//!
//! // Discover tools
//! let tools = manager.discover_tools().await?;
//! for tool in &tools {
//!     registry.register(tool);
//! }
//!
//! // Clean up
//! manager.stop_all().await;
//! ```
//!
//! # Configuration
//!
//! MCP servers can be configured in QUANT.md frontmatter:
//!
//! ```yaml
//! ---
//! mcp_servers:
//!   - name: "github"
//!     command: "npx"
//!     args: ["-y", "@modelcontextprotocol/server-github"]
//!     env:
//!       GITHUB_TOKEN: "${GITHUB_TOKEN}"
//!   - name: "filesystem"
//!     command: "npx"
//!     args: ["-y", "@modelcontextprotocol/server-filesystem", "./"]
//! ---
//! ```

pub mod client;
pub mod config;
pub mod lifecycle;
pub mod tools;
pub mod transport;
pub mod watcher;

// Re-exports
pub use client::{McpClient, McpResource, McpToolInfo};
pub use config::{McpConfig, McpServerConfig};
pub use lifecycle::{McpManager, McpResourceInfo, ServerState, ServerSummary};
pub use tools::{McpTool, PrefixedMcpTool};
pub use transport::{HttpTransport, McpTransport, StdioTransport};
pub use watcher::{ConfigChangeEvent, ConfigWatcher};

use crate::tools::registry::ToolRegistry;
use crate::tools::Tool;
use anyhow::Result;

/// Extension trait for ToolRegistry to add MCP tools
pub trait McpRegistryExt {
    /// Register all tools from an MCP manager
    fn register_mcp_tools(&mut self, tools: Vec<PrefixedMcpTool>);
}

impl McpRegistryExt for ToolRegistry {
    fn register_mcp_tools(&mut self, tools: Vec<PrefixedMcpTool>) {
        for tool in tools {
            tracing::debug!("Registering MCP tool: {}", tool.name());
            self.register(tool);
        }
    }
}

/// Create a registry with MCP tools
pub async fn create_registry_with_mcp(
    manager: &McpManager,
    include_builtin: bool,
) -> Result<ToolRegistry> {
    use crate::tools::builtin;

    let mut registry = if include_builtin {
        builtin::create_default_registry()
    } else {
        ToolRegistry::new()
    };

    // Discover and register MCP tools
    let mcp_tools = manager.discover_tools().await?;
    registry.register_mcp_tools(mcp_tools);

    Ok(registry)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_module_exports() {
        // Just verify exports compile
        let _ = McpManager::new();
    }
}
