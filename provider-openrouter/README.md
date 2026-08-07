# provider-openrouter

OpenRouter Chat Completions provider worker behind
[llm-router](https://github.com/iii-hq/workers/tree/main/llm-router): one API key in front of every major
vendor's models. OpenRouter's API is OpenAI Chat Completions-compatible, so
this worker forks the shared provider scaffolding; what makes it different
from the single-vendor providers is the catalog — OpenRouter's
`GET /api/v1/models` listing is self-describing (context windows, output
ceilings, per-token pricing, modalities, supported parameters, reasoning
efforts), so live discovery owns the entire record and there is no local
metadata table to maintain when upstream ships new models.

Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::openrouter::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::openrouter::refresh_models` (live `GET /api/v1/models`, admitted
rows mapped to full catalog records → `router::models::reconcile`).

## Behavior

- **Catalog ids are prefixed:** OpenRouter's own model ids are `vendor/model`
  (`anthropic/claude-sonnet-4.5`), which unprefixed would read as belonging
  to the sibling single-vendor providers. Catalog ids are therefore
  `openrouter/vendor/model`; the prefix is stripped on every upstream call.
- **Admission:** a model must support function `tools`, emit `text`, and be
  reachable over Chat Completions to be reconciled. The agent loop is
  unusable without tool calling; image or audio generators and
  batch-endpoint-only `:batch` variants (chat calls to them 404) would be
  dead rows in the picker. Everything else in the listing (several hundred
  models) lands in the catalog with live metadata.
- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The declaration carries no static `models` slice — the live listing is the
  source of truth and a refresh fires right after registration. Defaults:
  `api_url: https://openrouter.ai/api/v1/chat/completions`,
  `credential_env_var: OPENROUTER_API_KEY`.
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in iii-state (scope `provider-openrouter`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `OPENROUTER_API_KEY` env on the router → none). Both
  `api_key` and `oauth` credential shapes are sent as `Authorization:
  Bearer`; v1 performs no OAuth refresh.
- **Reasoning:** `thinking_level` maps to OpenRouter's unified
  `reasoning: {effort}` parameter, resolved against the model's advertised
  `supported_efforts` from the listing. Ladders step down, never up (a
  request can only get less reasoning than asked); a model that does not
  support the parameter gets no `reasoning` field and the caller is told via
  a report-and-continue warning. Reasoning deltas stream back as thinking
  blocks — OpenRouter's normalized `reasoning` field, with the older
  `reasoning_content` honored for compatible gateways pointed at via
  `api_url`.
- **Structured output:** strict `json_schema` mode on models whose listing
  declares `structured_outputs`; a schema requested for any other model
  degrades to `json_object` with a warning.
- **Usage accounting:** requests carry `usage: {include: true}`, so the
  final usage chunk reports native token counts, cache reads/writes,
  reasoning tokens, and the actual billed cost (cache discounts included).
  The billed cost lands on `usage.cost_usd` verbatim — the router's
  catalog-pricing fill keeps a provider-supplied value.
- **Token counting:** none. OpenRouter exposes no pre-request tokenizer
  endpoint, and one local tokenizer cannot honestly meter models spanning
  every vendor's vocabulary — `router::count_tokens` reports
  `no_token_counter` for this provider and the harness falls back to its own
  estimate.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request.
- **Errors:** OpenRouter's gateway-level status semantics map to the shared
  taxonomy: 401 → `auth_expired`; 402 (insufficient credits) and 403
  (moderation flagged the input) → `permanent`; 408 → `transient`; 429 →
  `rate_limited`; 502/503 (upstream down / no provider available) and
  network failures → `transient`; context-length errors →
  `context_overflow`. The numeric `error.code` envelope wins over the
  transport status; OpenAI-style string codes from compatibility proxies are
  honored too. No transport retries here — the router owns retry policy.
- **Attribution:** requests carry OpenRouter's optional app-attribution
  headers (`HTTP-Referer: https://iii.dev`, `X-OpenRouter-Title: iii`),
  identifying the harness stack, never the end user.

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
