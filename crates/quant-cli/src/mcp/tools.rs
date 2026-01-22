//! MCP Tool adapter
//!
//! Wraps MCP tools as quant Tool trait implementations.

use super::client::{CallToolResult, McpClient, McpToolInfo};
use crate::tools::{
    ParameterProperty, ParameterSchema, SecurityLevel, Tool, ToolContext, ToolDefinition,
    ToolResult,
};
use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Adapter that wraps an MCP tool as a quant Tool
pub struct McpTool {
    /// Server name (used as prefix)
    server_name: String,
    /// Original tool info from MCP server
    tool_info: McpToolInfo,
    /// Client for executing the tool
    client: Arc<Mutex<McpClient>>,
    /// Security level override
    security_level: SecurityLevel,
}

impl McpTool {
    /// Create a new MCP tool adapter
    pub fn new(
        server_name: impl Into<String>,
        tool_info: McpToolInfo,
        client: Arc<Mutex<McpClient>>,
    ) -> Self {
        Self {
            server_name: server_name.into(),
            tool_info,
            client,
            security_level: SecurityLevel::Moderate, // Default for MCP tools
        }
    }

    /// Set the security level
    pub fn with_security_level(mut self, level: SecurityLevel) -> Self {
        self.security_level = level;
        self
    }

    /// Get the prefixed tool name (server__tool)
    pub fn prefixed_name(&self) -> String {
        format!("{}_{}", self.server_name, self.tool_info.name)
    }

    /// Get the original (unprefixed) tool name
    pub fn original_name(&self) -> &str {
        &self.tool_info.name
    }

    /// Get the server name
    pub fn server_name(&self) -> &str {
        &self.server_name
    }

    /// Convert MCP JSON Schema to quant ParameterSchema
    fn convert_schema(&self) -> ParameterSchema {
        let mut schema = ParameterSchema::new();

        // Extract properties from MCP input_schema
        if let Some(props) = self.tool_info.input_schema.get("properties") {
            if let Some(obj) = props.as_object() {
                for (name, prop_value) in obj {
                    let prop = convert_json_schema_property(prop_value);
                    schema.properties.insert(name.clone(), prop);
                }
            }
        }

        // Extract required fields
        if let Some(required) = self.tool_info.input_schema.get("required") {
            if let Some(arr) = required.as_array() {
                schema.required = arr
                    .iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect();
            }
        }

        schema
    }
}

/// Convert a JSON Schema property to a ParameterProperty
fn convert_json_schema_property(value: &Value) -> ParameterProperty {
    let param_type = value
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("string")
        .to_string();

    let description = value
        .get("description")
        .and_then(|d| d.as_str())
        .unwrap_or("")
        .to_string();

    let mut prop = ParameterProperty {
        param_type,
        description,
        enum_values: None,
        default: None,
    };

    // Handle enum values
    if let Some(enum_vals) = value.get("enum") {
        if let Some(arr) = enum_vals.as_array() {
            prop.enum_values = Some(
                arr.iter()
                    .filter_map(|v| v.as_str().map(String::from))
                    .collect(),
            );
        }
    }

    // Handle default values
    if let Some(default) = value.get("default") {
        prop.default = Some(default.clone());
    }

    prop
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        // We need to return a &str, so we'll cache the prefixed name
        // For now, use a workaround by returning the original name
        // and handling prefixing in the registry
        &self.tool_info.name
    }

    fn description(&self) -> &str {
        self.tool_info
            .description
            .as_deref()
            .unwrap_or("MCP tool")
    }

    fn security_level(&self) -> SecurityLevel {
        self.security_level
    }

    fn parameters_schema(&self) -> ParameterSchema {
        self.convert_schema()
    }

    async fn execute(&self, args: &Value, _ctx: &ToolContext) -> Result<ToolResult> {
        let client = self.client.lock().await;

        // Call the MCP tool with original (unprefixed) name
        let result = client
            .call_tool(&self.tool_info.name, args.clone())
            .await?;

        // Convert MCP result to ToolResult
        Ok(mcp_result_to_tool_result(result))
    }

    fn to_definition(&self) -> ToolDefinition {
        // Use prefixed name in the definition
        ToolDefinition::new(
            self.prefixed_name(),
            self.description(),
            self.parameters_schema(),
        )
    }
}

