#!/usr/bin/env python3
import subprocess
from pathlib import Path

ENV_PATH = Path(".env.local")

def main() -> int:
    if not ENV_PATH.exists():
        raise SystemExit(".env.local not found; run select_model first")

    model = None
    for line in ENV_PATH.read_text(encoding="utf-8").splitlines():
        if line.startswith("OLLAMA_MODEL="):
            model = line.split("=", 1)[1].strip()
            break
    if not model:
        raise SystemExit("OLLAMA_MODEL not set in .env.local")

    cmd = ["docker", "compose", "exec", "ollama", "ollama", "pull", model]
    return subprocess.call(cmd)

if __name__ == "__main__":
    raise SystemExit(main())
