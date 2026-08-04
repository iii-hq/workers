# provider-opencode-go

OpenCode Go Chat Completions provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::opencode_go::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::opencode_go::refresh_models` (live `GET /v1/models` id list
enriched from a hardcoded curated metadata table → `router::models::reconcile`),
and `provider::opencode_go::abort` (cancels an in-flight upstream request).
There is no embedding surface — the OpenCode Go API is Chat Completions only.

Install with `iii worker add provider-opencode-go`; the worker takes no
per-worker config — credentials and endpoint live in the `llm-router`
configuration entry (`providers.opencode_go.api_key`, default endpoint
`https://opencode.ai/zen/go/v1/chat/completions`), exactly like
[provider-openai](https://github.com/iii-hq/workers/tree/main/provider-openai).

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The model slice is populated from live discovery and the declaration
  carries `credential_env_var: OPENCODE_GO_API_KEY`.
- **Transport:** the upstream endpoint is
  `https://opencode.ai/zen/go/v1/chat/completions` (overridable via
  `api_url`); Chat Completions wire format only.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in state (scope `provider-opencode-go`,
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
  without the field. When the upstream emits `reasoning_content` deltas they
  are relayed as thinking blocks; models that never emit them stream text
  only.
- **Structured output:** a `response_format` with a schema maps to strict
  `json_schema` mode; without one, `json_object` mode (the caller must
  mention "JSON" in the prompt per OpenAI-compatible API rules).
- **Prompt caching:** upstream-managed; `prompt_tokens_details.cached_tokens`
  lands on `usage.cache_read` when the API reports it.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream — no external API calls anywhere.

## Running

The binary takes the standard worker CLI flags: `--url` (engine WebSocket,
default `ws://127.0.0.1:49134`, falls back to the `III_URL` environment
variable), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).
