//! MCP client implementation
//!
//! Implements the Model Context Protocol client for communication with MCP servers.

use super::transport::{JsonRpcRequest, JsonRpcResponse, McpTransport};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;

/// MCP protocol version
pub const MCP_PROTOCOL_VERSION: &str = "2024-11-05";

/// Client capabilities
#[derive(Debug, Clone, Default, Serialize)]
pub struct ClientCapabilities {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roots: Option<RootsCapability>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sampling: Option<SamplingCapability>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RootsCapability {
    pub list_changed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SamplingCapability {}

/// Client info for initialization
#[derive(Debug, Clone, Serialize)]
pub struct ClientInfo {
    pub name: String,
    pub version: String,
}

impl Default for ClientInfo {
    fn default() -> Self {
        Self {
            name: "quant-cli".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
        }
    }
}

/// Server capabilities returned during initialization
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerCapabilities {
    #[serde(default)]
    pub tools: Option<ToolsCapability>,
    #[serde(default)]
    pub resources: Option<ResourcesCapability>,
    #[serde(default)]
    pub prompts: Option<PromptsCapability>,
    #[serde(default)]
    pub logging: Option<LoggingCapability>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ToolsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ResourcesCapability {
    #[serde(default)]
    pub subscribe: bool,
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct PromptsCapability {
    #[serde(default)]
    pub list_changed: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct LoggingCapability {}

/// Server info returned during initialization
#[derive(Debug, Clone, Deserialize)]
pub struct ServerInfo {
    pub name: String,
    #[serde(default)]
    pub version: Option<String>,
}

/// Initialize result
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub protocol_version: String,
    pub capabilities: ServerCapabilities,
    pub server_info: ServerInfo,
}

/// MCP Tool definition from server
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub input_schema: Value,
}

/// Tool list result
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListToolsResult {
    pub tools: Vec<McpToolInfo>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Tool call result content
#[derive(Debug, Clone, Deserialize)]
pub struct ToolResultContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub data: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Tool call result
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallToolResult {
    pub content: Vec<ToolResultContent>,
    #[serde(default)]
    pub is_error: bool,
}

/// MCP Resource definition from server
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// Resource list result
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListResourcesResult {
    pub resources: Vec<McpResource>,
    #[serde(default)]
    pub next_cursor: Option<String>,
}

/// Resource content
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub blob: Option<String>,
}

/// Read resource result
#[derive(Debug, Clone, Deserialize)]
pub struct ReadResourceResult {
    pub contents: Vec<ResourceContent>,
}

/// MCP Client
pub struct McpClient {
    transport: Arc<Mutex<Box<dyn McpTransport>>>,
    request_id: AtomicU64,
    server_info: Option<ServerInfo>,
    server_capabilities: Option<ServerCapabilities>,
    initialized: bool,
}

impl McpClient {
    /// Create a new MCP client with the given transport
    pub fn new(transport: Box<dyn McpTransport>) -> Self {
        Self {
            transport: Arc::new(Mutex::new(transport)),
            request_id: AtomicU64::new(1),
            server_info: None,
            server_capabilities: None,
            initialized: false,
        }
    }

    /// Get the next request ID
    fn next_id(&self) -> u64 {
        self.request_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a request and get the result
    async fn request<T: for<'de> Deserialize<'de>>(
        &self,
        method: &str,
        params: Option<Value>,
    ) -> Result<T> {
        let request = JsonRpcRequest::new(self.next_id(), method, params);

        let transport = self.transport.lock().await;
        let response: JsonRpcResponse = transport.send_request(request).await?;

        if let Some(error) = response.error {
            bail!("MCP error: {}", error);
        }

        let result = response.result.context("MCP response missing result")?;
        let typed_result: T =
            serde_json::from_value(result).context("Failed to parse MCP result")?;

        Ok(typed_result)
    }

    /// Initialize the connection with the MCP server
    pub async fn initialize(&mut self) -> Result<InitializeResult> {
        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": ClientCapabilities::default(),
            "clientInfo": ClientInfo::default()
        });

        let result: InitializeResult = self
            .request("initialize", Some(params))
            .await
            .context("Failed to initialize MCP connection")?;

        // Send initialized notification
        {
            let transport = self.transport.lock().await;
            transport.send_notification("notifications/initialized", None).await?;
        }

        self.server_info = Some(result.server_info.clone());
        self.server_capabilities = Some(result.capabilities.clone());
        self.initialized = true;

        Ok(result)
    }

    /// Check if the client is initialized
    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Get server info
    pub fn server_info(&self) -> Option<&ServerInfo> {
        self.server_info.as_ref()
    }

    /// Get server capabilities
    pub fn server_capabilities(&self) -> Option<&ServerCapabilities> {
        self.server_capabilities.as_ref()
    }

    /// List available tools
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        if !self.initialized {
            bail!("MCP client not initialized");
        }

        let mut tools = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = match &cursor {
                Some(c) => Some(serde_json::json!({ "cursor": c })),
                None => None,
            };

            let result: ListToolsResult = self
                .request("tools/list", params)
                .await
                .context("Failed to list MCP tools")?;

            tools.extend(result.tools);

            if result.next_cursor.is_none() {
                break;
            }
            cursor = result.next_cursor;
        }

