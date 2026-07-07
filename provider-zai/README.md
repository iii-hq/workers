# provider-zai

Z.AI (GLM) Chat Completions provider worker behind [llm-router](../llm-router/).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::zai::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::zai::refresh_models` (curated catalog →
`router::models::reconcile` — Z.AI exposes no models-listing endpoint).

Default upstream: `https://api.z.ai/api/coding/paas/v4/chat/completions` —
the GLM Coding Plan endpoint (subscription keys, the common case). The
catalog follows the resolved endpoint: the coding endpoint reconciles only
the plan's models (`glm-5.2`, `glm-5.1`, `glm-5`, `glm-5-turbo`, `glm-4.7`,
`glm-4.5-air`); override `api_url`
to `https://api.z.ai/api/paas/v4/chat/completions` (pay-as-you-go) for the
full GLM lineup.

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The declaration carries no models and
  `credential_env_var: ZAI_API_KEY`; the post-register refresh reconciles
  the curated catalog, gated on a configured credential (no key → empty
  slice, so the picker never shows unusable rows).
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in iii-state (scope `provider-zai`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `ZAI_API_KEY` env on the router → none). Both
  `api_key` and `oauth` credential shapes are sent as `Authorization:
  Bearer`; v1 performs no OAuth refresh. Keys are endpoint-bound on Z.AI's
  side: GLM Coding Plan keys work on the default coding endpoint but fail
  on the general endpoint with business code 1113, and pay-as-you-go
  Open Platform keys need the `api_url` override to the general endpoint.
- **Catalog:** `src/curated.rs` is the source of truth — ids, windows,
  output ceilings, capability flags, and pricing (USD per MTok) from
  docs.z.ai. Update it when Z.AI ships new models; there is no live listing
  to discover them from. `refresh_models` reconciles the slice matching the
  resolved endpoint (Coding Plan subset vs full table).
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited` (except
  `insufficient_quota`/code `1113`, billing walls → `permanent`),
  `context_length_exceeded` → `context_overflow`, 5xx/network → `transient`,
  other 4xx → `permanent`. No transport retries here — the router owns
  retry policy.
- **Structured output:** `json_object` mode only — Z.AI documents no strict
  json_schema mode. A `response_format` schema is dropped with a
  report-and-continue warning; every curated record declares
  `supports_structured_output: false`. The caller must mention "JSON" in
  the prompt per Z.AI's rules.
- **Reasoning:** every current GLM chat model takes the
  `thinking: {type: enabled|disabled}` toggle — sent explicitly, since the
  API default (enabled, auto-decide) would buy unrequested thinking tokens.
  `thinking_level` additionally maps 1:1 to `reasoning_effort` on GLM-5.2+
  (`src/reasoning.rs`); earlier families are not documented to take the
  param, so it is omitted for them. Reasoning models stream their chain of
  thought as `reasoning_content` deltas, which the worker surfaces as
  `thinking` blocks on the channel (`src/sse.rs`);
  `completion_tokens_details.reasoning_tokens` lands on `usage.reasoning`.
- **Tool streaming:** `tool_stream: true` rides along whenever tools are
  present on GLM-4.6-or-newer models, so tool-call arguments stream
  incrementally; older families deliver whole-chunk arguments, which the
  SSE decoder handles either way.
- **Prompt caching:** implicit on Z.AI's side — no request markers.
  `prompt_tokens_details.cached_tokens` lands on `usage.cache_read`, billed
  at the cached-input rate in the pricing table.

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
