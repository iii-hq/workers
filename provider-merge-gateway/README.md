# provider-merge-gateway

[Merge Gateway](https://docs.merge.dev/merge-gateway/get-started) provider worker behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router). Merge Gateway puts every
vendor LLM (OpenAI, Anthropic, Google, AWS Bedrock, …) behind one API with routing, failover,
cost management, and observability built in. This worker is a fork of `provider-openai` that
talks to Merge Gateway's OpenAI Chat Completions-compatible surface
(`https://api-gateway.merge.dev/v1/openai`) instead of `api.openai.com` directly — same wire
format, same SSE relay, same tool-calling/structured-output/reasoning mapping, pointed at a
different (multi-vendor) upstream.

Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::merge-gateway::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::merge-gateway::refresh_models` (live `GET /v1/openai/models` filtered to
chat/reasoning families ∪ curated capability snapshot → `router::models::reconcile`),
`provider::merge-gateway::embed` (batch text embeddings behind `router::embed`), and
`provider::merge-gateway::count_tokens` (local prompt token estimation with the tiktoken
tokenizers behind `router::count_tokens`; never runs the model, costs nothing, and needs no
network — an estimate, since Merge Gateway may route the actual request to a non-OpenAI
vocabulary).

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The model slice is populated from live discovery and the declaration carries
  `credential_env_var: MERGE_GATEWAY_API_KEY`.
- **Transport:** the default endpoint is
  `https://api-gateway.merge.dev/v1/openai/chat/completions` — Merge Gateway's OpenAI SDK
  drop-in surface. The worker speaks the OpenAI Chat Completions wire format (request shaping,
  SSE deltas, tool calls, usage accounting) exactly as `provider-openai` does for
  OpenAI-compatible gateways; only the endpoint and credential env var differ. Point `api_url`
  at a project-scoped Merge Gateway endpoint or a self-hosted Gateway instance to override.
- **Model routing:** with no routing policy configured on the Merge Gateway project, the
  `model` field selects the upstream vendor/model directly (e.g. `gpt-5.2`); with a routing
  policy configured, omit `model` (or send Merge's `default_routing` sentinel via `api_url`
  query/config) to let the policy pick the provider and model. See
  [Routing policies](https://docs.merge.dev/merge-gateway/routing/overview).
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in state (scope `provider-merge-gateway`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `MERGE_GATEWAY_API_KEY` env on the router → none), sent as
  `Authorization: Bearer <merge-gateway-api-key>` — the same key from the
  [Merge Gateway dashboard](https://gateway.merge.dev/api-keys) used with the OpenAI SDK
  base-URL swap. Both `api_key` and `oauth` credential shapes are supported; v1 performs no
  OAuth refresh.
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
  must mention "JSON" in the prompt per OpenAI's rules, which Merge Gateway's
  OpenAI-compatible surface preserves).
- **Reasoning:** `thinking_level` maps to the model family's reasoning knob per
  `src/reasoning.rs`, same ladders as `provider-openai`'s Chat Completions path — accurate for
  OpenAI models routed through Merge Gateway; non-OpenAI models proxied through the same
  surface fall back to whatever reasoning field their Chat-Completions-compatible response
  exposes.
- **Curated snapshot:** `src/curated.rs` carries windows / output ceilings /
  capability flags / pricing for known OpenAI model families reachable through Merge Gateway;
  unrecognized ids (other vendors' models Merge Gateway also proxies) get conservative
  defaults rather than vanishing from the catalog.

## Configuration

Set in the engine's `llm-router` configuration entry, provider key `merge-gateway`:

```yaml
llm-router:
  providers:
    merge-gateway:
      api_key: "YOUR_MERGE_GATEWAY_API_KEY"   # or leave unset and export MERGE_GATEWAY_API_KEY
      # api_url: "https://api-gateway.merge.dev/v1/openai/chat/completions"  # default
```

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
