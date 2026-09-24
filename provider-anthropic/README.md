# provider-anthropic

Claude models behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router). Install this worker next
to the router, give it an API key, and the Anthropic catalog — streaming,
extended thinking, tool use, vision, automatic prompt caching — appears
behind the router's single front door (`router::chat` / `router::complete`).
You never call this worker directly: the router invokes it worker-to-worker,
and `iii-permissions.yaml` blocks agent access to `provider::anthropic::*`.

## Install

```bash
iii trigger compose::add worker=provider-anthropic
```

The provider does nothing on its own — it plugs into the router:

```bash
iii trigger compose::add worker=llm-router
```

`iii trigger compose::add` resolves each worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Quickstart

Give the router a credential: paste a key into the `anthropic` slice of the
engine's `llm-router` configuration entry, or set `ANTHROPIC_API_KEY` in the
router's environment.

```json
{ "providers": { "anthropic": { "api_key": "sk-ant-…" } } }
```

The router picks up the change and kicks model discovery; Claude models land
in the catalog (`router::models::list`). Then make a first call — Node shown,
any SDK works:

```ts
const res = await iii.trigger('router::complete', {
  model: 'claude-sonnet-4-6',
  messages: [{ role: 'user', content: [{ type: 'text', text: 'Hello' }], timestamp: Date.now() }],
}, { timeout_ms: 320_000 }); // outer timeout ≥ the router's 300s stream budget
// res: { message, usage, provider, model } — message is the final AssistantMessage
```

For token-by-token streaming, call `router::chat` with an iii channel — the
walkthrough lives in [llm-router's Quickstart](https://github.com/iii-hq/workers/blob/main/llm-router/README.md).
Request extended thinking with `thinking_level`; the worker maps the level
to an Anthropic thinking budget from the model's catalog record. `xhigh`
degrades to `high` on models that don't support it, and the level is
dropped on models that don't support thinking at all.

## Configuration

All operator configuration lives in the router's `llm-router` entry — this
worker keeps no config of its own:

```jsonc
"anthropic": {
  "api_key": "sk-ant-…",                               // or ANTHROPIC_API_KEY in the router's env
  "api_url": "https://api.anthropic.com/v1/messages",  // override for proxies / gateways
  "max_tokens": 8192                                   // output ceiling when a request sets none
}
```

Worker-side environment variables:

| Variable | Default | Meaning |
|---|---|---|
| `PROVIDER_ANTHROPIC_CACHE` | enabled | `0`/`false` disables automatic prompt-cache markers |
| `PROVIDER_ANTHROPIC_CACHE_TTL` | `1h` | TTL of the shared-prefix markers on the sectioned path; `5m` restores the default cache |
| `III_WS_URL` | `ws://127.0.0.1:49134` | engine WebSocket to attach to when `--url` is not set |

The binary also takes the standard worker CLI flags: `--url` (engine
WebSocket), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).

Prompt caching needs no setup: markers go on the system prompt, the tools
tail, and the last stable assistant turn whenever the prefix is big enough
to be worth a cache write. When the router forwards `system_sections`, the
`system` field becomes one text block per section and the marker moves to the
block flagged `cache_boundary` (the frozen agent-profile prefix) whatever its
size — Anthropic applies its per-model token minimum over the whole prefix,
tools included, and silently skips a short one — so the per-session tail
after it no longer invalidates the shared entry. At most two system blocks are marked, keeping
the total at Anthropic's four. On that sectioned path the boundary block (and
the tools marker ahead of it) use the 1-hour cache (`ttl: "1h"`, 2x base on
the one write, reads unchanged) so a profile stays warm across sessions up to
an hour apart; the per-turn messages anchor keeps the 5-minute default, which
also satisfies the longer-before-shorter TTL rule. `PROVIDER_ANTHROPIC_CACHE_TTL=5m`
goes back to 5 minutes everywhere.

## Models

The catalog slice is **live** `GET /v1/models`: model ids, display names,
context windows, output ceilings, and the capability flags (adaptive
thinking, `xhigh` effort, vision) all come from the API on every refresh, so
a newly shipped model (Opus 5.5, Sonnet 5, Fable 5.1, …) appears in the
picker as soon as the API lists it — no code or SDK change required. Rows the
API marks as thinking-capable but *not* adaptive-capable (the pre-4.6
generation) are dropped because this provider only implements adaptive
thinking; the haiku family is the exception and stays with thinking gated off.

The API does not publish pricing, so that is the one hand-maintained table:
[`src/curated.rs`](src/curated.rs), USD per MTok keyed by base model id
(date suffixes stripped). A model missing there still routes and shows up;
it only loses cost enrichment, and the harness refuses `max_cost_usd`
budgets on it. Update the table against
[platform.claude.com/docs/en/about-claude/pricing](https://platform.claude.com/docs/en/about-claude/pricing)
when Anthropic ships or reprices a model.

## Notes

- **Token counting:** `provider::anthropic::count_tokens` (behind
  `router::count_tokens`) posts the assembled prompt to the messages
  endpoint's `count_tokens` sibling for an exact provider-metered count;
  it never runs the model and costs nothing.
- **Structured output:** the Messages API has no native JSON mode; every
  catalog record declares `supports_structured_output: false`, and a
  forwarded `response_format` is reported in `warnings` and ignored.
- **Errors:** 401/403 → `auth_expired`, 429 → `rate_limited`, 413/context →
  `context_overflow`, 5xx/network → `transient`, other 4xx → `permanent`.
  The worker never retries — the router owns retry policy.
- **Credentials:** resolved per request via `router::provider::resolve`
  (config slice → `ANTHROPIC_API_KEY` on the router → none). Both `api_key`
  (x-api-key) and `oauth` (Bearer) shapes work; v1 performs no OAuth refresh.
- **Identity binding:** the router issues a `registration_token` on first
  registration, persisted in state (scope `provider-anthropic`). If that
  state is lost the router rejects re-registration — clear the binding on
  the router side to re-pair.

## Tests

```bash
cargo test                                            # unit (pure modules + TCP stubs)
III_ENGINE_BIN=$(which iii) cargo test --test integration -- --test-threads=1
```

The integration suite spawns a real engine, the real router (path dep), this
provider, and a local stub upstream — no external API calls anywhere.
