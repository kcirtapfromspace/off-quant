# quant - Unified CLI for Local LLMs

A Claude Code-like experience for local LLMs via Ollama, with zero API costs and full privacy.

## Project Architecture

- `crates/llm-core/` - Core Ollama client library with retry logic and streaming
- `crates/quant-cli/` - Main CLI application with REPL, agent mode, and tools
- `crates/ollama-bar/` - macOS menu bar app (separate from CLI)

## Instructions

- Use async/await and tokio for all async code
- Follow Rust idioms and clippy recommendations
- Add tracing spans for observability on important operations
- Write tests for new functionality
- Keep error messages user-friendly
- Tools should implement the `Tool` trait from `crates/quant-cli/src/tools/mod.rs`
- Security levels: Safe (read-only), Moderate (network), Dangerous (write/execute)

## Code Style

- Prefer explicit error handling with `anyhow::Result`
- Use `tracing::{debug, info, warn}` for logging
- Keep functions focused and under 50 lines when possible
- Document public APIs with rustdoc comments

## Testing

Run all tests with: `cargo test --workspace`
Run specific crate: `cargo test --package quant-cli`

## Key Files

- `crates/quant-cli/src/agent/agent_loop.rs` - Main agent orchestration
- `crates/quant-cli/src/tools/` - Tool implementations
- `crates/llm-core/src/ollama.rs` - Ollama API client
- `llm.toml` - Configuration file format
