#!/usr/bin/env python3
"""
llm-ctl: Unified CLI for local LLM management.

Commands:
    status      Show Ollama status and loaded models
    health      Health check with retries (for scripts)
    models      List available models
    import      Import local GGUF files into Ollama
    pull        Pull a model from Ollama registry
    select      Auto-select best model based on RAM
    env         Generate .env.local for Aider
    serve       Start Ollama (if not using launchd)
"""

import argparse
import json
import os
import platform
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Optional

try:
    import tomllib
except ImportError:
    import tomli as tomllib  # Python < 3.11

# ANSI colors
GREEN = "\033[92m"
RED = "\033[91m"
YELLOW = "\033[93m"
BLUE = "\033[94m"
RESET = "\033[0m"
BOLD = "\033[1m"


@dataclass
class Config:
    ollama_host: str
    ollama_port: int
    models_path: Path
    ollama_home: Path
    local_models: dict
    coding_model: str
    chat_model: str
    threshold_high: int
    threshold_medium: int

    @classmethod
    def load(cls, path: Path) -> "Config":
        with open(path, "rb") as f:
            data = tomllib.load(f)

        return cls(
            ollama_host=data["ollama"]["host"],
            ollama_port=data["ollama"]["port"],
            models_path=Path(data["ollama"]["models_path"]),
            ollama_home=Path(data["ollama"]["ollama_home"]),
            local_models=data["models"]["local"],
            coding_model=data["models"]["coding"],
            chat_model=data["models"]["chat"],
            threshold_high=data["models"]["auto_select"]["threshold_high"],
            threshold_medium=data["models"]["auto_select"]["threshold_medium"],
        )

    @property
    def base_url(self) -> str:
        return f"http://{self.ollama_host}:{self.ollama_port}"


def get_config() -> Config:
    # Look for llm.toml in current dir or parent dirs
    search = Path.cwd()
    for _ in range(5):
        candidate = search / "llm.toml"
        if candidate.exists():
            return Config.load(candidate)
        search = search.parent
    raise FileNotFoundError("llm.toml not found")


def get_mem_gb() -> int:
    """Get system RAM in GB."""
    system = platform.system().lower()
    if system == "darwin":
        raw = subprocess.check_output(["sysctl", "-n", "hw.memsize"]).strip()
        return int(raw) // (1024 ** 3)
    if system == "linux":
        with open("/proc/meminfo", "r", encoding="utf-8") as f:
            for line in f:
                if line.startswith("MemTotal:"):
                    kb = int(line.split()[1])
                    return kb // (1024 ** 2)
    return 0


def ollama_api(endpoint: str, method: str = "GET", data: Optional[dict] = None, timeout: int = 30) -> Optional[dict]:
    """Make an API call to Ollama."""
    import urllib.request
    import urllib.error

    config = get_config()
    url = f"{config.base_url}{endpoint}"

    req = urllib.request.Request(url, method=method)
    req.add_header("Content-Type", "application/json")

    body = json.dumps(data).encode() if data else None

    try:
        with urllib.request.urlopen(req, body, timeout=timeout) as resp:
            return json.loads(resp.read().decode())
    except urllib.error.URLError:
        return None
    except json.JSONDecodeError:
        return None


def is_ollama_running() -> bool:
    """Check if Ollama is responding."""
    result = ollama_api("/api/tags", timeout=5)
    return result is not None


def wait_for_ollama(timeout: int = 60, interval: int = 2) -> bool:
    """Wait for Ollama to be ready."""
    start = time.time()
    while time.time() - start < timeout:
        if is_ollama_running():
            return True
        time.sleep(interval)
    return False


def get_models() -> list[dict]:
    """Get list of models from Ollama."""
    result = ollama_api("/api/tags")
    if result and "models" in result:
        return result["models"]
    return []


def print_status(ok: bool, msg: str) -> None:
    """Print a status line."""
    icon = f"{GREEN}✓{RESET}" if ok else f"{RED}✗{RESET}"
    print(f"  {icon} {msg}")


# --- Commands ---

