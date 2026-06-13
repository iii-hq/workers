# provider-openai

OpenAI Chat Completions provider worker behind [llm-router](../llm-router/).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::openai::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::openai::refresh_models` (live `GET /v1/models` filtered to
chat/reasoning families ∪ curated capability snapshot →
`router::models::reconcile`).

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` pubsub topic.
  The declaration ships a static curated `models` slice (no cold-catalog
  hole) and `credential_env_var: OPENAI_API_KEY`.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in iii-state (scope `provider-openai`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `OPENAI_API_KEY` env on the router → none). Both
  `api_key` and `oauth` credential shapes are sent as `Authorization:
  Bearer`; v1 performs no OAuth refresh.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited` (except
  `insufficient_quota`, a billing wall → `permanent`),
  `context_length_exceeded` → `context_overflow`, 5xx/network → `transient`,
  other 4xx → `permanent`. No transport retries here — the router owns
  retry policy.
- **Structured output:** native. A `response_format` with a schema maps to
  strict `json_schema` mode; without one, `json_object` mode (the caller
  must mention "JSON" in the prompt per OpenAI's rules). Every curated
  record declares `supports_structured_output: true`.
- **Reasoning:** `thinking_level` maps to `reasoning_effort` per model
  family (`src/reasoning.rs` — the ladders encode real 400s: o1 and
  chat-tuned variants take no param, pro is high-only, xhigh is gpt-5.2+).
  No reasoning text is streamed back on Chat Completions;
  `completion_tokens_details.reasoning_tokens` lands on `usage.reasoning`.
- **Prompt caching:** automatic on OpenAI's side — no request markers.
  `prompt_tokens_details.cached_tokens` lands on `usage.cache_read`.
- **Curated snapshot:** `src/curated.rs` carries windows / output ceilings /
  capability flags / pricing (USD per MTok). Update it against models.dev
  when OpenAI ships new models — discovery only supplies bare ids.

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
