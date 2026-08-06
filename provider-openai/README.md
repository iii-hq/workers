# provider-openai

OpenAI Responses provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router), with a
Chat Completions compatibility path for custom gateways.
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::openai::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::openai::refresh_models` (live `GET /v1/models` filtered to
chat/reasoning families ∪ curated capability snapshot →
`router::models::reconcile`), and `provider::openai::embed` (batch text
embeddings behind `router::embed`; the endpoint derives from the configured
`api_url`, so OpenAI-compatible local servers — llama.cpp `--embeddings`,
Ollama, vLLM, LM Studio — and gateways work through the same surface).

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The model slice is populated from live discovery and the declaration carries
  `credential_env_var: OPENAI_API_KEY`.
- **Transport:** the default `https://api.openai.com/v1/responses` endpoint
  uses Responses items and typed streaming events. An explicitly configured
  endpoint that does not end in `/responses` keeps the Chat Completions wire
  format for compatible gateways that have not migrated.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in state (scope `provider-openai`,
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
- **Reasoning:** `thinking_level` maps to the selected transport's reasoning
  field per model
  family (`src/reasoning.rs` — the ladders encode real 400s: o1 and
  chat-tuned variants take no param, pro is high-only, xhigh is gpt-5.2+).
  Responses reasoning summaries are relayed as thinking blocks. Chat
  Completions still exposes only `completion_tokens_details.reasoning_tokens`
  as `usage.reasoning`. Luna requests with function tools on the legacy path
  force `reasoning_effort: none`, matching that endpoint's compatibility rule.
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
