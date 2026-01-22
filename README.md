# off-quant

Local LLM tooling for coding and API access on macOS. Features a Claude Code-like CLI experience, native menu bar app, and comprehensive model management.

![CI](https://github.com/kcirtapfromspace/off-quant/actions/workflows/ci.yml/badge.svg)

## Architecture

```
┌─────────────────────────────────────────────────────────────────────────┐
│                              macOS Host                                  │
│                                                                          │
│  ┌───────────────────────────────────────────────────────────────────┐  │
│  │  CLI Tools                                                         │  │
│  │  ├── quant          Claude Code-like REPL with streaming          │  │
│  │  ├── quant ask      One-shot queries with context injection       │  │
│  │  ├── quant status   Ollama status and system info                 │  │
│  │  └── off-quant      Direct llama.cpp/EXO inference                │  │
│  └───────────────────────────────────────────────────────────────────┘  │
│                                      │                                   │
│  ┌──────────────────┐    ┌───────────▼───────────┐    ┌──────────────┐  │
│  │  OllamaBar       │    │   Ollama (native)     │    │  llm-core    │  │
│  │  - Menu bar app  │◄──►│   - Metal GPU accel   │◄──►│  - API client│  │
│  │  - Model switch  │    │   - :11434            │    │  - Streaming │  │
│  │  - Tailscale     │    │   - Model management  │    │  - Config    │  │
│  └──────────────────┘    └───────────────────────┘    └──────────────┘  │
│                                      │                                   │
│  ┌───────────────────────────────────▼───────────────────────────────┐  │
│  │  Docker (Optional)                                                 │  │
│  │  ├── Caddy (auth proxy :8080)                                     │  │
│  │  └── Aider (coding assistant)                                     │  │
│  └───────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────┘
```

**Why native Ollama?** Docker on macOS cannot access Metal GPU. Running Ollama natively gives you hardware acceleration, which is significantly faster.

## Quick Start

```bash
# Install dependencies
brew install just ollama

# Clone and build
git clone https://github.com/kcirtapfromspace/off-quant.git
cd off-quant
cargo build --release

# Start Ollama
just serve

# Start interactive chat (Claude Code-like experience)
./target/release/quant chat

# Or one-shot query
./target/release/quant ask "explain what Rust lifetimes are"
```

## The `quant` CLI

A unified CLI providing a Claude Code-like experience for local LLMs.

### Interactive Chat (REPL)

```bash
quant chat                        # Start interactive session
quant chat --model llama3.2       # Use specific model
quant chat --system "You are..."  # Set system prompt
quant chat --load my-session      # Load saved conversation
```

**Slash Commands in REPL:**
| Command | Description |
|---------|-------------|
| `/help` | Show available commands |
| `/model <name>` | Switch to different model |
| `/models` | List available models |
| `/system <prompt>` | Set system prompt |
| `/context add <path>` | Add files to context |
| `/save [name]` | Save conversation |
| `/load <name>` | Load conversation |
| `/clear` | Clear conversation history |
| `/exit` | Exit REPL |

### One-Shot Queries

```bash
quant ask "explain this code"                    # Simple query
quant ask --stdin < file.rs                      # Pipe input
quant ask -c ./src "review this code"            # With context
quant ask --json "list all functions"            # JSON output
quant ask -t 0.2 "be precise"                    # Set temperature
```

### Model Management

```bash
quant status                      # Show Ollama status
quant models list                 # List available models
quant models pull llama3.2        # Pull a model
quant models rm old-model         # Remove a model
quant models ps                   # Show loaded models
quant run --model llama3.2        # Warm up a model
```

### Service Control

```bash
quant serve start                 # Start Ollama
quant serve stop                  # Stop Ollama
quant serve restart               # Restart Ollama
quant health --timeout 60         # Health check with retry
```

### Context Management (RAG)

```bash
quant context add ./src           # Add directory to context
quant context add file.rs         # Add specific file
quant context list                # List tracked files
quant context rm ./src            # Remove from context
quant context clear               # Clear all context
```

### Configuration

```bash
quant config init                 # Create default config
quant config show                 # Show current config
quant config path                 # Print config file path
quant config edit                 # Open in $EDITOR
```

Config file: `~/.config/quant/config.toml`

```toml
[repl]
default_model = "llama3.2"
system_prompt = "You are a helpful coding assistant."
auto_save = true
history_size = 1000

[ask]
temperature = 0.7
max_tokens = 4096

[aliases.models]
code = "deepseek-coder:6.7b"
chat = "llama3.2"
```

## OllamaBar Menu Bar App

A native macOS menu bar app for managing Ollama with one-click controls.

### Features
- One-click start/stop/restart Ollama
- Switch between models from menu bar
- Pull new models with progress dialog
- Tailscale network sharing toggle
- Memory usage monitoring
- Auto-start with last used model

### Install

**From Release:**
Download the latest DMG from [Releases](https://github.com/kcirtapfromspace/off-quant/releases).

**Build from Source:**
```bash
just build-app      # Build
just run-app        # Run
just install-app    # Install to /Applications
just bundle-app     # Create DMG
```

## Project Structure

```
off-quant/
├── crates/
│   ├── llm-core/        # Shared library: Ollama client, config, streaming
│   ├── quant-cli/       # Unified CLI (quant command)
│   ├── ollama-bar/      # macOS menu bar app
│   └── off-quant-cli/   # Direct llama.cpp/EXO wrapper
├── scripts/
│   └── llm_ctl.py       # Legacy Python CLI
├── modelfiles/          # Ollama Modelfiles for GGUF imports
├── llm.toml             # Main configuration
└── justfile             # Command runner
```

## Configuration

### llm.toml (Main Config)

```toml
[ollama]
host = "127.0.0.1"
port = 11434
models_path = "/Volumes/models"
ollama_home = "/Volumes/models/ollama"

[models]
coding = "local/qwen2.5-coder-7b-q4km"
chat = "local/glm-4-9b-chat-q4k"

[models.auto_select]
threshold_high = 64    # >= 64GB RAM: qwen2.5-coder-7b
threshold_medium = 32  # >= 32GB RAM: deepseek-coder-6.7b
                       # < 32GB RAM: starcoder2-7b
```

### Model Storage

Place GGUF files in `/Volumes/models/`:

```
/Volumes/models/
├── ollama/                    # Ollama's data directory
├── qwen2.5-coder-7b-instruct-q4_k_m.gguf
├── deepseek-coder-6.7b-instruct.Q4_K_M.gguf
└── glm-4-9b-chat.Q4_K.gguf
```

## Justfile Commands

Run `just` to see all commands. Key ones:

| Command | Description |
|---------|-------------|
| `just serve` | Start Ollama (foreground) |
| `just status` | Show Ollama status |
| `just import` | Import local GGUF files |
| `just aider` | Start Aider coding session |
| `just build-app` | Build OllamaBar |
| `just run-app` | Run OllamaBar |
| `just service-install` | Install as launchd service |

## Network Access

Expose Ollama to your network with authentication:

```bash
just setup-auth       # Set up credentials
just proxy-up         # Start Caddy proxy
```

API endpoint: `http://your-ip:8080/api/...`

## Running as a Service

```bash
just service-install  # Install launchd agent
just logs             # View logs
```

The service:
- Starts Ollama on login
- Restarts on crash
- Waits for `/Volumes/models` mount

## CI/CD

### Workflows

- **CI** (`ci.yml`): Format, lint, build, test on PRs and main
- **Release Please** (`release-please.yml`): Automated versioning via Conventional Commits
- **Release** (`release.yml`): Build, sign, notarize, publish DMG

### Creating a Release

```bash
git commit -m "feat: add new feature"    # Triggers minor bump
git commit -m "fix: resolve bug"         # Triggers patch bump
```

Release Please creates a PR with version bump. Merge to trigger release.

### Code Signing (Optional)

Add these secrets for signed releases:

| Secret | Description |
|--------|-------------|
| `MACOS_CERTIFICATE` | Base64-encoded .p12 certificate |
| `MACOS_CERTIFICATE_PASSWORD` | Password for .p12 |
| `SIGNING_IDENTITY` | Developer ID Application: Name (TEAMID) |
| `APPLE_ID` | Apple ID email |
| `APPLE_TEAM_ID` | Developer Team ID |
| `APPLE_APP_PASSWORD` | App-specific password |

## Development

```bash
# Build all crates
cargo build --workspace

# Run tests
cargo test --workspace

# Check formatting
cargo fmt --check

# Run lints
cargo clippy --workspace
```

## Troubleshooting

**Ollama not starting:**
```bash
just logs  # Check error logs
```

**Models volume not mounted:**
```bash
just check-volume
```

**Port already in use:**
```bash
lsof -i :11434
```

**quant can't connect:**
```bash
quant health --timeout 30  # Check with retries
quant status               # View detailed status
```

## License

MIT
