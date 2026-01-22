#!/usr/bin/env python3
import subprocess

MODELS = [
    ("local/qwen2.5-coder-7b-q4km", "/repo/modelfiles/qwen2.5-coder-7b-instruct-q4km"),
    ("local/deepseek-coder-6.7b-q4km", "/repo/modelfiles/deepseek-coder-6.7b-instruct-q4km"),
    ("local/starcoder2-7b-q4km", "/repo/modelfiles/starcoder2-7b-q4km"),
    ("local/glm-4-9b-chat-q4k", "/repo/modelfiles/glm-4-9b-chat-q4k"),
]


def list_models() -> set[str]:
    cmd = ["docker", "compose", "exec", "ollama", "ollama", "list"]
    out = subprocess.check_output(cmd, text=True, stderr=subprocess.DEVNULL)
    lines = out.splitlines()[1:]  # skip header
    names = set()
    for line in lines:
        parts = line.split()
        if parts:
            names.add(parts[0])
    return names


def main() -> int:
    existing = list_models()
    for name, modelfile in MODELS:
        if name in existing:
            continue
        cmd = ["docker", "compose", "exec", "ollama", "ollama", "create", name, "-f", modelfile]
        rc = subprocess.call(cmd)
        if rc != 0:
            return rc
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