def cmd_status(args: argparse.Namespace) -> int:
    """Show Ollama status and models."""
    config = get_config()
    print(f"{BOLD}Ollama Status{RESET}")
    print(f"  Endpoint: {config.base_url}")

    if not is_ollama_running():
        print_status(False, "Ollama is not running")
        print(f"\n  Start with: {BLUE}ollama serve{RESET}")
        print(f"  Or: {BLUE}just serve{RESET}")
        return 1

    print_status(True, "Ollama is running")

    # Check models volume
    vol_ok = config.models_path.exists()
    print_status(vol_ok, f"Models volume: {config.models_path}")

    # List models
    models = get_models()
    print(f"\n{BOLD}Loaded Models ({len(models)}){RESET}")
    if models:
        for m in sorted(models, key=lambda x: x["name"]):
            size_gb = m.get("size", 0) / (1024**3)
            print(f"  - {m['name']} ({size_gb:.1f} GB)")
    else:
        print(f"  {YELLOW}No models loaded{RESET}")
        print(f"  Run: {BLUE}just import{RESET}")

    # Show RAM
    ram = get_mem_gb()
    print(f"\n{BOLD}System{RESET}")
    print(f"  RAM: {ram} GB")
    print(f"  Arch: {platform.machine()}")

    return 0


def cmd_health(args: argparse.Namespace) -> int:
    """Health check with retries."""
    timeout = args.timeout
    print(f"Waiting for Ollama (timeout: {timeout}s)...", end="", flush=True)

    if wait_for_ollama(timeout=timeout):
        print(f" {GREEN}OK{RESET}")
        return 0
    else:
        print(f" {RED}FAILED{RESET}")
        return 1


def cmd_models(args: argparse.Namespace) -> int:
    """List available models."""
    config = get_config()

    print(f"{BOLD}Local GGUF Files{RESET}")
    for key, model in config.local_models.items():
        path = config.models_path / model["file"]
        exists = path.exists()
        status = f"{GREEN}exists{RESET}" if exists else f"{RED}missing{RESET}"
        print(f"  {model['name']}: {status}")

    if not is_ollama_running():
        print(f"\n{YELLOW}Ollama not running - can't list imported models{RESET}")
        return 0

    print(f"\n{BOLD}Imported in Ollama{RESET}")
    models = get_models()
    local_names = {m["name"] for m in config.local_models.values()}

    for m in sorted(models, key=lambda x: x["name"]):
        tag = f" {BLUE}(local){RESET}" if m["name"] in local_names else ""
        print(f"  - {m['name']}{tag}")

    return 0


def cmd_import(args: argparse.Namespace) -> int:
    """Import local GGUF files into Ollama."""
    config = get_config()

    if not is_ollama_running():
        print(f"{RED}Ollama is not running{RESET}")
        return 1

    if not config.models_path.exists():
        print(f"{RED}Models volume not mounted: {config.models_path}{RESET}")
        return 1

    existing = {m["name"] for m in get_models()}
    imported = 0

    for key, model in config.local_models.items():
        name = model["name"]
        gguf_path = config.models_path / model["file"]
        modelfile_path = Path(model["modelfile"])

        if name in existing:
            print(f"  {YELLOW}skip{RESET} {name} (already exists)")
            continue

        if not gguf_path.exists():
            print(f"  {RED}skip{RESET} {name} (GGUF not found: {gguf_path})")
            continue

        if not modelfile_path.exists():
            print(f"  {RED}skip{RESET} {name} (Modelfile not found: {modelfile_path})")
            continue

        print(f"  {BLUE}importing{RESET} {name}...", end="", flush=True)

        # Use ollama create command
        cmd = ["ollama", "create", name, "-f", str(modelfile_path)]
        result = subprocess.run(cmd, capture_output=True, text=True)

        if result.returncode == 0:
            print(f" {GREEN}OK{RESET}")
            imported += 1
        else:
            print(f" {RED}FAILED{RESET}")
            print(f"    {result.stderr.strip()}")

    print(f"\nImported {imported} model(s)")
    return 0


def cmd_pull(args: argparse.Namespace) -> int:
    """Pull a model from Ollama registry."""
    model = args.model
    print(f"Pulling {model}...")

    result = subprocess.run(["ollama", "pull", model])
    return result.returncode


