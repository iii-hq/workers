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
  chat completion surface, so they are dropped; the absence of a context window
  is what tells them apart. A gateway that reports no windows for anything is
  left alone, since requiring the field there would empty the catalog.
- **Pricing:** Groq's pricing page renders its figures client-side and ships
  none in the document, so these rows come from published third-party tracking
  rather than from Groq directly. Worth re-checking before anyone leans on the
  cost display.
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
