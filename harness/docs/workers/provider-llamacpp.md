# provider-llamacpp

llama.cpp `llama-server` (localhost) Chat Completions streaming
provider; exposes `provider::llamacpp::stream` and
`provider::llamacpp::complete` on the iii bus.

## Purpose

Talks to a `llama-server` process (the binary built from the
[`llama.cpp`](https://github.com/ggml-org/llama.cpp) repository) over
its OpenAI-compatible REST API. Mirrors provider-lmstudio for the chat
path, with the runtime-management surface trimmed: llama-server hosts
exactly one model per process and has no `load_model` / `unload_model`.

## Bus functions

- `provider::llamacpp::stream` — stream a single assistant turn into a
  caller-supplied `ChannelWriter`. Mirrors the other providers'
  `::stream` shape; downstream consumers (turn-orchestrator) need no
  changes.
- `provider::llamacpp::complete` — legacy non-streaming wrapper that
  drains the SSE and returns the final `AssistantMessage`.
- `provider::llamacpp::refresh_models` — re-discover the loaded model
  via `GET /v1/models` and register it into the iii models catalog.
  Idempotent.

There is no `provider::llamacpp::load_model` / `unload_model`. To run
a different model, restart `llama-server` with a different `-m`.

## Configuration

Per-machine override via env var; otherwise read from `config.yaml`:

```
LLAMACPP_BASE_URL=http://localhost:8080
# or:
LLAMACPP_BASE_URL=http://localhost:8080/v1/chat/completions
```

YAML:

```yaml
provider_llamacpp:
  default_api_url: http://localhost:8080/v1/chat/completions
  default_max_tokens: 8192
```

Default URL: `http://localhost:8080/v1/chat/completions` (llama-server's
default port; LM Studio uses 1234).

`default_max_tokens` is the fallback for `max_completion_tokens` when the
model is not in the catalog. When the loaded model is registered in the
catalog, the per-request value resolves as: registry override (clamped to
the catalog ceiling) → `min(catalog max_output_tokens, 32_000)` (env
override `HARNESS_OUTPUT_TOKEN_MAX`) → this fallback. See
[src/runtime/output-tokens.ts](harness/src/runtime/output-tokens.ts).

Both env and YAML values are validated as http(s) URLs; a malformed
value falls back to the next tier. A WARN log fires when the resolved
host is non-loopback so operators see they're shipping bearers to a
remote.

## Authentication

llama-server is local-first. Unlike LM Studio, llama.cpp has no
documented "default" bearer string — the server either enforces a
shared `--api-key` (in which case the caller must match it) or accepts
any/no token.

Policy:
- `LLAMACPP_API_KEY` configured → use it (loopback or remote).
- Loopback without a credential → omit `Authorization` entirely.
- Non-loopback without a credential → omit `Authorization` AND log a
  WARN with code `llamacpp_auth_omitted_nonloopback`.

Synthetic bearers are never sent to non-loopback hosts.

## Wire format

Standard OpenAI Chat Completions JSON over SSE:
- Requests: `messages: [{role, content, tool_calls?, tool_call_id?,
  reasoning_content?}]` with `stream: true` and `stream_options:
  { include_usage: true }`.
- Responses: SSE `data:` chunks shaped
  `{"choices":[{"delta":{"content":"...", "tool_calls":[...],
  "reasoning_content":"..."}, "finish_reason":...}]}`,
  terminated by `data: [DONE]`.

Thinking-mode models (DeepSeek-R1, Qwen-thinking) emit
`delta.reasoning_content` when llama-server is started with `--jinja
--reasoning-format deepseek` (or equivalent). The provider captures
these into a `thinking` `ContentBlock` and echoes them back via
`reasoning_content` on subsequent assistant messages — same convention
as Kimi and LM Studio.

Tool-call support requires `--jinja` plus a tool-aware chat template.

## Discovery

`/v1/models` returns the single loaded model. The startup
fire-and-forget call registers it into the catalog so the picker
shows its real id (e.g. `Meta-Llama-3.1-8B-Instruct`) instead of just
the `llamacpp-local` placeholder.

Catalog miss for any user-supplied model id falls back to the
`llamacpp-local` row so capability gating (`supports_tools` etc.)
still works.

## Defenses

- Tool-call index clamp: rejects `delta.tool_calls[].index` > 256 to
  prevent unbounded array allocation from a hostile server.
- Error chunks: surface in `error_message` only — never injected as a
  `text` ContentBlock, so a compromised server can't smuggle markup
  into the persisted assistant content stream.
- Non-2xx response bodies: truncated to 256 bytes and stripped of
  control chars before logging / surfacing.
- Stream-closed-mid-response: explicit error event when SSE closes
  without `[DONE]` or a `finish_reason`.
- Fetch timeout: 30s connect + first-byte abort so a misconfigured URL
  doesn't hang the worker for the macOS SYN timeout.

## Differences vs provider-lmstudio

| | provider-lmstudio | provider-llamacpp |
|-|-|-|
| Default port | 1234 | 8080 |
| Fallback bearer | `lm-studio` (LM Studio convention) | none — omit header |
| `load_model` / `unload_model` | yes (`/api/v0/...`) | no — restart process |
| Discovery endpoint | `/api/v0/models` (rich metadata, all downloaded) | `/v1/models` (id-only, loaded model) |
| Auto-load retry | yes (re-tries after `load_model`) | no |
| Placeholder id | `lmstudio-local` | `llamacpp-local` |
