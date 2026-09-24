# provider-openai

OpenAI Responses provider worker behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router), with a
Chat Completions compatibility path for custom gateways.
Implements the provider protocol from
`tech-specs/2026-06-agentic/llm-router.md`: `provider::openai::stream`
(SSE chunks → `AssistantMessageEvent` frames into a router-owned channel),
`provider::openai::refresh_models` (live `GET /v1/models` filtered to
chat/reasoning families ∪ curated capability snapshot →
`router::models::reconcile`), `provider::openai::embed` (batch text
embeddings behind `router::embed`; the endpoint derives from the configured
`api_url`, so OpenAI-compatible local servers — llama.cpp `--embeddings`,
Ollama, vLLM, LM Studio — and gateways work through the same surface), and
`provider::openai::count_tokens` (local prompt token estimation with the
tiktoken tokenizers behind `router::count_tokens`; never runs the model,
costs nothing, and needs no network).

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
  `prompt_cache_key` routes requests that share a prefix to one cache shard:
  a caller's `provider_options.openai.prompt_cache_key` wins, else a key
  derived from the router's `cache_intent.surface_digest` (the frozen
  agent-profile prefix, so independent sessions on one profile share a
  shard), else a key derived from
  the session id. On GPT-5.6 and later (official endpoint only) the router's
  `system_sections` go out as one developer message each, with
  `prompt_cache_breakpoint: {"mode":"explicit"}` on the block flagged
  `cache_boundary`: those models look up the cache only at message-level
  boundaries — explicit breakpoints, the implicit breakpoint on the latest
  eligible message, up to 20 earlier eligible message endings, and the end of
  the initial developer block — so a single system message ending with
  per-session text never matched across sessions (every new session paid a 1.25× cache write
  and read nothing). Implicit caching stays on for the conversation history;
  the key is accounting-only there. On the models that document extended
  retention (gpt-5.5, gpt-5.4, gpt-5.2, gpt-5.1*, gpt-5, gpt-5-codex, gpt-4.1
  and their dated snapshots, official endpoint only) every request sends
  `prompt_cache_retention: "24h"` so an entry lives up to a day instead of
  5-10 minutes, at no extra write charge; `PROVIDER_OPENAI_CACHE_RETENTION`
  (`in_memory` | `off`) overrides it. GPT-5.6 and later take
  `prompt_cache_options.ttl`, whose only value (30m) is already the default,
  so nothing is sent. `prompt_tokens_details.cached_tokens` lands on
  `usage.cache_read`.
- **Curated snapshot:** `src/curated.rs` carries windows / output ceilings /
  capability flags / pricing (USD per MTok). Update it against models.dev
  when OpenAI ships new models — discovery only supplies bare ids.
  GPT-6 Astra uses the [official model specifications](https://developers.openai.com/api/docs/models/gpt-6-astra):
  a 1,050,000-token context window and 128,000-token output ceiling. Its tiered
  pricing is intentionally omitted from the flat-rate catalog fields.

## Image generation

`provider::openai::image::generate` and `provider::openai::image::read` are
the **agent-callable** functions of this worker (images have no router front
door, and the calls touch none of the router's accounting). `generate` posts
to the Images API sibling of the configured `api_url`
(`…/v1/images/generations`) with the router-resolved credential and writes
the picture, at full resolution, to
`data/provider-openai/images/<stem>-<unix_ms>-<8 hex>.<ext>` (relative to the
Compose project directory, `III_COMPOSE_DIR`; the process cwd outside
Compose). The result names that file and carries **no image bytes**: base64
in a function result only inflates the conversation context.

```jsonc
// request
{ "model": "gpt-image-2.5-sunburst",      // or gpt-image-2.5-flare (anything else is rejected)
  "prompt": "a lighthouse at dusk, oil painting",
  "size": "1536x1024",                     // 1024x1024 | 1536x1024 | 1024x1536 | auto
  "quality": "high",                       // low | medium | high | auto
  "output_format": "png",                  // png (default) | jpeg | webp
  "background": "opaque",                  // transparent | opaque | auto
  "file_name": "lighthouse" }              // optional stem for the saved file

// response — the same envelope as browser::screenshot, minus the picture
{ "content": [
    { "type": "text", "text": "Image generated by gpt-image-2.5-sunburst (1536x1024, 1834021 bytes, image/png) saved to /…/data/provider-openai/images/lighthouse-1790171000000-3f9a2c1e.png; view with provider::openai::image::read { path }" } ],
  "details": { "model", "path", "mime", "bytes", "size", "quality", "width", "height", "inline": "none", "revised_prompt?", "usage?" } }
```

**Preview in the chat.** This worker ships an injected console renderer
(`ui/`, served through `src/ui.rs`): the chat card for a `generate` call
fetches a downscaled JPEG from the worker itself through `image::read` and
shows it beside the file path (copyable), dimensions, size and model — the
bytes travel worker → console, never through the model.

**Looking at the picture from an agent.** `image::read { path, variant? }`
takes the `details.path` a generate call returned (or the bare file name)
and returns viewable content blocks: `variant: "preview"` (default; a JPEG
≤512px / ~80 KB, which fits the harness's 256 KiB result cap — base64 is
counted twice there, content + details) or `variant: "full"`. Anything
outside `data/provider-openai/images/` is refused, symlinks included. For
one-shot use, `generate` also accepts `inline: "preview" | "full"` to embed
the picture in its own result.

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
