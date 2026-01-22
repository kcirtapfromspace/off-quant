//! Built-in tools for the agent framework

mod bash;
mod file_read;
mod file_write;
mod git;
mod glob;
mod grep;
mod multi_edit;
mod sandbox;
mod web_fetch;
mod web_search;

pub use bash::BashTool;
pub use file_read::FileReadTool;
pub use file_write::FileWriteTool;
pub use git::GitTool;
pub use glob::GlobTool;
pub use grep::GrepTool;
pub use multi_edit::MultiEditTool;
pub use sandbox::{SandboxBackend, SandboxConfig, SandboxTool};
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

    // Moderate tools (network access, git operations)
    registry.register(WebFetchTool::new());
    registry.register(WebSearchTool);
    registry.register(GitTool::new());

    // Dangerous tools (write/execute)
    registry.register(FileWriteTool);
    registry.register(MultiEditTool);
    registry.register(BashTool);
    registry.register(SandboxTool::new());

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
