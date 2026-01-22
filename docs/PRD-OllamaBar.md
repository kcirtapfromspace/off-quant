# PRD: OllamaBar - macOS Menu Bar App for Local LLM Management

**Author:** off-quant
**Status:** Draft
**Last Updated:** 2025-01-21

---

## 1. Customer Problem

### Who is the customer?
A developer running local LLMs on macOS for coding assistance and general AI queries. They have technical expertise but want a frictionless experience for daily use.

### What problem are we solving?
Managing local LLMs currently requires terminal commands, remembering CLI syntax, and manual process management. This creates friction that discourages use and makes it hard to share the service with other devices.

### Current Pain Points (measured)

| Task | Current Time | Current Steps | Target |
|------|--------------|---------------|--------|
| Check if Ollama is running | ~10s | Open terminal, run command | Glance at icon |
| Start Ollama + load model | ~30s | Terminal, type commands, wait | 1 click |
| Switch models | ~20s | Terminal command, wait | 2 clicks |
| Share to network | ~60s | Start proxy, configure, get IP | 1 toggle |
| View memory usage | ~15s | Activity Monitor or htop | Hover on icon |

### Success Metrics

1. **Time to first inference**: From app launch to first LLM response < 5 seconds (if model cached)
2. **Daily active usage**: User interacts with menu bar app at least 1x/day
3. **Error recovery rate**: 90% of errors can be resolved without terminal
4. **Network sharing adoption**: > 50% of sessions use Tailscale sharing

---

## 2. Solution Overview

### Product Vision
A native macOS menu bar application that provides single-click management of local LLM infrastructure with seamless Tailscale network sharing.

### Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                    OllamaBar (SwiftUI)                      │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────────────┐ │
│  │ Menu Bar UI │  │ Status      │  │ Settings Window     │ │
│  │ - Icon      │  │ Monitor     │  │ - Config editor     │ │
│  │ - Dropdown  │  │ - Polling   │  │ - Model management  │ │
│  └──────┬──────┘  └──────┬──────┘  └──────────┬──────────┘ │
│         │                │                     │            │
│  ┌──────▼─────────────────▼─────────────────────▼────────┐  │
│  │              Service Controller Layer                 │  │
│  │  - OllamaService (start/stop/health)                 │  │
│  │  - ModelService (list/load/pull)                     │  │
│  │  - TailscaleService (detect/bind)                    │  │
│  │  - DockerService (proxy containers)                  │  │
│  └───────────────────────┬──────────────────────────────┘  │
└──────────────────────────┼──────────────────────────────────┘
                           │
         ┌─────────────────┼─────────────────┐
         ▼                 ▼                 ▼
   ┌──────────┐     ┌──────────┐     ┌──────────┐
   │ Ollama   │     │Tailscale │     │ Docker   │
   │ (native) │     │ (daemon) │     │ (Caddy)  │
   └──────────┘     └──────────┘     └──────────┘
