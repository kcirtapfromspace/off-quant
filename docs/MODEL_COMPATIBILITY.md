# Model Compatibility Guide

This document describes Ollama model compatibility with `quant` agent's tool calling feature.

## Overview

The `quant agent` command uses LLM tool/function calling to execute actions. Ollama models vary in their support for the native tool calling API.

## Tool Calling Methods

### 1. Native Tool Calling (Preferred)

Models with native support use Ollama's `/api/chat` endpoint with the `tools` parameter. The model returns tool calls in a structured `tool_calls` field.

**Advantages:**
- Reliable, structured output
- No parsing ambiguity
- Better error handling

**Models with Native Support:**
| Model | Tool Support | Notes |
|-------|--------------|-------|
| `llama3.1` | Excellent | Official tool calling support |
| `llama3.2` | Excellent | Official tool calling support |
| `mistral` | Good | Supports function calling |
| `mixtral` | Good | Supports function calling |

### 2. Content Parsing Fallback

When models don't support native tool calling, `quant` automatically parses JSON from the model's text response. This handles multiple formats:

| Format | Example | Support |
|--------|---------|---------|
| Direct JSON | `{"name": "grep", "arguments": {...}}` | Full |
| Code blocks | ````json\n{...}\n```` | Full |
| Multiline JSON | Spanning multiple lines | Full |
| Mixed content | Text before/after JSON | Full |
| JSON arrays | `[{...}, {...}]` | Full |

**Models Requiring Fallback:**
| Model | Compatibility | Notes |
|-------|--------------|-------|
| `qwen2.5-coder` | Good | Outputs JSON in content field |
| `codellama` | Moderate | May need prompting hints |
| `deepseek-coder` | Good | Reliable JSON output |

## Recommended Models

### For Agent Tasks

1. **`llama3.2`** - Best overall, native tool support
2. **`qwen2.5-coder:7b`** - Good for coding, uses fallback
3. **`mistral`** - Good balance of speed and capability

### For Code-Heavy Tasks

1. **`qwen2.5-coder:7b`** or **`qwen2.5-coder:32b`**
2. **`deepseek-coder`**
3. **`codellama`**

## Troubleshooting

### Model Repeats Same Failing Tool Call

**Symptom:** Agent gets stuck calling the same tool with identical arguments.

**Cause:** Model doesn't learn from tool error feedback.

**Solution:** `quant` includes automatic detection - it aborts after 3 consecutive identical failures and reports the error.

### Model Outputs JSON as Plain Text

**Symptom:** Tool calls appear in response text but aren't executed.

**Cause:** Model doesn't use native tool calling format.

**Solution:** Automatic - `quant` parses JSON from content using multiple strategies.

### Model Doesn't Call Tools

**Symptom:** Model responds with text instead of calling available tools.

**Causes:**
1. Model doesn't understand tool calling
2. System prompt not clear enough

**Solutions:**
- Use a model with better tool support (see table above)
- Check if model documentation mentions function calling

### Invalid JSON in Response

**Symptom:** `Tool error: Failed to parse arguments`

**Cause:** Model generated malformed JSON.

**Solutions:**
- Try a more capable model
- Report if it's a consistent pattern (may need parser improvements)

## Testing Model Compatibility

Run this command to test if a model works with agent tools:

```bash
quant agent "List the files in the current directory"
```

Expected behavior:
1. Model should call `glob` or `bash` tool
2. Tool executes and returns file list
3. Model summarizes the result

If the model responds with text instructions instead of executing tools, it may not be compatible.

## Configuration

Set your preferred model in `~/.config/quant/llm.toml`:

```toml
[llm]
default_model = "llama3.2"

[llm.ollama]
url = "http://localhost:11434"
```

Or specify per-command:

```bash
quant agent --model qwen2.5-coder:7b "Analyze this codebase"
```

## Contributing

If you find a model that works well (or has issues), please open an issue with:
1. Model name and version
2. Whether native tool calling works
3. Any quirks or workarounds needed
