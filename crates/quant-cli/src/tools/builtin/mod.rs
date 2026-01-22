//! Built-in tools for the agent framework

mod bash;
mod file_read;
mod file_write;
mod glob;
mod grep;
mod web_fetch;
mod web_search;

pub use bash::BashTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use web_fetch::WebFetchTool;
pub use web_search::WebSearchTool;

use super::registry::ToolRegistry;

/// Create a registry with all default tools
pub fn create_default_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    // Safe tools (no confirmation needed)
    registry.register(FileReadTool);
    registry.register(GlobTool);
    registry.register(GrepTool);

    // Moderate tools (network access)
    registry.register(WebFetchTool::new());
    registry.register(WebSearchTool);

    // Dangerous tools (write/execute)
    registry.register(FileWriteTool);
    registry.register(BashTool);

    registry
}

/// Create a registry with only safe tools
pub fn create_safe_registry() -> ToolRegistry {
    let mut registry = ToolRegistry::new();

    registry.register(FileReadTool);
    registry.register(GlobTool);
    registry.register(GrepTool);

    registry
}
