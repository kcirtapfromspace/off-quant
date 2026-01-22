//! Tool routing and dispatch

use std::sync::Arc;

use anyhow::{bail, Result};
use tracing::{debug, info, instrument, warn};

use super::registry::ToolRegistry;
use super::security::{ConfirmationHandler, ConfirmationResult};
use super::{SecurityLevel, ToolCall, ToolContext, ToolResult};

/// Result of routing a tool call
#[derive(Debug)]
pub enum RouteResult {
    /// Tool executed successfully
    Success(ToolResult),
    /// Tool execution was skipped by user
    Skipped,
    /// Tool execution was denied by user
    Denied,
    /// Operation was aborted by user
    Aborted,
    /// Tool not found
    NotFound(String),
    /// Error during execution
    Error(String),
}

/// Router for dispatching tool calls
pub struct ToolRouter {
    registry: ToolRegistry,
    confirmation: Arc<dyn ConfirmationHandler>,
}

impl ToolRouter {
    /// Create a new router with the given registry and confirmation handler
    pub fn new(registry: ToolRegistry, confirmation: impl ConfirmationHandler + 'static) -> Self {
        Self {
            registry,
            confirmation: Arc::new(confirmation),
        }
    }

    /// Route a single tool call
    #[instrument(skip(self, ctx), fields(tool = %tool_call.name))]
    pub async fn route(&self, tool_call: &ToolCall, ctx: &ToolContext) -> RouteResult {
        // Look up the tool
        let tool = match self.registry.get(&tool_call.name) {
            Some(t) => t,
            None => {
                warn!(tool = %tool_call.name, "Tool not found");
                return RouteResult::NotFound(tool_call.name.clone());
            }
        };

        let security_level = tool.security_level();
        debug!(security_level = %security_level, "Tool security level");

        // Check if confirmation is needed
        let needs_confirmation = match security_level {
            SecurityLevel::Safe => false,
            SecurityLevel::Moderate => !ctx.auto_mode,
            SecurityLevel::Dangerous => !ctx.auto_mode,
        };

        if needs_confirmation {
            debug!("Requesting user confirmation");
            match self.confirmation.confirm(tool_call, security_level).await {
                ConfirmationResult::Approved => {
                    debug!("User approved tool execution");
                }
                ConfirmationResult::Denied => {
                    info!(tool = %tool_call.name, "User denied tool execution");
                    return RouteResult::Denied;
                }
                ConfirmationResult::Skip => {
                    info!(tool = %tool_call.name, "User skipped tool execution");
                    return RouteResult::Skipped;
                }
                ConfirmationResult::Abort => {
                    info!(tool = %tool_call.name, "User aborted operation");
                    return RouteResult::Aborted;
                }
            }
        }

        // Execute the tool (pass by reference to avoid cloning)
        info!(tool = %tool_call.name, "Executing tool");
        match tool.execute(&tool_call.arguments, ctx).await {
            Ok(result) => {
                if result.success {
                    info!(tool = %tool_call.name, output_len = result.output.len(), "Tool executed successfully");
                } else {
                    warn!(tool = %tool_call.name, error = ?result.error, "Tool execution failed");
                }
                RouteResult::Success(result)
            }
            Err(e) => {
                warn!(tool = %tool_call.name, error = %e, "Tool execution error");
                RouteResult::Error(e.to_string())
            }
        }
    }

    /// Route multiple tool calls sequentially
    pub async fn route_all(&self, tool_calls: &[ToolCall], ctx: &ToolContext) -> Vec<(String, RouteResult)> {
        let mut results = Vec::new();

        for call in tool_calls {
            let result = self.route(call, ctx).await;
            let name = call.name.clone();

            // Check for abort
            if matches!(result, RouteResult::Aborted) {
                results.push((name, result));
                break;
            }

            results.push((name, result));
        }

        results
    }

    /// Get a reference to the registry
    pub fn registry(&self) -> &ToolRegistry {
        &self.registry
    }

    /// Execute a tool call directly, returning an error for failures
    pub async fn execute(&self, tool_call: &ToolCall, ctx: &ToolContext) -> Result<ToolResult> {
        match self.route(tool_call, ctx).await {
            RouteResult::Success(result) => Ok(result),
            RouteResult::Skipped => bail!("Tool execution was skipped"),
            RouteResult::Denied => bail!("Tool execution was denied"),
            RouteResult::Aborted => bail!("Operation was aborted"),
            RouteResult::NotFound(name) => bail!("Tool not found: {}", name),
            RouteResult::Error(e) => bail!("Tool execution error: {}", e),
        }
    }
}

impl std::fmt::Debug for ToolRouter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolRouter")
            .field("registry", &self.registry)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::security::AutoApprove;
    use crate::tools::{ParameterSchema, Tool};
    use async_trait::async_trait;
    use serde_json::json;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn name(&self) -> &str {
            "echo"
        }

        fn description(&self) -> &str {
            "Echoes input"
        }

        fn security_level(&self) -> SecurityLevel {
            SecurityLevel::Safe
        }

        fn parameters_schema(&self) -> ParameterSchema {
            ParameterSchema::new()
        }

        async fn execute(&self, args: &serde_json::Value, _ctx: &ToolContext) -> Result<ToolResult> {
            let text = args.get("text").and_then(|v| v.as_str()).unwrap_or("empty");
            Ok(ToolResult::success(text))
        }
    }

    #[tokio::test]
    async fn test_router_execute() {
        let mut registry = ToolRegistry::new();
        registry.register(EchoTool);

        let router = ToolRouter::new(registry, AutoApprove);
        let ctx = ToolContext::default();

        let call = ToolCall {
            name: "echo".to_string(),
            arguments: json!({"text": "hello"}),
        };

        let result = router.execute(&call, &ctx).await.unwrap();
        assert!(result.success);
        assert_eq!(result.output, "hello");
    }

    #[tokio::test]
    async fn test_router_not_found() {
        let registry = ToolRegistry::new();
        let router = ToolRouter::new(registry, AutoApprove);
        let ctx = ToolContext::default();

        let call = ToolCall {
            name: "nonexistent".to_string(),
            arguments: json!({}),
        };

        let result = router.route(&call, &ctx).await;
        assert!(matches!(result, RouteResult::NotFound(_)));
    }
}
