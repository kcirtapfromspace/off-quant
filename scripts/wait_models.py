#!/usr/bin/env python3
import time
from pathlib import Path

FILES = [
    Path("/Volumes/models/qwen2.5-coder-7b-instruct-q4_k_m.gguf"),
    Path("/Volumes/models/deepseek-coder-6.7b-instruct.Q4_K_M.gguf"),
    Path("/Volumes/models/starcoder2-7b-Q4_K_M.gguf"),
    Path("/Volumes/models/glm-4-9b-chat.Q4_K.gguf"),
]

def main() -> int:
    while True:
        missing = [str(p) for p in FILES if not p.exists()]
        if not missing:
            return 0
        print("Waiting for:")
        for m in missing:
            print(f"- {m}")
        time.sleep(10)

if __name__ == "__main__":
    raise SystemExit(main())