```

### Technology Choice: Native SwiftUI

**Why SwiftUI over Electron/web-based?**

| Factor | SwiftUI | Electron |
|--------|---------|----------|
| Memory footprint | ~20 MB | ~100+ MB |
| Startup time | < 0.5s | 2-3s |
| Native feel | Yes | No |
| System integration | Full (launchd, notifications) | Limited |
| Battery impact | Minimal | Noticeable |

**Decision:** Customer is running LLMs which consume significant RAM. The menu bar app must be lightweight. SwiftUI provides native integration with launchd, Notification Center, and system appearance.

---

## 3. Feature Requirements

### P0: Core Functionality (MVP)

#### 3.1 Status Indicator
**Customer value:** Instant visibility into LLM availability

- **Icon states:**
  - 🟢 Green: Ollama running, model loaded, ready
  - 🟡 Yellow: Ollama starting or model loading
  - 🔴 Red: Ollama stopped or error
  - ⚪ Gray: Volume not mounted (models unavailable)

- **Tooltip on hover:** "Ollama: Running | Model: qwen2.5-coder-7b | RAM: 8.2 GB"

**Observability:** Log state transitions with timestamps

#### 3.2 Start/Stop Ollama
**Customer value:** One-click service management

- Menu item: "Start Ollama" / "Stop Ollama" (toggles based on state)
- Start action:
  1. Check if `/Volumes/models` is mounted → show warning if not
  2. Set environment variables (OLLAMA_HOME, OLLAMA_HOST)
  3. Launch `ollama serve` as background process
  4. Poll health endpoint until ready (timeout: 30s)
  5. Update icon to green
- Stop action:
  1. Send SIGTERM to Ollama process
  2. Wait for graceful shutdown (timeout: 10s)
  3. Send SIGKILL if needed
  4. Update icon to red

**Failure modes:**
- Port 11434 in use → Show "Port in use by [process]. Kill it?"
- Crash on start → Show last 10 lines of log, offer to open full log

#### 3.3 Current Model Display
**Customer value:** Know which model is active without guessing

- Show in dropdown: "Current: local/qwen2.5-coder-7b-q4km"
- Show model size and quantization
- Show memory usage for loaded model

#### 3.4 Model Switching
**Customer value:** Quick context switch between coding and chat models

- Submenu: "Switch Model →"
  - List all available models (from Ollama API)
  - Checkmark next to currently loaded model
  - Click to switch (unloads current, loads new)
- Show loading progress in menu
- Keyboard shortcut: ⌘M to open model menu

**Performance requirement:** Model switch < 10s for cached models

### P1: Network Sharing

#### 3.5 Tailscale Integration
**Customer value:** Share LLM with other devices without complex networking

- Toggle: "Share via Tailscale" (disabled if Tailscale not running)
- When enabled:
  1. Get Tailscale IP: `tailscale ip -4`
  2. Restart Ollama with `OLLAMA_HOST=<tailscale-ip>:11434`
  3. Show copyable URL: "http://100.x.x.x:11434"
  4. Optional: Start Caddy proxy for auth layer
- When disabled:
  1. Restart Ollama with `OLLAMA_HOST=127.0.0.1:11434`

- Show Tailscale status: "Connected as macbook.tail1234.ts.net"
- Copy URL button for easy sharing

**Security consideration:** Tailscale network is authenticated. All devices on your tailnet are trusted. Basic auth is defense-in-depth, optional for personal use.

**Failure modes:**
- Tailscale not installed → Hide toggle, show "Install Tailscale" link
- Tailscale disconnected → Gray out toggle, show reconnect hint
- IP changed → Re-detect on each toggle

#### 3.6 Memory Usage Display
**Customer value:** Prevent OOM situations, informed model choices

- Show in dropdown: "Memory: 8.2 / 32 GB (25%)"
- Color coding: Green < 50%, Yellow 50-80%, Red > 80%
- Warning before loading model that would exceed 80%

### P2: Model Management

#### 3.7 Pull New Model
**Customer value:** Discover and download models without terminal

- Menu item: "Pull Model..."
- Opens small window with:
  - Text field for model name (e.g., "llama3.2:7b")
  - Popular models list (curated)
  - Download progress bar
  - Cancel button
- Background download, notification when complete

#### 3.8 Import Local GGUF
**Customer value:** Use downloaded GGUF files

- Menu item: "Import GGUF..."
- File picker filtered to .gguf files
- Auto-generate modelfile
- Import via `ollama create`

#### 3.9 View Logs
**Customer value:** Debug issues without terminal

- Menu item: "View Logs →"
  - "Ollama Logs" → Opens /tmp/ollama.out.log in Console.app
  - "App Logs" → Opens ~/Library/Logs/OllamaBar/
- Show last error inline if recent (< 5 min)

### P3: Settings & Configuration

#### 3.10 Settings Window
- Edit `llm.toml` with form UI
- Configure:
  - Models path
  - Default model for coding/chat
  - Auto-start on login
  - Tailscale auth preference

#### 3.11 Docker Container Management
- Show Caddy proxy status
- Start/stop proxy
- View proxy logs

---

## 4. User Interface Design

### Menu Bar Dropdown

```
┌─────────────────────────────────────┐
│ ● Ollama Running                    │
│   Model: qwen2.5-coder-7b-q4km     │
│   Memory: 8.2 GB / 32 GB           │
├─────────────────────────────────────┤
│ ■ Stop Ollama                    ⌘S │
│ ↻ Restart                        ⌘R │
├─────────────────────────────────────┤
│ Switch Model                      → │
│   ✓ local/qwen2.5-coder-7b-q4km    │
│     local/deepseek-coder-6.7b-q4km │
│     local/glm-4-9b-chat-q4k        │
├─────────────────────────────────────┤
│ □ Share via Tailscale               │
│   http://100.64.0.1:11434    [Copy] │
├─────────────────────────────────────┤
│ Pull Model...                    ⌘P │
│ Import GGUF...                   ⌘I │
├─────────────────────────────────────┤
│ View Logs                         → │
│ Settings...                      ⌘, │
├─────────────────────────────────────┤
│ Quit OllamaBar                   ⌘Q │
└─────────────────────────────────────┘
```

### Icon States

| State | Icon | Color |
|-------|------|-------|
| Running, ready | ◉ | Green (#34C759) |
| Starting/loading | ◉ | Yellow (#FF9500) |
| Stopped | ○ | Red (#FF3B30) |
| Volume missing | ○ | Gray (#8E8E93) |
| Shared (Tailscale) | ◉↗ | Green with arrow |

---

## 5. Technical Implementation

### 5.1 Process Management

```swift
class OllamaService {
    private var process: Process?

    func start() async throws {
        // Set environment
        let env = [
            "OLLAMA_HOME": "/Volumes/models/ollama",
            "OLLAMA_HOST": currentHost
        ]

        // Launch process
        process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/local/bin/ollama")
        process.arguments = ["serve"]
        process.environment = env

        try process.run()

        // Wait for health
        try await waitForHealth(timeout: 30)
    }

