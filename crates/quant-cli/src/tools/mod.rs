//! Tool framework for agent-based execution
//!
//! Provides Claude Code-like tool/function calling capabilities.

pub mod builtin;
pub mod registry;
pub mod router;
pub mod security;

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

/// Security classification for tools
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityLevel {
    /// Read-only operations, no confirmation needed
    Safe,
    /// Network operations, optional confirmation
    Moderate,
    /// Write/execute operations, always confirm unless auto mode
    Dangerous,
}

impl std::fmt::Display for SecurityLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SecurityLevel::Safe => write!(f, "safe"),
            SecurityLevel::Moderate => write!(f, "moderate"),
            SecurityLevel::Dangerous => write!(f, "dangerous"),
        }
    }
}

/// Result of tool execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolResult {
    /// Whether the tool executed successfully
    pub success: bool,
    /// Output from the tool
    pub output: String,
    /// Error message if failed
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl ToolResult {
    /// Create a successful result
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            success: true,
            output: output.into(),
            error: None,
        }
    }

    /// Create a failed result
    pub fn error(error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: String::new(),
            error: Some(error.into()),
        }
    }

    /// Create a failed result with output
    pub fn failure(output: impl Into<String>, error: impl Into<String>) -> Self {
        Self {
            success: false,
            output: output.into(),
            error: Some(error.into()),
        }
    }
}

/// Context provided to tools during execution
#[derive(Debug, Clone)]
pub struct ToolContext {
    /// Current working directory
    pub working_dir: PathBuf,
    /// Whether running in auto mode (skip confirmations)
    pub auto_mode: bool,
    /// Maximum output length (truncate if exceeded)
    pub max_output_len: usize,
    /// Default timeout for command execution (bash) in seconds
    pub command_timeout_secs: u64,
    /// Default timeout for HTTP requests in seconds
    pub http_timeout_secs: u64,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            working_dir: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            auto_mode: false,
            max_output_len: 50000,
            command_timeout_secs: 120,
            http_timeout_secs: 30,
        }
    }
}

impl ToolContext {
    /// Create a new context with the given working directory
    pub fn new(working_dir: PathBuf) -> Self {
        Self {
            working_dir,
            ..Default::default()
        }
    }

    /// Set auto mode
    pub fn with_auto_mode(mut self, auto: bool) -> Self {
        self.auto_mode = auto;
        self
    }

    /// Set command timeout
    pub fn with_command_timeout(mut self, secs: u64) -> Self {
        self.command_timeout_secs = secs;
        self
    }

    /// Set HTTP timeout
    pub fn with_http_timeout(mut self, secs: u64) -> Self {
        self.http_timeout_secs = secs;
        self
    }
}

/// Schema for a tool parameter
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterProperty {
    /// Parameter type (string, number, boolean, array, object)
    #[serde(rename = "type")]
    pub param_type: String,
    /// Parameter description
    pub description: String,
    /// Enum values if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enum_values: Option<Vec<String>>,
    /// Default value if applicable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default: Option<Value>,
}

impl ParameterProperty {
    pub fn string(description: impl Into<String>) -> Self {
        Self {
            param_type: "string".to_string(),
            description: description.into(),
            enum_values: None,
            default: None,
        }
    }

    pub fn number(description: impl Into<String>) -> Self {
        Self {
            param_type: "number".to_string(),
            description: description.into(),
            enum_values: None,
            default: None,
        }
    }

    pub fn boolean(description: impl Into<String>) -> Self {
        Self {
            param_type: "boolean".to_string(),
            description: description.into(),
            enum_values: None,
            default: None,
        }
    }

    pub fn array(description: impl Into<String>) -> Self {
        Self {
            param_type: "array".to_string(),
            description: description.into(),
            enum_values: None,
            default: None,
        }
    }

    pub fn object(description: impl Into<String>) -> Self {
        Self {
            param_type: "object".to_string(),
            description: description.into(),
            enum_values: None,
            default: None,
        }
    }

    pub fn with_default(mut self, value: Value) -> Self {
        self.default = Some(value);
        self
    }

    pub fn with_enum(mut self, values: Vec<String>) -> Self {
        self.enum_values = Some(values);
        self
    }
}

/// Schema describing tool parameters
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterSchema {
    /// Type is always "object"
    #[serde(rename = "type")]
    pub schema_type: String,
    /// Parameter properties
    pub properties: std::collections::HashMap<String, ParameterProperty>,
    /// Required parameter names
    #[serde(default)]
    pub required: Vec<String>,
}

impl ParameterSchema {
    pub fn new() -> Self {
        Self {
            schema_type: "object".to_string(),
            properties: std::collections::HashMap::new(),
            required: Vec::new(),
        }
    }

    pub fn with_property(mut self, name: impl Into<String>, prop: ParameterProperty) -> Self {
        self.properties.insert(name.into(), prop);
        self
    }

    pub fn with_required(mut self, name: impl Into<String>, prop: ParameterProperty) -> Self {
        let name = name.into();
        self.properties.insert(name.clone(), prop);
        self.required.push(name);
        self
    }
}

impl Default for ParameterSchema {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool definition for Ollama API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    /// Tool type (always "function")
    #[serde(rename = "type")]
    pub tool_type: String,
    /// Function definition
    pub function: FunctionDefinition,
}

/// Function definition within a tool
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FunctionDefinition {
    /// Function name
    pub name: String,
    /// Function description
    pub description: String,
    /// Parameter schema
    pub parameters: ParameterSchema,
}

impl ToolDefinition {
    pub fn new(name: impl Into<String>, description: impl Into<String>, parameters: ParameterSchema) -> Self {
        Self {
            tool_type: "function".to_string(),
            function: FunctionDefinition {
                name: name.into(),
                description: description.into(),
                parameters,
            },
        }
    }
}

/// A tool call from the LLM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    /// Tool/function name
    pub name: String,
    /// Arguments as JSON
    pub arguments: Value,
}

/// The Tool trait that all tools must implement
#[async_trait]
pub trait Tool: Send + Sync {
    /// Get the tool name
    fn name(&self) -> &str;

    /// Get a description of what the tool does
    fn description(&self) -> &str;

    /// Get the security level
    fn security_level(&self) -> SecurityLevel;

    /// Get the parameter schema
    fn parameters_schema(&self) -> ParameterSchema;

    /// Execute the tool with the given arguments
    async fn execute(&self, args: &Value, ctx: &ToolContext) -> Result<ToolResult>;

    /// Convert to a tool definition for the LLM
    fn to_definition(&self) -> ToolDefinition {
        ToolDefinition::new(self.name(), self.description(), self.parameters_schema())
    }
}
