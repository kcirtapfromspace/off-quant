# off-quant

Local LLM runner with a Rust-first wrapper around llama.cpp and EXO.

## Why this
- Open source runtime (llama.cpp, EXO)
- Simple Rust CLI so you can extend into a real app
- Tuned for Apple Silicon

## Quick start (llama.cpp)
1) Install llama.cpp

```bash
brew install llama.cpp
```

2) Download a GGUF model and set `MODEL_PATH`

```bash
mkdir -p models
export MODEL_PATH="$(pwd)/models/tinyllama-1.1b-chat-v1.0.Q4_K_M.gguf"
```

3) Run

```bash
cargo run -- "Write a short cyberpunk haiku about rust"
```

## EXO runtime
EXO exposes an OpenAI-compatible API. Start EXO locally, then point this CLI at it.

```bash
export RUNTIME=exo
export EXO_URL="http://localhost:52415"
export MODEL_PATH="llama-3.2-1b" # EXO model id
cargo run -- "Give me a 3-bullet roadmap for a rust CLI"
```

## CLI options
```
Usage: off-quant [OPTIONS] <PROMPT...>

Options:
  --runtime <RUNTIME>          [default: llama] [possible values: llama, exo]
  --model <MODEL_PATH>         Model path or EXO model id (or set MODEL_PATH)
  --llama-bin <LLAMA_CPP_BIN>  llama.cpp binary (default: llama-cli)
  --exo-url <EXO_URL>          EXO base URL (default: http://localhost:52415)
  --gpu-layers <GPU_LAYERS>    llama.cpp GPU layers (default: 99)
  --threads <THREADS>          llama.cpp threads (default: 8)
  --temp <TEMP>                Sampling temperature
  --top-p <TOP_P>              Top-p
  --max-tokens <MAX_TOKENS>    Max tokens to generate
  --repeat-penalty <REPEAT_PENALTY> Repeat penalty
  --ctx-size <CTX_SIZE>        Context size
```

## Recommended defaults (M1 Ultra / 128GB)
- `GPU_LAYERS=99`
- `THREADS=16`

Example:

```bash
export GPU_LAYERS=99
export THREADS=16
cargo run -- "Explain quantization in one paragraph"
```

## Suggested GGUF models
- TinyLlama 1.1B Chat (Q4_K_M) for fast downloads
- Mistral 7B Instruct (Q4_K_M) for small/fast
- Llama 3.1 8B Instruct (Q4_K_M) for quality

## Env vars
- `RUNTIME` (`llama` or `exo`)
- `MODEL_PATH` (required) path to GGUF file or EXO model id
- `LLAMA_CPP_BIN` (optional) path to llama.cpp binary
- `EXO_URL` (optional) EXO base URL
- `GPU_LAYERS`, `THREADS`, `TEMP`, `TOP_P`, `MAX_TOKENS`, `REPEAT_PENALTY`, `CTX_SIZE`