/// A wrapper that holds the prefixed name for proper &str return
pub struct PrefixedMcpTool {
    inner: McpTool,
    prefixed_name: String,
}

impl PrefixedMcpTool {
    pub fn new(
        server_name: impl Into<String>,
        tool_info: McpToolInfo,
        client: Arc<Mutex<McpClient>>,
    ) -> Self {
        let server_name = server_name.into();
        let prefixed_name = format!("{}_{}", server_name, tool_info.name);
        Self {
            inner: McpTool::new(server_name, tool_info, client),
            prefixed_name,
        }
    }

    pub fn with_security_level(mut self, level: SecurityLevel) -> Self {
        self.inner.security_level = level;
        self
    }

    pub fn server_name(&self) -> &str {
        &self.inner.server_name
    }

    pub fn original_name(&self) -> &str {
        &self.inner.tool_info.name
    }
}

#[async_trait]
impl Tool for PrefixedMcpTool {
    fn name(&self) -> &str {
        &self.prefixed_name
    }

    fn description(&self) -> &str {
        self.inner.description()
    }

    fn security_level(&self) -> SecurityLevel {
        self.inner.security_level()
    }

    fn parameters_schema(&self) -> ParameterSchema {
        self.inner.parameters_schema()
    }

    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult> {
        self.inner.execute(args, ctx).await
    }

    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::new(
            &self.prefixed_name,
            self.description(),
            self.parameters_schema(),
        )
    }
}

/// Convert MCP CallToolResult to quant ToolResult
fn mcp_result_to_tool_result(result: CallToolResult) -> ToolResult {
    // Combine all text content
    let mut output_parts = Vec::new();

    for content in &result.content {
        match content.content_type.as_str() {
            "text" => {
                if let Some(text) = &content.text {
                    output_parts.push(text.clone());
                }
            }
            "image" => {
                // For images, we can only describe them
                output_parts.push("[Image data]".to_string());
            }
            "resource" => {
                // Resource reference
                if let Some(text) = &content.text {
                    output_parts.push(format!("[Resource: {}]", text));
                }
            }
            _ => {
                // Unknown content type
                if let Some(text) = &content.text {
                    output_parts.push(text.clone());
                }
            }
        }
    }

    let output = output_parts.join("\n");

    if result.is_error {
        ToolResult::error(output)
    } else {
        ToolResult::success(output)
    }
}

/// Parse security level from string
pub fn parse_security_level(s: &str) -> Option<SecurityLevel> {
    match s.to_lowercase().as_str() {
        "safe" => Some(SecurityLevel::Safe),
        "moderate" => Some(SecurityLevel::Moderate),
        "dangerous" => Some(SecurityLevel::Dangerous),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_convert_json_schema_property() {
        let schema = serde_json::json!({
            "type": "string",
            "description": "A test parameter",
            "enum": ["a", "b", "c"],
            "default": "a"
        });

        let prop = convert_json_schema_property(&schema);
        assert_eq!(prop.param_type, "string");
        assert_eq!(prop.description, "A test parameter");
        assert_eq!(prop.enum_values, Some(vec!["a".to_string(), "b".to_string(), "c".to_string()]));
        assert_eq!(prop.default, Some(serde_json::json!("a")));
    }

    #[test]
    fn test_mcp_result_to_tool_result() {
        let mcp_result = CallToolResult {
            content: vec![
                super::super::client::ToolResultContent {
                    content_type: "text".to_string(),
                    text: Some("Hello, world!".to_string()),
                    data: None,
                    mime_type: None,
                }
            ],
            is_error: false,
        };

        let result = mcp_result_to_tool_result(mcp_result);
        assert!(result.success);
        assert_eq!(result.output, "Hello, world!");
    }

    #[test]
    fn test_parse_security_level() {
        assert_eq!(parse_security_level("safe"), Some(SecurityLevel::Safe));
        assert_eq!(parse_security_level("MODERATE"), Some(SecurityLevel::Moderate));
        assert_eq!(parse_security_level("Dangerous"), Some(SecurityLevel::Dangerous));
        assert_eq!(parse_security_level("unknown"), None);
    }
}
