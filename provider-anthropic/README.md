# provider-anthropic

Claude models behind [llm-router](https://github.com/iii-hq/workers/tree/main/llm-router). Install this worker next
to the router, give it an API key, and the Anthropic catalog — streaming,
extended thinking, tool use, vision, automatic prompt caching — appears
behind the router's single front door (`router::chat` / `router::complete`).
You never call this worker directly: the router invokes it worker-to-worker,
and `iii-permissions.yaml` blocks agent access to `provider::anthropic::*`.

## Install

```bash
iii worker add provider-anthropic
```

The provider does nothing on its own — it plugs into the router:

```bash
iii worker add llm-router
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker the next time it boots.

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
| `III_WS_URL` | `ws://127.0.0.1:49134` | engine WebSocket to attach to when `--url` is not set |

The binary also takes the standard worker CLI flags: `--url` (engine
WebSocket), `--manifest` (print the registry manifest and exit), and
`--config` (accepted but ignored with a warning — provider config comes
from the `llm-router` configuration entry).

Prompt caching needs no setup: markers go on the system prompt, the tools
tail, and the last stable assistant turn whenever the prefix is big enough
to be worth a cache write.

## Models

The catalog slice is live `GET /v1/models` merged with a curated capability
snapshot — context windows, output ceilings, thinking budgets, pricing
(USD per MTok). Live ids the snapshot doesn't know get conservative
defaults; curated aliases the API doesn't enumerate are kept, so the catalog
has no cold hole before first discovery. The snapshot lives in
[`src/curated.rs`](src/curated.rs) — update it against models.dev when
Anthropic ships new models; discovery only supplies bare ids.

## Notes

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
