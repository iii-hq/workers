# provider-groq

Groq Chat Completions provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::groq::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::groq::refresh_models` (upstream `GET /models`, enriched with local
metadata → `router::models::reconcile`), and `provider::groq::count_tokens`
(prompt token counting behind `router::count_tokens`).

Default upstream: `https://api.groq.com/openai/v1/chat/completions`. Override
`api_url` to point at any other OpenAI-compatible endpoint; the models listing
is always read from that endpoint's `/models` sibling.

## Behavior

- **Registration:** self-declares via `router::provider::register` with backoff
  until acked, and re-declares on the `router::ready` trigger type. The
  declaration carries no models and `credential_env_var: GROQ_API_KEY`; the
  post-register refresh discovers the catalog, gated on a configured credential
  (no key → empty slice, so the picker never shows unusable rows).
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in state (scope `provider-groq`, key
  `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `GROQ_API_KEY` env on the router → none), sent as
  `Authorization: Bearer`.
- **Catalog:** `GET /models` owns the id list, and unusually for a provider it
  also reports `context_window` and `active` per model. Both are taken live: a
  window Groq raises reaches the router without a release, and a model that is
  not serving is not offered, because a row the picker cannot use is worse than
  no row. `src/curated.rs` supplies what the listing cannot — display names,
  output ceilings, capabilities and pricing. An id the table does not know
  still lands in the catalog on conservative defaults rather than disappearing,
  so a model Groq ships tomorrow is routable today.

  Speech and moderation models share the listing with chat models. They have no
  chat completion surface, so they are dropped; modality is what tells them
  apart, since a speech model reports a context window like everything else.

  The listing is also per account: an Enterprise-gated model is simply absent
  for a key without access to it, which is the argument for reading it rather
  than shipping a table that would offer models an operator cannot reach.

  `groq/compound` and `groq/compound-mini` are systems rather than models: a
  collection of models and tools that Groq runs together, with web search and
  code execution of their own. They report no `tools` feature, so the catalog
  marks them `supports_tools: false` and they will refuse a request carrying
  function definitions. That is the system declining to take someone else's
  tools, not a capability gap to route around.
- **Pricing:** comes from the listing, which quotes a per-token rate per model;
  the catalog carries USD per MTok, so each is scaled and rounded. A rate that
  will not parse is dropped rather than guessed at, because a wrong number on a
  cost display is worse than no number. Nothing about pricing is kept locally:
  a hand-maintained table beside a live one would go stale in silence.
- **Token counting:** Groq is an inference host, so a Llama, a GPT-OSS and a
  Qwen model sit behind one endpoint with three different tokenizers between
  them. The vocabulary is therefore chosen per model rather than per provider,
  and a model no rule recognizes is answered with a typed `no_token_counter`
  rather than a borrowed vocabulary that would read as authoritative while
  being wrong. Meta's repositories are gated behind a licence click a worker
  cannot perform, so Llama resolves through a public mirror of the identical
  tokenizer. Counting is local, needs no credential, and costs nothing.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited`, 413 and
  `context_length_exceeded` → `context_overflow`, **498 (flex-tier capacity
  exhausted) → `transient`** because the same request may be served later,
  **499 (caller cancelled) → `permanent`** because retrying would resurrect
  work somebody deliberately stopped, 500/502/503 and network → `transient`.
  No transport retries here: the router owns retry policy.
- **Reasoning:** the GPT-OSS models take `reasoning_effort`; the Llama models
  do not reason at all. That distinction does not arise at a single-family
  provider, and is why the catalog marks thinking per row rather than for the
  provider as a whole. Reasoning models stream their chain of thought as
  `reasoning_content` deltas, surfaced as `thinking` blocks (`src/sse.rs`),
  and `completion_tokens_details.reasoning_tokens` lands on `usage.reasoning`.
- **Structured output:** the OpenAI-compatible surface takes `response_format`,
  json_schema included.

## Configuration

Credentials and `api_url` live in the router's `llm-router` configuration
entry under `providers.groq`, not in this worker's own config.

```yaml
providers:
  groq:
    api_key: gsk_…
```