        Ok(tools)
    }

    /// Call a tool
    pub async fn call_tool(&self, name: &str, arguments: Value) -> Result<CallToolResult> {
        if !self.initialized {
            bail!("MCP client not initialized");
        }

        let params = serde_json::json!({
            "name": name,
            "arguments": arguments
        });

        let result: CallToolResult = self
            .request("tools/call", Some(params))
            .await
            .with_context(|| format!("Failed to call MCP tool: {}", name))?;

        Ok(result)
    }

    /// List available resources
    pub async fn list_resources(&self) -> Result<Vec<McpResource>> {
        if !self.initialized {
            bail!("MCP client not initialized");
        }

        // Check if server supports resources
        if let Some(caps) = &self.server_capabilities {
            if caps.resources.is_none() {
                return Ok(Vec::new());
            }
        }

        let mut resources = Vec::new();
        let mut cursor: Option<String> = None;

        loop {
            let params = match &cursor {
                Some(c) => Some(serde_json::json!({ "cursor": c })),
                None => None,
            };

            let result: ListResourcesResult = self
                .request("resources/list", params)
                .await
                .context("Failed to list MCP resources")?;

            resources.extend(result.resources);

            if result.next_cursor.is_none() {
                break;
            }
            cursor = result.next_cursor;
        }

        Ok(resources)
    }

    /// Read a resource by URI
    pub async fn read_resource(&self, uri: &str) -> Result<ReadResourceResult> {
        if !self.initialized {
            bail!("MCP client not initialized");
        }

        let params = serde_json::json!({
            "uri": uri
        });

        let result: ReadResourceResult = self
            .request("resources/read", Some(params))
            .await
            .with_context(|| format!("Failed to read MCP resource: {}", uri))?;

        Ok(result)
    }

    /// Ping the server
    pub async fn ping(&self) -> Result<()> {
        if !self.initialized {
            bail!("MCP client not initialized");
        }

        let _: Value = self.request("ping", None).await?;
        Ok(())
    }

    /// Close the connection
    pub async fn close(&mut self) -> Result<()> {
        let mut transport = self.transport.lock().await;
        transport.close().await
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        // Note: This is a sync check, actual connection status
        // might need async verification
        self.initialized
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_client_info_default() {
        let info = ClientInfo::default();
        assert_eq!(info.name, "quant-cli");
    }

    #[test]
    fn test_initialize_params_serialization() {
        let params = serde_json::json!({
            "protocolVersion": MCP_PROTOCOL_VERSION,
            "capabilities": ClientCapabilities::default(),
            "clientInfo": ClientInfo::default()
        });

        let json = serde_json::to_string(&params).unwrap();
        assert!(json.contains("protocolVersion"));
        assert!(json.contains("quant-cli"));
    }
}