def cmd_select(args: argparse.Namespace) -> int:
    """Auto-select best model based on RAM."""
    config = get_config()
    ram = get_mem_gb()

    if ram >= config.threshold_high:
        model = "local/qwen2.5-coder-7b-q4km"
    elif ram >= config.threshold_medium:
        model = "local/deepseek-coder-6.7b-q4km"
    else:
        model = "local/starcoder2-7b-q4km"

    print(f"RAM: {ram} GB")
    print(f"Selected: {model}")

    if args.json:
        print(json.dumps({"ram_gb": ram, "model": model}))

    return 0


def cmd_env(args: argparse.Namespace) -> int:
    """Generate .env.local for Aider."""
    config = get_config()
    ram = get_mem_gb()

    # Auto-select model
    if ram >= config.threshold_high:
        model = "local/qwen2.5-coder-7b-q4km"
    elif ram >= config.threshold_medium:
        model = "local/deepseek-coder-6.7b-q4km"
    else:
        model = "local/starcoder2-7b-q4km"

    env_path = Path(args.output)
    lines = [
        f"OLLAMA_MODEL={model}",
        f"AIDER_MODEL=ollama/{model}",
        f"OLLAMA_API_BASE={config.base_url}",
        "AIDER_AUTO_COMMITS=1",
        "AIDER_LOG_FILE=.aider/aider.log",
        f"HOST_RAM_GB={ram}",
        f"HOST_ARCH={platform.machine()}",
    ]

    env_path.write_text("\n".join(lines) + "\n", encoding="utf-8")
    print(f"Wrote: {env_path}")
    print(f"Model: {model}")
    return 0


def cmd_serve(args: argparse.Namespace) -> int:
    """Start Ollama server."""
    config = get_config()

    # Set OLLAMA_HOME for model storage location
    env = os.environ.copy()
    env["OLLAMA_HOME"] = str(config.ollama_home)
    env["OLLAMA_HOST"] = f"{config.ollama_host}:{config.ollama_port}"

    print(f"Starting Ollama...")
    print(f"  OLLAMA_HOME={config.ollama_home}")
    print(f"  OLLAMA_HOST={config.ollama_host}:{config.ollama_port}")

    try:
        subprocess.run(["ollama", "serve"], env=env)
    except KeyboardInterrupt:
        print("\nStopped")

    return 0


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Unified CLI for local LLM management",
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    subparsers = parser.add_subparsers(dest="command", required=True)

    # status
    subparsers.add_parser("status", help="Show Ollama status and loaded models")

    # health
    health_p = subparsers.add_parser("health", help="Health check with retries")
    health_p.add_argument("-t", "--timeout", type=int, default=60, help="Timeout in seconds")

    # models
    subparsers.add_parser("models", help="List available models")

    # import
    subparsers.add_parser("import", help="Import local GGUF files into Ollama")

    # pull
    pull_p = subparsers.add_parser("pull", help="Pull a model from Ollama registry")
    pull_p.add_argument("model", help="Model name to pull")

    # select
    select_p = subparsers.add_parser("select", help="Auto-select best model based on RAM")
    select_p.add_argument("--json", action="store_true", help="Output as JSON")

    # env
    env_p = subparsers.add_parser("env", help="Generate .env.local for Aider")
    env_p.add_argument("-o", "--output", default=".env.local", help="Output file path")

    # serve
    subparsers.add_parser("serve", help="Start Ollama server")

    args = parser.parse_args()

    commands = {
        "status": cmd_status,
        "health": cmd_health,
        "models": cmd_models,
        "import": cmd_import,
        "pull": cmd_pull,
        "select": cmd_select,
        "env": cmd_env,
        "serve": cmd_serve,
    }

    try:
        return commands[args.command](args)
    except FileNotFoundError as e:
        print(f"{RED}Error: {e}{RESET}", file=sys.stderr)
        return 1
    except KeyboardInterrupt:
        print("\nInterrupted")
        return 130


if __name__ == "__main__":
    sys.exit(main())
