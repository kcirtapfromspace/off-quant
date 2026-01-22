# off-quant: Local LLM tooling
# Run `just` to see all available commands

set dotenv-load := true

# Default recipe - show help
default:
    @just --list

# === Setup ===

# Install Ollama (macOS)
install-ollama:
    @echo "Installing Ollama..."
    brew install ollama

# Install just (macOS)
install-just:
    brew install just

# Install Python dependencies
install-deps:
    pip install tomli  # For Python < 3.11

# Full setup
setup: install-ollama install-deps
    @echo "Setup complete!"
    @echo "Next steps:"
    @echo "  1. Mount your models volume at /Volumes/models"
    @echo "  2. Run: just serve"
    @echo "  3. Run: just import"

# === Ollama Service ===

# Start Ollama (foreground)
serve:
    python3 scripts/llm_ctl.py serve

# Start Ollama as launchd service
service-install:
    @echo "Installing launchd service..."
    cp com.ollama.server.plist ~/Library/LaunchAgents/
    launchctl load ~/Library/LaunchAgents/com.ollama.server.plist
    @echo "Ollama will now start automatically"

# Stop and remove launchd service
service-uninstall:
    launchctl unload ~/Library/LaunchAgents/com.ollama.server.plist || true
    rm -f ~/Library/LaunchAgents/com.ollama.server.plist
    @echo "Service removed"

# Restart launchd service
service-restart:
    launchctl kickstart -k gui/$(id -u)/com.ollama.server

# View Ollama logs
logs:
    tail -f /tmp/ollama.out.log /tmp/ollama.err.log

# === Model Management ===

# Show status
status:
    python3 scripts/llm_ctl.py status

# Health check (for scripts)
health timeout="60":
    python3 scripts/llm_ctl.py health -t {{timeout}}

# List all models
models:
    python3 scripts/llm_ctl.py models

# Import local GGUF files
import:
    python3 scripts/llm_ctl.py import

# Pull a model from registry
pull model:
    python3 scripts/llm_ctl.py pull {{model}}

# Auto-select model based on RAM
select:
    python3 scripts/llm_ctl.py select

# Generate .env.local
env:
    python3 scripts/llm_ctl.py env

# === Network Proxy ===

# Start Caddy proxy (exposes Ollama to network)
proxy-up:
    docker compose -f docker-compose.proxy.yml up -d caddy
    @echo "Ollama API available at http://localhost:8080"
    @echo "Health check at http://localhost:8081/health"

# Stop Caddy proxy
proxy-down:
    docker compose -f docker-compose.proxy.yml down

# View proxy logs
proxy-logs:
    docker compose -f docker-compose.proxy.yml logs -f caddy

# Setup auth password for proxy
setup-auth:
    @echo "Enter password for API access:"
    @docker run --rm caddy:2-alpine caddy hash-password --plaintext
    @echo ""
    @echo "Copy the hash above and update Caddyfile"

# === Aider ===

# Generate env and start Aider
aider: env
    docker compose -f docker-compose.proxy.yml run --rm aider

# Start Aider (native, if installed)
aider-native: env
    aider --model ollama/$(grep OLLAMA_MODEL .env.local | cut -d= -f2)

# === OllamaBar App ===

# Build the menu bar app
build-app:
    cargo build --release -p ollama-bar
    @echo "Built: target/release/ollama-bar ($(ls -lh target/release/ollama-bar | awk '{print $5}'))"

# Run the menu bar app
run-app:
    cargo run --release -p ollama-bar

# Bundle the app (creates .app and .dmg in dist/)
bundle-app:
    ./scripts/bundle-app.sh

# Bundle and sign (for distribution)
bundle-app-signed identity:
    ./scripts/bundle-app.sh --sign "{{identity}}"

# Bundle, sign, and notarize (for App Store/distribution)
bundle-app-notarized identity:
    ./scripts/bundle-app.sh --sign "{{identity}}" --notarize

# Install the menu bar app to /Applications
install-app: build-app
    @mkdir -p /Applications/OllamaBar.app/Contents/MacOS
    @mkdir -p /Applications/OllamaBar.app/Contents/Resources
    @cp target/release/ollama-bar /Applications/OllamaBar.app/Contents/MacOS/
    @cp crates/ollama-bar/assets/Info.plist /Applications/OllamaBar.app/Contents/
    @test -f crates/ollama-bar/assets/AppIcon.icns && cp crates/ollama-bar/assets/AppIcon.icns /Applications/OllamaBar.app/Contents/Resources/ || true
    @echo "Installed to /Applications/OllamaBar.app"

# Uninstall the menu bar app
uninstall-app:
    rm -rf /Applications/OllamaBar.app
    @echo "Uninstalled OllamaBar"

# Setup notarization credentials (run once)
setup-notarize:
    @echo "Setting up notarization credentials..."
    @echo "You'll need your Apple ID and an app-specific password from appleid.apple.com"
    xcrun notarytool store-credentials "notarytool-profile" --apple-id "" --team-id ""
    @echo "Credentials stored. Use 'just bundle-app-notarized' to build and notarize."

# === Development ===

# Run with old Tilt workflow (deprecated)
tilt:
    tilt up

# Check models volume
check-volume:
    @if [ -d "/Volumes/models" ]; then \
        echo "✓ /Volumes/models is mounted"; \
        ls -la /Volumes/models/*.gguf 2>/dev/null || echo "  No GGUF files found"; \
    else \
        echo "✗ /Volumes/models is not mounted"; \
    fi

# Quick start (serve + wait + import)
quick-start:
    @echo "Starting Ollama..."
    @just serve &
    @sleep 2
    @just health
    @just import
    @echo "Ready! Run 'just aider' to start coding"