    func stop() {
        process?.terminate()
        // Wait, then SIGKILL if needed
    }
}
```

### 5.2 Health Monitoring

```swift
class StatusMonitor: ObservableObject {
    @Published var status: OllamaStatus = .unknown
    @Published var currentModel: String?
    @Published var memoryUsage: MemoryInfo?

    private var timer: Timer?

    func startMonitoring() {
        timer = Timer.scheduledTimer(withTimeInterval: 5.0, repeats: true) { _ in
            Task { await self.checkHealth() }
        }
    }

    private func checkHealth() async {
        // GET http://localhost:11434/api/tags
        // Update status, model list, memory
    }
}
```

### 5.3 Tailscale Integration

```swift
class TailscaleService {
    func getIP() async throws -> String {
        let output = try shell("tailscale ip -4")
        return output.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    func isConnected() async -> Bool {
        let status = try? shell("tailscale status --json")
        // Parse JSON, check BackendState == "Running"
    }
}
```

### 5.4 Data Persistence

- **Config:** Read/write `llm.toml` using Swift TOML library
- **State:** UserDefaults for UI preferences (window position, last model)
- **Logs:** FileHandle to `~/Library/Logs/OllamaBar/app.log`

---

## 6. Failure Modes & Mitigations

| Failure | Customer Impact | Detection | Mitigation |
|---------|-----------------|-----------|------------|
| Ollama won't start | Can't use LLMs | Health check timeout | Show log snippet, common fixes |
| OOM on model load | System slowdown | Memory threshold | Warn before load, suggest smaller model |
| Tailscale disconnected | Can't share | Status check | Gray toggle, show reconnect |
| Port conflict | Start fails | EADDRINUSE | Show process, offer to kill |
| Volume not mounted | No models | Path check | Warning icon, disable model ops |
| Process crash | Service dies | Process monitor | Auto-restart with backoff |

### Error Recovery UX

1. **Inline errors:** Show in dropdown for 5 minutes
2. **Notification:** For background errors (download failed)
3. **Log access:** Always 2 clicks away
4. **Recovery actions:** Buttons for common fixes (kill port, restart)

---

## 7. Observability & Metrics

### Logging

```
2025-01-21T10:30:45Z INFO  Starting Ollama service
2025-01-21T10:30:45Z INFO  Environment: OLLAMA_HOST=127.0.0.1:11434
2025-01-21T10:30:47Z INFO  Health check passed (2.1s)
2025-01-21T10:30:47Z INFO  Status: running, model: qwen2.5-coder-7b-q4km
```

### Metrics (local, privacy-preserving)

- Start/stop count per day
- Model switch count
- Tailscale share sessions
- Error count by type
- Average startup time

Stored in: `~/Library/Application Support/OllamaBar/metrics.json`

---

## 8. Security Considerations

1. **No credentials in logs:** Redact any auth tokens
2. **Tailscale trust model:** Document that tailnet = trusted
3. **Optional basic auth:** For defense-in-depth when sharing
4. **No auto-update without consent:** Manual update check
5. **Sandboxing:** Request only necessary entitlements

---

## 9. Launch Plan

### Phase 1: MVP (P0)
- Status indicator
- Start/stop
- Model display and switching
- Basic error handling

### Phase 2: Network (P1)
- Tailscale integration
- Memory monitoring
- Model pull with progress

### Phase 3: Polish (P2)
- Settings UI
- GGUF import
- Docker container management

### Phase 4: Future (P3)
- Model presets/profiles
- Usage statistics
- Auto-updates

---

## 10. Open Questions

1. **Should we bundle Ollama?** Or require separate install?
   - Recommendation: Require separate install, detect version

2. **How to handle multiple simultaneous models?** Ollama supports this but uses more RAM
   - Recommendation: v1 single model, v2 add multi-model support

3. **Should Caddy proxy be required for Tailscale?**
   - Recommendation: Optional, tailnet is already authenticated

4. **App Store distribution or direct download?**
   - Recommendation: Direct download (notarized), App Store sandboxing is limiting

---

## Appendix A: Competitive Analysis

| Feature | OllamaBar | Ollama Desktop | LM Studio |
|---------|-----------|----------------|-----------|
| Menu bar app | Yes | No (full app) | No |
| Native macOS | Yes (SwiftUI) | Yes | Electron |
| Tailscale integration | Yes | No | No |
| Memory footprint | ~20 MB | ~50 MB | ~150 MB |
| Model management | Yes | Yes | Yes |
| Network sharing | Yes (1-click) | Manual | Manual |

---

## Appendix B: API Reference

### Ollama API (used by app)

- `GET /api/tags` - List models
- `GET /api/ps` - Running models
- `POST /api/generate` - Health check (empty prompt)
- `POST /api/pull` - Download model
- `POST /api/create` - Import GGUF
- `DELETE /api/delete` - Remove model

### Tailscale CLI

- `tailscale status --json` - Connection status
- `tailscale ip -4` - Get IPv4 address
- `tailscale up` - Connect (if disconnected)
