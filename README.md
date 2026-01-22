# off-quant

Local LLM tooling for coding and API access on macOS.

## Architecture

```
┌────────────────────────────────────────────────────────────┐
│                      macOS Host                            │
│                                                            │
│  ┌──────────────────┐    ┌─────────────────────────────┐  │
│  │  Ollama (native) │◄───│  llm-ctl (Python CLI)       │  │
│  │  - Metal GPU     │    │  - model management         │  │
│  │  - launchd svc   │    │  - health checks            │  │
│  │  :11434          │    │  - GGUF imports             │  │
│  └────────┬─────────┘    └─────────────────────────────┘  │
│           │                                                │
│  ┌────────▼─────────────────────────────────────────────┐ │
│  │                    Docker                             │ │
│  │  ┌─────────────┐     ┌─────────────┐                 │ │
│  │  │   Caddy     │     │   Aider     │                 │ │
│  │  │  - Auth     │     │  (manual)   │                 │ │
│  │  │  :8080      │     └─────────────┘                 │ │
│  │  └─────────────┘                                     │ │
│  └──────────────────────────────────────────────────────┘ │
└────────────────────────────────────────────────────────────┘
         │
  Network → :8080 (authenticated OpenAI-compatible API)
```

**Why native Ollama?** Docker on macOS cannot access Metal GPU. Running Ollama natively gives you hardware acceleration, which is significantly faster.

## Quick Start

```bash
# Install dependencies
brew install just ollama

# Start Ollama
just serve

# In another terminal: import your GGUF models
just import

# Start coding with Aider
just aider
```

## Commands

Run `just` to see all commands. Key ones:

| Command | Description |
|---------|-------------|
| `just serve` | Start Ollama (foreground) |
| `just service-install` | Install as launchd service (auto-start) |
| `just status` | Show Ollama status and models |
| `just import` | Import local GGUF files |
| `just aider` | Start Aider coding session |
| `just proxy-up` | Expose API to network with auth |

## Model Storage

Put GGUF files in `/Volumes/models/`:

```
/Volumes/models/
├── ollama/                    # Ollama's data directory
├── qwen2.5-coder-7b-instruct-q4_k_m.gguf
├── deepseek-coder-6.7b-instruct.Q4_K_M.gguf
├── starcoder2-7b-Q4_K_M.gguf
└── glm-4-9b-chat.Q4_K.gguf
```

## Network Access

To expose Ollama to your network:

```bash
# Set up authentication
just setup-auth
# Copy the hash and edit Caddyfile

# Start the proxy
just proxy-up
```

API endpoint: `http://your-ip:8080/api/...`

Requires basic auth with credentials set in `Caddyfile`.

## Configuration

All settings in `llm.toml`:

```toml
[ollama]
host = "127.0.0.1"
port = 11434
models_path = "/Volumes/models"

[models]
coding = "local/qwen2.5-coder-7b-q4km"
chat = "local/glm-4-9b-chat-q4k"
```

## Model Selection

Auto-selected based on RAM:
- >= 64 GB: qwen2.5-coder-7b
- >= 32 GB: deepseek-coder-6.7b
- < 32 GB: starcoder2-7b

Override in `llm.toml` or `.env.local`.

## Running as a Service

To have Ollama start automatically:

```bash
just service-install
```

This installs a launchd agent that:
- Starts Ollama on login
- Restarts on crash
- Waits for `/Volumes/models` to be mounted

View logs: `just logs`

## Legacy Tilt Workflow

The original Tilt-based workflow is still available:

```bash
just tilt
```

But the new native approach is recommended for better performance.

## Files

| File | Purpose |
|------|---------|
| `llm.toml` | Main configuration |
| `justfile` | Command runner |
| `scripts/llm_ctl.py` | CLI for model management |
| `com.ollama.server.plist` | launchd service definition |
| `Caddyfile` | Reverse proxy config |
| `docker-compose.proxy.yml` | Docker services |
| `modelfiles/` | Ollama Modelfiles for GGUF imports |

## OllamaBar Menu Bar App

A native macOS menu bar app for managing Ollama.

![CI](https://github.com/kcirtapfromspace/off-quant/actions/workflows/ci.yml/badge.svg)

### Features
- One-click start/stop/restart Ollama
- Switch between models from menu bar
- Tailscale network sharing toggle
- Memory usage monitoring
- Native macOS notifications
- Pull new models with progress

### Install from Release

Download the latest DMG from [Releases](https://github.com/kcirtapfromspace/off-quant/releases).

### Build from Source

```bash
# Build
just build-app

# Run
just run-app

# Install to /Applications
just install-app

# Create DMG
just bundle-app
```

### Commands

| Command | Description |
|---------|-------------|
| `just build-app` | Build menu bar app |
| `just run-app` | Run menu bar app |
| `just install-app` | Install to /Applications |
| `just bundle-app` | Create .app bundle and DMG |
| `just bundle-app-signed ID` | Sign with Developer ID |
| `just bundle-app-notarized ID` | Sign and notarize |

## CI/CD

This project uses GitHub Actions for continuous integration and releases.

### Workflows

- **CI** (`ci.yml`): Runs on PRs and pushes to main
  - Format check, Clippy lints, build, tests
  - Uploads unsigned artifact on main branch

- **Release Please** (`release-please.yml`): Manages versioning
  - Creates release PRs based on [Conventional Commits](https://www.conventionalcommits.org/)
  - Automatically bumps version in Cargo.toml

- **Release** (`release.yml`): Builds and publishes releases
  - Triggered on version tags (v*)
  - Signs and notarizes the app (if secrets configured)
  - Creates GitHub release with DMG and ZIP

### Creating a Release

1. Use [Conventional Commits](https://www.conventionalcommits.org/):
   ```bash
   git commit -m "feat: add dark mode support"
   git commit -m "fix: resolve memory leak"
   ```

2. Release Please will create a PR with version bump

3. Merge the PR to trigger the release

### Setting Up Code Signing (Optional)

To sign and notarize releases, add these secrets to your repository:

| Secret | Description |
|--------|-------------|
| `MACOS_CERTIFICATE` | Base64-encoded .p12 certificate |
| `MACOS_CERTIFICATE_PASSWORD` | Password for .p12 file |
| `KEYCHAIN_PASSWORD` | Temporary keychain password |
| `SIGNING_IDENTITY` | e.g., "Developer ID Application: Your Name (TEAMID)" |
| `APPLE_ID` | Your Apple ID email |
| `APPLE_TEAM_ID` | Your Apple Developer Team ID |
| `APPLE_APP_PASSWORD` | App-specific password from appleid.apple.com |

**Export your certificate:**
```bash
# Export from Keychain Access, then:
base64 -i certificate.p12 | pbcopy
# Paste as MACOS_CERTIFICATE secret
```

Without these secrets, releases will be unsigned but still functional for local use.

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
lsof -i :11434  # Find what's using the port
```

**Import failing:**
```bash
just status   # Check if Ollama is running
just models   # Check if GGUF files exist
```

## License

MIT
