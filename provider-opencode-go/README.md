# provider-opencode-go
OpenCode Go Chat Completions provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::opencode_go::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::opencode_go::refresh_models` (live `GET /v1/models` id list
enriched from a hardcoded curated metadata table → `router::models::reconcile`),
and `provider::opencode_go::abort` (cancels an in-flight upstream request).
There is no embedding surface — the OpenCode Go API is Chat Completions only.

## Behavior
- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The model slice is populated from live discovery and the declaration
  carries `credential_env_var: OPENCODE_GO_API_KEY`.
- **Transport:** the upstream endpoint is
  `https://opencode.ai/zen/go/v1/chat/completions` (overridable via
  `api_url`); Chat Completions wire format only.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in iii-state (scope `provider-opencode-go`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `OPENCODE_GO_API_KEY` env on the router → none). The key
  is sent as `Authorization: Bearer`.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited`,
  `context_length_exceeded` → `context_overflow`, 5xx/network → `transient`,
  other 4xx → `permanent`. No transport retries here — the router owns
  retry policy.
- **Model metadata:** discovery fetches the live id list from `GET /v1/models`
  (the API carries no capability data) and enriches each id from a hardcoded
  curated table (`src/curated.rs`) covering the maintainer's OpenCode Go
  subscription catalog — the 24 `opencode-go` entries on
  [models.dev](https://models.dev) (fetched 2026-08-03) plus `hy3-preview`,
  which models.dev does not list and which keeps conservative defaults —
  context windows, reasoning support/effort levels, tool-call and
  structured-output capability. Ids the table does not know keep conservative
  defaults (128K context, no thinking, tools on). Same pattern as
  provider-openai.
- **Reasoning:** `thinking_level` maps to the upstream `reasoning_effort`
  when the model's curated effort list accepts the level (e.g. `grok-4.5`
  accepts `low`/`medium`/`high`, `deepseek-v4-flash` accepts `high`/`max`);
  models that reason without published effort levels, and unknown ids, stream
  without the field. Thinking content is not streamed — the OpenCode Go Chat
  Completions wire carries no reasoning deltas.
- **Structured output:** a `response_format` with a schema maps to strict
  `json_schema` mode; without one, `json_object` mode (the caller must
  mention "JSON" in the prompt per OpenAI-compatible API rules).
- **Prompt caching:** upstream-managed; `prompt_tokens_details.cached_tokens`
  lands on `usage.cache_read` when the API reports it.

## Install
```bash
iii worker add provider-opencode-go
```
`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker the next time it
boots. The provider must be able to reach the engine's WebSocket (`--url`,
default `ws://127.0.0.1:49134`).

## Quickstart
The provider registers itself with llm-router; you drive it through
`router::llm`-style calls, never directly:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());
    let result = iii
        .trigger(TriggerRequest {
            function_id: "router::llm::chat".into(),
            payload: json!({
                "provider": "opencode_go",
                "model": "deepseek-v4-flash",
                "messages": [{"role": "user", "content": "hello"}],
                "max_tokens": 512,
            }),
            action: None,
            timeout_ms: Some(60_000),
        })
        .await?;
    println!("{result:#?}");
    Ok(())
}
```

## Configuration
The worker takes no per-worker config — provider settings live in the
`llm-router` configuration entry, exactly like
[provider-openai](https://github.com/iii-hq/workers/tree/main/provider-openai):

```yaml
# ~/.iii/config.yaml — llm-router section
llm-router:
  opencode_go:
    api_key: ${OPENCODE_GO_API_KEY:}   # fallback: env on the router
    api_url: https://opencode.ai/zen/go/v1/chat/completions  # optional override
```

The `OPENCODE_GO_API_KEY` environment variable on the router (or on the
provider process) is the canonical credential source; a key under
`llm-router.opencode_go.api_key` wins if both are set.

## Tests
```bash
cargo test   # unit (pure modules + TCP stubs); no external API calls
```
