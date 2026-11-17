# Add LLM providers and choose models

**Goal:** connect one or more model providers.

Harvest supports `anthropic`, `gemini` and `openai-compatible` (OpenRouter, Groq, Ollama, vLLM and others).

## With the CLI

```bash
sudo harvest llm provider add --id primary --provider anthropic --api-key sk-ant-... --model claude-sonnet-5-5
sudo harvest llm provider add --id router --provider open-ai-compatible \
  --base-url https://openrouter.ai/api/v1 --api-key sk-or-... --model anthropic/claude-sonnet-5.5 --priority 1
harvest llm provider list
sudo harvest llm provider set-primary primary
sudo harvest llm provider models primary set claude-sonnet-5-5,claude-opus-5-5
```

`harvest llm provider detect-models <id>` asks the provider which models it offers. Commands apply and restart the server at once unless you pass `--apply-now false`.

## In `server.toml`

```toml
[[llm]]
provider = "openai-compatible"
id       = "openrouter"
name     = "OpenRouter"
base_url = "https://openrouter.ai/api/v1"
api_key  = "sk-or-..."
model    = "anthropic/claude-sonnet-5.5"
models   = ["anthropic/claude-sonnet-5.5", "openai/gpt-5.3-codex"]
priority = 0
```

- `model` is the default; `models` limits what users can pick in the chat's model picker.
- With several providers, a rate-limited request moves to the next by `priority` (lower first).
- `expose_to_ui = false` keeps a provider for failover only.
- Prompt caching is on for Claude models behind `openai-compatible` providers, which cuts their cost by most of the repeated input. Force it with `enable_prompt_caching = true` or off with `false` (CLI: `--enable-prompt-caching`).

All keys: [configuration reference](../reference/configuration.md#llm).
