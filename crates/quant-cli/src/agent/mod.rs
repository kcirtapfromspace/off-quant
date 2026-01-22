//! Agent framework for autonomous task execution
//!
//! Implements an observe-think-act loop similar to Claude Code.

mod agent_loop;
mod state;

pub use agent_loop::AgentLoop;
pub use state::{AgentConfig, AgentState};
