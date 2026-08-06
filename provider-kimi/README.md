# provider-kimi

Moonshot (Kimi) Chat Completions provider worker behind
[llm-router](../llm-router/). Moonshot's API is OpenAI Chat
Completions–compatible, so this worker forks the shared provider scaffolding
and adds Kimi's `reasoning_content` thinking stream.

Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::kimi::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::kimi::refresh_models` (live `GET /v1/models` filtered to Kimi /
Moonshot chat families, enriched with a curated capability snapshot →
`router::models::reconcile`).

`provider::kimi::count_tokens` counts a prompt through Moonshot's own
estimator endpoint behind `router::count_tokens`, so the number is the
upstream's rather than a local reconstruction of it. Counting never runs the
model.

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The declaration carries no static `models` slice — `GET /v1/models` is the
  source of truth and a refresh fires right after registration. Defaults:
  `api_url: https://api.moonshot.ai/v1/chat/completions`,
  `credential_env_var: MOONSHOT_API_KEY`.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in state (scope `provider-kimi`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `MOONSHOT_API_KEY` env on the router → none). Both
  `api_key` and `oauth` credential shapes are sent as `Authorization:
  Bearer`; v1 performs no OAuth refresh.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited` (except a quota
  wall, `exceeded_current_quota_error` → `permanent`),
  context-length errors → `context_overflow`, 5xx/`engine_overloaded_error` /
  network → `transient`, other 4xx → `permanent`. No transport retries here —
  the router owns retry policy.
- **Request shape vs OpenAI:** Moonshot accepts the classic `max_tokens` param
  (not `max_completion_tokens`) and has no `reasoning_effort` knob.
- **Thinking:** Kimi thinking models (Kimi K2 Thinking, `kimi-thinking-preview`)
  stream their reasoning in `delta.reasoning_content` before the answer text.
  This worker surfaces it as a `Thinking` content block via
  `ThinkingStart`/`ThinkingDelta`/`ThinkingEnd` (no replay signature). The
  model itself decides whether it thinks; `thinking_level` is advisory.
- **Structured output:** Moonshot supports JSON mode only
  (`response_format: {"type":"json_object"}`), not OpenAI's strict
  `json_schema` mode. A requested schema is mapped to `json_object` and a
  report-and-continue warning rides the final message (the caller must mention
  "JSON" in the prompt). Curated records declare
  `supports_structured_output: true`.
- **Prompt caching:** Moonshot context caching is automatic — no request
  markers. `prompt_tokens_details.cached_tokens` lands on `usage.cache_read`.
- **Curated snapshot:** `src/curated.rs` carries display names / context
  windows / output ceilings / capability flags / pricing for known Kimi and
  Moonshot families; conservative defaults for unknown ones. Context windows
  and pricing are best-effort placeholders — **verify against
  platform.moonshot.ai before release**. Discovery supplies only bare ids.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream — no external API calls anywhere.

## Running

The binary takes the standard worker CLI flags: `--url` (engine WebSocket,
default `ws://127.0.0.1:49134`, falls back to the `III_WS_URL` environment
variable), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).
