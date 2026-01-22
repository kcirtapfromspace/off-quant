//! Tool registry for managing available tools

use std::collections::HashMap;
use std::sync::Arc;

use super::{Tool, ToolDefinition};

/// Registry of available tools
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    /// Create a new empty registry
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    /// Register a tool
    pub fn register<T: Tool + 'static>(&mut self, tool: T) {
        let name = tool.name().to_string();
        self.tools.insert(name, Arc::new(tool));
    }

    /// Get a tool by name
    pub fn get(&self, name: &str) -> Option<Arc<dyn Tool>> {
        self.tools.get(name).cloned()
    }

    /// List all registered tool names
    pub fn list_names(&self) -> Vec<&str> {
        self.tools.keys().map(|s| s.as_str()).collect()
    }

    /// Get all tools
    pub fn all_tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.values().cloned().collect()
    }

    /// Get tool definitions for the LLM API
    pub fn tool_definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.to_definition()).collect()
    }

    /// Number of registered tools
    pub fn len(&self) -> usize {
        self.tools.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl std::fmt::Debug for ToolRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRegistry")
            .field("tools", &self.list_names())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{ParameterSchema, SecurityLevel, ToolContext, ToolResult};
    use anyhow::Result;
    use async_trait::async_trait;
    use serde_json::Value;

    struct MockTool;

    #[async_trait]
    impl Tool for MockTool {
        fn name(&self) -> &str {
            "mock"
        }

        fn description(&self) -> &str {
            "A mock tool for testing"
        }

        fn security_level(&self) -> SecurityLevel {
            SecurityLevel::Safe
        }

        fn parameters_schema(&self) -> ParameterSchema {
            ParameterSchema::new()
        }

        async fn execute(&self, _args: &Value, _ctx: &ToolContext) -> Result<ToolResult> {
            Ok(ToolResult::success("mock output"))
        }
    }

    #[test]
    fn test_registry_register_and_get() {
        let mut registry = ToolRegistry::new();
        registry.register(MockTool);

        assert_eq!(registry.len(), 1);
        assert!(registry.get("mock").is_some());
        assert!(registry.get("nonexistent").is_none());
    }

    #[test]
    fn test_registry_list_names() {
        let mut registry = ToolRegistry::new();
        registry.register(MockTool);

        let names = registry.list_names();
        assert!(names.contains(&"mock"));
    }

    #[test]
    fn test_registry_tool_definitions() {
        let mut registry = ToolRegistry::new();
        registry.register(MockTool);

        let defs = registry.tool_definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].function.name, "mock");
    }
}
