//! MCP transport layer
//!
//! Supports stdio and HTTP transports for MCP server communication.

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::process::Stdio;
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex;

/// JSON-RPC 2.0 request
#[derive(Debug, Clone, Serialize)]
pub struct JsonRpcRequest {
    pub jsonrpc: &'static str,
    pub id: u64,
    pub method: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub params: Option<Value>,
}

impl JsonRpcRequest {
    pub fn new(id: u64, method: impl Into<String>, params: Option<Value>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            method: method.into(),
            params,
        }
    }
}

/// JSON-RPC 2.0 response
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    pub id: Option<u64>,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<JsonRpcError>,
}

/// JSON-RPC 2.0 error
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcError {
    pub code: i64,
    pub message: String,
    #[serde(default)]
    pub data: Option<Value>,
}

impl std::fmt::Display for JsonRpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "JSON-RPC error {}: {}", self.code, self.message)
    }
}

/// JSON-RPC 2.0 notification (no id)
#[derive(Debug, Clone, Deserialize)]
pub struct JsonRpcNotification {
    pub jsonrpc: String,
    pub method: String,
    #[serde(default)]
    pub params: Option<Value>,
}

/// Transport trait for MCP communication
#[async_trait]
pub trait McpTransport: Send + Sync {
    /// Send a request and wait for response
    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse>;

    /// Send a notification (no response expected)
    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()>;

    /// Check if transport is still connected
    fn is_connected(&self) -> bool;

    /// Close the transport
    async fn close(&mut self) -> Result<()>;
}

/// Stdio transport for MCP servers running as child processes
pub struct StdioTransport {
    stdin: Arc<Mutex<ChildStdin>>,
    stdout: Arc<Mutex<BufReader<ChildStdout>>>,
    child: Arc<Mutex<Child>>,
    connected: std::sync::atomic::AtomicBool,
}

impl StdioTransport {
    /// Create a new stdio transport from a running process
    pub fn new(mut child: Child) -> Result<Self> {
        let stdin = child
            .stdin
            .take()
            .context("Failed to capture stdin of MCP server")?;
        let stdout = child
            .stdout
            .take()
            .context("Failed to capture stdout of MCP server")?;

        Ok(Self {
            stdin: Arc::new(Mutex::new(stdin)),
            stdout: Arc::new(Mutex::new(BufReader::new(stdout))),
            child: Arc::new(Mutex::new(child)),
            connected: std::sync::atomic::AtomicBool::new(true),
        })
    }

    /// Spawn a new process and create transport
    pub async fn spawn(
        command: &str,
        args: &[String],
        env: &std::collections::HashMap<String, String>,
        cwd: Option<&std::path::Path>,
    ) -> Result<Self> {
        let mut cmd = Command::new(command);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);

        // Set environment variables
        for (key, value) in env {
            cmd.env(key, value);
        }

        // Set working directory if specified
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("Failed to spawn MCP server: {}", command))?;

        Self::new(child)
    }

    /// Read a line from stdout, parsing as JSON
    async fn read_message(&self) -> Result<Value> {
        let mut stdout = self.stdout.lock().await;
        let mut line = String::new();

        // MCP uses newline-delimited JSON
        stdout
            .read_line(&mut line)
            .await
            .context("Failed to read from MCP server")?;

        if line.is_empty() {
            bail!("MCP server closed connection");
        }

        let value: Value =
            serde_json::from_str(&line).context("Failed to parse JSON from MCP server")?;

        Ok(value)
    }

    /// Write a message to stdin
    async fn write_message(&self, value: &Value) -> Result<()> {
        let mut stdin = self.stdin.lock().await;
        let json = serde_json::to_string(value)?;

        stdin.write_all(json.as_bytes()).await?;
        stdin.write_all(b"\n").await?;
        stdin.flush().await?;

        Ok(())
    }
}

#[async_trait]
impl McpTransport for StdioTransport {
    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let request_id = request.id;

        // Send request
        let value = serde_json::to_value(&request)?;
        self.write_message(&value).await?;

        // Read responses until we get one matching our ID
        loop {
            let response_value = self.read_message().await?;

            // Check if this is a notification (no id)
            if response_value.get("id").is_none() {
                // It's a notification, skip it for now
                // TODO: Handle notifications properly
                continue;
            }

            let response: JsonRpcResponse = serde_json::from_value(response_value)
                .context("Failed to parse JSON-RPC response")?;

            // Check if response matches our request
            if response.id == Some(request_id) {
                return Ok(response);
            }
        }
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        self.write_message(&notification).await
    }

    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn close(&mut self) -> Result<()> {
        self.connected
            .store(false, std::sync::atomic::Ordering::SeqCst);

        // Try to kill the child process
        let mut child = self.child.lock().await;
        let _ = child.kill().await;

        Ok(())
    }
}

/// HTTP/SSE transport for remote MCP servers
pub struct HttpTransport {
    base_url: String,
    client: reqwest::Client,
    connected: std::sync::atomic::AtomicBool,
}

impl HttpTransport {
    /// Create a new HTTP transport
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            client: reqwest::Client::new(),
            connected: std::sync::atomic::AtomicBool::new(true),
        }
    }
}

#[async_trait]
impl McpTransport for HttpTransport {
    async fn send_request(&self, request: JsonRpcRequest) -> Result<JsonRpcResponse> {
        let response = self
            .client
            .post(&self.base_url)
            .json(&request)
            .send()
            .await
            .context("Failed to send HTTP request to MCP server")?;

        if !response.status().is_success() {
            bail!(
                "MCP server returned error status: {}",
                response.status()
            );
        }

        let json_response: JsonRpcResponse = response
            .json()
            .await
            .context("Failed to parse JSON-RPC response from MCP server")?;

        Ok(json_response)
    }

    async fn send_notification(&self, method: &str, params: Option<Value>) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params
        });

        self.client
            .post(&self.base_url)
            .json(&notification)
            .send()
            .await
            .context("Failed to send notification to MCP server")?;

        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.connected.load(std::sync::atomic::Ordering::SeqCst)
    }

    async fn close(&mut self) -> Result<()> {
        self.connected
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_json_rpc_request_serialization() {
        let request = JsonRpcRequest::new(
            1,
            "tools/list",
            Some(serde_json::json!({"cursor": null})),
        );

        let json = serde_json::to_string(&request).unwrap();
        assert!(json.contains("\"jsonrpc\":\"2.0\""));
        assert!(json.contains("\"id\":1"));
        assert!(json.contains("\"method\":\"tools/list\""));
    }
}
