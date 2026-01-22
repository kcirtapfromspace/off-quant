#!/usr/bin/env python3
import argparse
import os
import platform
import subprocess
from pathlib import Path

def get_mem_gb() -> int:
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

def select_model(mem_gb: int) -> str:
    # Simple, deterministic selection by available RAM.
    if mem_gb >= 64:
        return "qwen2.5-coder:7b"
    if mem_gb >= 32:
        return "deepseek-coder:6.7b"
    return "starcoder2:7b"

def write_env(path: Path, model: str, mem_gb: int) -> None:
    lines = [
        f"OLLAMA_MODEL={model}",
        f"AIDER_MODEL=ollama/{model}",
        "OLLAMA_API_BASE=http://ollama:11434",
        "AIDER_AUTO_COMMITS=1",
        "AIDER_LOG_FILE=/repo/.aider/aider.log",
        f"HOST_RAM_GB={mem_gb}",
        f"HOST_ARCH={platform.machine()}",
    ]
    path.write_text("\n".join(lines) + "\n", encoding="utf-8")

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--env", default=".env.local", help="env file path")
    args = parser.parse_args()

    mem_gb = get_mem_gb()
    model = select_model(mem_gb)
    env_path = Path(args.env)

    write_env(env_path, model, mem_gb)
    print(f"Selected model: {model}")
    print(f"Wrote: {env_path}")
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
