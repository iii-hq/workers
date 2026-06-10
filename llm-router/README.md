# llm-router

One front door + provider protocol in front of every LLM provider: routing,
provider registry, credential resolution, model catalog, and a single failure
contract. llm-router is a standalone iii worker — it has no dependency on any
harness, and LLM providers plug in as separate workers at runtime through the
self-registration protocol (`iii worker add provider-<x>`; the router never
compiles against a provider).

Spec: `tech-specs/2026-06-agentic/llm-router.md`. Shared wire contracts:
`tech-specs/2026-06-agentic/README.md` § Cross-cutting contracts.

## Install

```bash
iii worker add llm-router
```

## Quickstart

Consumers stream a turn by creating an iii channel, handing the router the
channel's **write** endpoint, and reading frames from the **read** endpoint
while the `router::chat` iii function runs. Any SDK works; Node shown:

```ts
import { createChannel } from 'iii-sdk';

const { reader, writerRef } = await createChannel(iii);
reader.onMessage((frame) => {
  const event = JSON.parse(frame); // AssistantMessageEvent (15-variant union)
  if (event.type === 'text_delta') process.stdout.write(event.delta);
});

const res = await iii.trigger('router::chat', {
  writer_ref: writerRef, // direction "write"
  model: 'claude-sonnet-4',
  messages: [{ role: 'user', content: [{ type: 'text', text: 'Hello' }], timestamp: Date.now() }],
}, { timeout_ms: 320_000 }); // outer timeout ≥ the router's 300s stream budget
// res: { ok, provider, model, stop_reason, usage }
```

Every stream ends with exactly one terminal frame (`done` or `error`); a
stream the router has to kill (idle timeout, provider crash) gets a
synthesized terminal carrying the partial content. `router::complete` is the
non-streaming convenience over the same pipeline; `router::abort`
(`{ request_id }`) cancels an in-flight turn.

## Configuration

All operator configuration lives in the engine's `llm-router` configuration
entry (no env vars, no config file). The entry schema is composed at runtime
from each registered provider's declaration:

```json
{
  "default_provider": "anthropic",
  "providers": {
    "anthropic": { "api_key": "sk-…", "api_url": "https://api.anthropic.com/v1/messages", "max_tokens": 8192 }
  },
  "routing_heuristics": [{ "pattern": "^gpt-", "provider": "openai" }],
  "settings": {
    "stream_timeout_ms": 300000,
    "idle_timeout_ms": 120000,
    "retry_max": 2,
    "output_token_max": 32000
  }
}
```

Pasting a key into a provider's slice is the whole onboarding flow: the router
diffs the changed slice, debounces ~2s, and kicks that provider's
`provider::<id>::refresh_models` discovery; discovered models land in the
catalog via `router::models::reconcile`.

## Migration notes

In the previous-generation harness, provider credentials/settings lived inside
the single `harness` configuration entry alongside its `permissions` block.
Only the provider credentials/settings move to this `llm-router` entry — the
permissions block stays in the harness's own entry. Neither may be silently
dropped during migration.

## Custom trigger types

The router owns three custom iii trigger types; bind an iii function to them
to react:

| Trigger type | Fires when | Payload |
|---|---|---|
| `router::models::changed` | a provider reconciles its catalog slice | `{ "provider": "<id>", "count": <n> }` |
| `router::provider::changed` | the registry changes (declare / availability flip) | `{ "provider": "<id>", "op": "register" \| "available" \| "unavailable" }` |
| `router::ready` | the router finishes booting; providers re-declare on it | `{}` |

## Function surface

Consumer: `router::chat`, `router::complete`, `router::abort`,
`router::models::{list,get,supports}`, `router::provider::list`.
Provider protocol (token-gated after first declare):
`router::provider::register`, `router::provider::resolve`,
`router::provider::update_credential`, `router::models::reconcile`; the
provider worker itself exposes `provider::<id>::stream` and (optionally)
`provider::<id>::refresh_models`. Agent exposure is restricted per
`iii-permissions.yaml` — the read surface only.
