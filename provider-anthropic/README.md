# provider-anthropic

Anthropic Messages API provider worker behind [llm-router](../llm-router/).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::anthropic::stream`
(SSE → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::anthropic::refresh_models` (live `GET /v1/models` ∪ curated
capability snapshot → `router::models::reconcile`).

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` pubsub topic.
  The declaration ships a static curated `models` slice (no cold-catalog
  hole) and `credential_env_var: ANTHROPIC_API_KEY`.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in iii-state (scope `provider-anthropic`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `ANTHROPIC_API_KEY` env on the router → none). Both
  `api_key` (x-api-key) and `oauth` (Bearer) credential shapes are sent;
  v1 performs no OAuth refresh.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited`, 413/context →
  `context_overflow`, 5xx/network → `transient`, other 4xx → `permanent`.
  No transport retries here — the router owns retry policy.
- **Structured output:** the Messages API has no native JSON mode; every
  catalog record declares `supports_structured_output: false` and a
  forwarded `response_format` (cold-catalog fail-open) is reported in
  `warnings` and ignored.
- **Prompt caching:** cache markers on the system prompt, tools tail, and
  the last stable assistant turn. Kill switch: `PROVIDER_ANTHROPIC_CACHE=0`.
- **Curated snapshot:** `src/curated.rs` carries windows / output ceilings /
  thinking budgets / pricing (USD per MTok). Update it against models.dev
  when Anthropic ships new models — discovery only supplies bare ids.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream — no external API calls anywhere.
