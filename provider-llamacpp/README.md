# provider-llamacpp

llama.cpp server ([`llama-server`](https://github.com/ggml-org/llama.cpp/blob/master/tools/server/README.md))
Chat Completions provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::llamacpp::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::llamacpp::refresh_models` (live `GET /v1/models` + `GET /props` →
`router::models::reconcile`).

Default upstream: `http://127.0.0.1:8080/v1/chat/completions` — `llama-server`'s
own default bind address and port. Point `api_url` at any running
`llama-server` instance (local, LAN, or a remote box) to use it.

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The declaration carries no models and `credential_env_var:
  LLAMACPP_API_KEY`; the post-register refresh discovers the live catalog
  from the resolved server.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in iii-state (scope `provider-llamacpp`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials are optional** — the main difference from every other
  provider here. `llama-server` only requires `Authorization: Bearer` when
  started with `--api-key`; most local setups run with none at all. Streaming
  and discovery both resolve credentials via `router::provider::resolve` as
  usual, but a missing/blank credential is treated as "no key configured",
  not a configuration error: requests simply go out with no `Authorization`
  header. If the server *does* have `--api-key` set and ours is missing or
  wrong, the server's 401/403 surfaces as the normal `auth_expired` error.
- **Catalog:** `src/discovery.rs` discovers the catalog live — `GET
  /v1/models` lists every id the server serves (no "gpt-"-style family gate:
  llama.cpp serves arbitrary GGUF aliases, so every id is kept), enriched
  with `GET /props` for the runtime context size (`n_ctx`, the operator's
  `--ctx-size` — more accurate than `/v1/models`' `meta.n_ctx_train`, the
  model's *trained* max) and vision-modality support. No pricing
  (self-hosted). Multi-model router-mode (`--models-dir`, `GET /models`,
  `/models/load`) is out of scope for v1 — this targets the common
  single-loaded-model server.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** 401/403 → `auth_expired` (only reachable when `--api-key` is
  set), `context_length_exceeded`/message-sniffed prompt-overflow phrasing →
  `context_overflow`, 5xx/network → `transient`, other 4xx → `permanent`. No
  transport retries here — the router owns retry policy.
- **Structured output:** real schema-constrained decoding, unlike
  `json_object`-only providers — `response_format: {"type": "json_schema",
  "schema": {...}}` (llama.cpp nests the schema directly, not under an extra
  OpenAI-style `json_schema` wrapper key), or `{"type": "json_object"}` with
  no schema. Every discovered model advertises
  `supports_structured_output: true`.
- **Reasoning:** llama.cpp has no dedicated per-request reasoning switch. Its
  only lever is the `enable_thinking` chat-template kwarg, so a requested
  `thinking_level` is mapped best-effort onto `chat_template_kwargs:
  {"enable_thinking": …}` (any level → `true`, absent → `false`) — which
  reasoning GGUFs conventionally gate their thinking channel on. It is
  effective only if the model's chat template references that key; otherwise
  reasoning stays whatever the server's `--reasoning-format` flag and template
  dictate, and a report-and-continue warning notes the mapping is best-effort.
  When the server runs with `--reasoning-format deepseek`, chain-of-thought
  streams as `reasoning_content` deltas, which this worker surfaces as
  `thinking` blocks on the channel (`src/sse.rs`).
- **Tool calling:** requires the server be started with `--jinja` and a
  chat template that supports tool calls; tool schemas ride as the standard
  OpenAI `{"type":"function","function":{...}}` envelope. Every discovered
  model optimistically advertises `supports_tools: true` — llama.cpp
  silently ignores tools a template can't use, so this never breaks
  non-tool turns.
- **Prompt caching:** `cache_prompt` reuse is a server-side default in
  llama.cpp, not something this provider requests explicitly.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream — no external servers required.

## Running

The binary takes the standard worker CLI flags: `--url` (engine WebSocket,
default `ws://127.0.0.1:49134`, falls back to the `III_WS_URL` environment
variable), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).

Point it at a real `llama-server`:

```bash
llama-server -m /path/to/model.gguf --jinja --port 8080
cargo run -- --url ws://127.0.0.1:49134
```
