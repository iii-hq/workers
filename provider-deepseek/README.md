# provider-deepseek

DeepSeek Chat Completions provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router).
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::deepseek::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel) and
`provider::deepseek::refresh_models` (upstream `GET /models`, enriched with
local metadata → `router::models::reconcile`).

Default upstream: `https://api.deepseek.com/chat/completions` — DeepSeek's
OpenAI-compatible surface is rooted at the bare host, with no `/v1` segment.
Override `api_url` to point at any other OpenAI-compatible endpoint; the
models listing is always read from that endpoint's `/models` sibling.

## Behavior

- **Registration:** self-declares via `router::provider::register` with
  backoff until acked, and re-declares on the `router::ready` trigger type.
  The declaration carries no models and
  `credential_env_var: DEEPSEEK_API_KEY`; the post-register refresh
  discovers the catalog, gated on a configured credential (no key → empty
  slice, so the picker never shows unusable rows).
- **Identity binding:** the router returns a `registration_token` on first
  registration; it is persisted in state (scope `provider-deepseek`,
  key `registration_token`) and presented on every later
  `register`/`resolve`/`reconcile`. If that state is lost the router rejects
  re-registration — the operator must clear the binding on the router side.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `DEEPSEEK_API_KEY` env on the router → none). Both
  `api_key` and `oauth` credential shapes are sent as `Authorization:
  Bearer`; v1 performs no OAuth refresh.
- **Catalog:** `GET /models` owns the id list; `src/curated.rs` supplies what
  the listing does not carry — display names, context windows, output
  ceilings, and pricing (USD per MTok) from api-docs.deepseek.com. An id the
  table does not know still lands in the catalog on conservative defaults
  (64K context, 8K output, no pricing) rather than disappearing, so a model
  DeepSeek ships tomorrow is routable today. The prices are DeepSeek's
  regular rates, which is what is billed today, so reported cost is exact.
  DeepSeek has announced a peak/off-peak policy charging 2x during
  09:00–12:00 and 14:00–18:00 Beijing time but has not set an effective
  date; if it lands, cost display becomes a floor during those windows,
  since a time-of-day multiplier is not something the catalog can express.
- **Liveness:** `ping` at least every 30s of upstream silence; a failed
  channel write (caller gone / `router::abort`) drops the SSE receiver and
  aborts the in-flight HTTP request. DeepSeek holds an overloaded request
  open with `: keep-alive` SSE comments, which the decoder ignores.
- **Errors:** 401/403 → `auth_expired`, **402 (out of balance) →
  `permanent`** (a billing wall retry cannot fix), 429 → `rate_limited`,
  400/422 → `permanent` unless the message says the prompt did not fit,
  `context_length_exceeded` → `context_overflow`, 500/503 and network →
  `transient`. `finish_reason: insufficient_system_resource` — the upstream
  running out of capacity mid-generation — terminates the stream as a
  `transient` error so the router retries instead of returning a silently
  truncated answer. No transport retries here: the router owns retry policy.
- **Structured output:** `json_object` mode only — DeepSeek documents no
  strict json_schema mode. A `response_format` schema is dropped with a
  report-and-continue warning; every catalog record declares
  `supports_structured_output: false`. The caller must mention "json" in the
  prompt per DeepSeek's rules.
- **Reasoning:** with a requested thinking level the request carries
  `thinking: {type: enabled}` plus the top-level `reasoning_effort` param —
  the router's five levels collapse onto DeepSeek's three-wide vocabulary as
  `minimal`/`low` → `low`, `medium`/`high` → `high`, `xhigh` → `max`
  (`src/reasoning.rs`). With **no** level both params are omitted, so each
  model runs its own documented default: the V4 family reasons at `high`
  effort — an unconfigured console chat streams its chain of thought out of
  the box, with reasoning tokens billed as output — while a legacy
  non-thinking alias (`deepseek-chat`) keeps the behavior its name encodes.
  `disabled` is never sent: the router has no off level to express, and a
  synthetic off-by-default would blank the console's thinking pane.
  Reasoning models stream their chain of thought as `reasoning_content`
  deltas, which the worker surfaces as `thinking` blocks (`src/sse.rs`), and
  `completion_tokens_details.reasoning_tokens` lands on `usage.reasoning`.
  Assistant thinking is **replayed** to the API as `reasoning_content` only
  on tool-calling messages — DeepSeek 400s a tool round whose intermediate
  reasoning was dropped — and nowhere else: the API documents replayed
  reasoning as ignored outside tool rounds, where it would only re-bill the
  whole chain as input on every later turn.
- **Block ordering:** the assembled message is a *sequence* of blocks in the
  order the model produced them, not one merged block per kind. A turn that
  reasons, answers, reasons again and answers again lands as four blocks, and
  a tool call sits between the blocks it actually fell between — matching what
  the `thinking_start`/`text_start`/… frames already described on the wire, and
  the same shape `provider-anthropic` assembles. DeepSeek's own response
  carries one `reasoning_content` and one `content` per message, so in practice
  a turn is one thinking block then one text block; the ordering matters behind
  an `api_url` override onto a gateway that genuinely interleaves. On the
  turns that replay at all (tool-calling ones — see Reasoning), replay
  flattens the blocks back to the two scalar fields the request shape has
  (thinking blocks joined, text blocks joined) — the only representation
  DeepSeek accepts.
- **Text only:** DeepSeek documents no multimodal content-part array, so
  image blocks are replaced with a text marker instead of being sent as
  `image_url` parts (which would fail the whole turn); the caller gets a
  report-and-continue warning and every known catalog record declares
  `supports_vision: false`.
- **Prompt caching:** on by default for every account, with no request
  markers (context caching on disk). Hits land on shared request *prefixes*,
  so the wire layer keeps a session's transcript append-only by
  construction: every serialization rule depends only on a message's own
  content (never on what follows it), which makes turn N's request body a
  byte-stable prefix of turn N+1's; ignorable reasoning is dropped instead
  of resent (see Reasoning); and no `user_id` is sent, so all traffic shares
  one account-wide cache instead of partitioning it. The two deliberate
  exceptions mutate history for correctness and cost one cache bust each: a
  late tool result replacing its orphan placeholder, and latest-wins dedup
  of a duplicated result. DeepSeek reports the resulting prompt split
  directly, and it maps straight onto the spec's disjoint usage splits:
  `prompt_cache_hit_tokens` → `usage.cache_read` (billed at the cached-input
  rate) and `prompt_cache_miss_tokens` → `usage.input` (billed at the input
  rate). Because the router's cost fill adds the two, `input` is deliberately
  the miss slice rather than the `prompt_tokens` total — with a ~120x cache
  discount, billing the cached prefix at both rates would roughly double the
  reported cost of a long agent loop.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream serving both `/models` and
`/chat/completions` — no external API calls anywhere.

## Running

The binary takes the standard worker CLI flags: `--url` (engine WebSocket,
default `ws://127.0.0.1:49134`, falls back to the `III_URL` environment
variable), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).
