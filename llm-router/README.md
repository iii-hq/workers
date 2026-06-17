# llm-router

One front door for every LLM provider. The router owns routing, the provider
registry, credential resolution, the model catalog, streaming relay, retries,
and a single failure contract — consumers call one chat surface and never talk
to a provider directly.

llm-router is a standalone iii worker. Providers plug in as separate workers
at runtime through a self-registration protocol (`iii worker add
provider-<x>`); the router never compiles against a provider, and removing a
provider worker removes the provider.

## Install

```bash
iii worker add llm-router
```

## Quickstart

A consumer streams a turn by creating an iii channel, handing the router the
channel's **write** endpoint, and reading frames from the **read** endpoint
while `router::chat` runs. Any SDK works; Node shown:

```ts
import { createChannel } from 'iii-sdk';

const { reader, writerRef } = await createChannel(iii);
reader.onMessage((frame) => {
  const event = JSON.parse(frame); // AssistantMessageEvent
  if (event.type === 'text_delta') process.stdout.write(event.delta);
});

const res = await iii.trigger('router::chat', {
  writer_ref: writerRef, // direction "write"
  model: 'claude-sonnet-4',
  messages: [{ role: 'user', content: [{ type: 'text', text: 'Hello' }], timestamp: Date.now() }],
}, { timeout_ms: 320_000 }); // outer timeout ≥ the router's 300s stream budget
// res: { ok, provider, model, stop_reason, usage }
```

The streaming contract: every stream ends with exactly one terminal frame
(`done` or `error`). When the router has to kill a stream itself (idle
timeout, provider crash), it synthesizes the terminal frame and attaches the
partial content, so consumers never hang on a half-open stream.

## Functions

### Consumer surface

| Function | Purpose |
|---|---|
| `router::chat` | Stream a turn into the caller's channel; returns the turn summary. |
| `router::complete` | Non-streaming convenience over the same pipeline; returns the final message. |
| `router::abort` | Cancel an in-flight turn by `request_id`. |
| `router::route` | Read-only routing preview: `{model, provider?}` → `{provider, candidates}`, same rules and error codes as `router::chat`. Pin the result as the explicit `provider` on the chat call when you need the provider before streaming. |
| `router::models::list` | List catalog models, filterable by `provider` / `capability`. |
| `router::models::get` | Fetch one model record (`null` when unknown). |
| `router::models::supports` | Check one capability flag for one model. |
| `router::provider::list` | Registered providers with `configured` / `available` status. |

Agent exposure is restricted per `iii-permissions.yaml` to the read surface
(`router::models::*`, `router::provider::list`).

### Provider protocol

Token-gated after the first declare: the response to `register` carries a
registration token, and every later protocol call must present it.

| Function | Purpose |
|---|---|
| `router::provider::register` | Self-declaration at attach time; idempotent re-declare with the token. |
| `router::provider::resolve` | Per-request credential + endpoint resolution (config > env > none). |
| `router::provider::update_credential` | Persist a refreshed credential (OAuth write-back). |
| `router::models::reconcile` | Replace the provider's catalog slice in one write. |

The provider worker itself exposes `provider::<id>::stream` and, when it
supports model discovery, `provider::<id>::refresh_models`.

## Configuration

All operator configuration lives in the engine's `llm-router` configuration
entry — no env vars, no config file. The entry schema is composed at runtime
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

| Setting | Default | Meaning |
|---|---|---|
| `stream_timeout_ms` | `300000` | Hard budget for one streamed turn. |
| `idle_timeout_ms` | `120000` | Max silence between provider frames before the attempt is cut. |
| `retry_max` | `2` | Retries per turn for retryable failures before the first forwarded frame. |
| `output_token_max` | `32000` | Ceiling on `max_output_tokens` forwarded to providers. |

Pasting a key into a provider's slice is the whole onboarding flow: the
router diffs the changed slice, debounces ~2 s, and kicks that provider's
`provider::<id>::refresh_models` discovery; discovered models land in the
catalog via `router::models::reconcile` and show up in `router::models::list`
within seconds — no restart.

### Operational notes

- **Env-var credential fallback resolves in the router's process.** A
  provider's `credential_env_var` (e.g. `ANTHROPIC_API_KEY`) is read by the
  llm-router binary, not by the provider worker — launch the router with
  those variables set, or put keys in the entry. A key present only in
  another worker's environment shows up as `configured: false`.
- **Registration-token recovery.** Re-registering a provider id without its
  original token is rejected (anti-takeover). If a provider durably lost its
  token, delete the router's registry state (iii-state scope `llm-router`,
  key `registry`) and restart the affected providers to re-bind; pasted
  credentials in the configuration entry are unaffected.

## Events

The router registers three custom trigger types and fans out to every bound
handler. Bind with the standard two-step pattern; the handler receives the
payload verbatim (no envelope).

| Trigger type | Fires when | Payload |
|---|---|---|
| `router::models::changed` | a provider reconciles its catalog slice | `{ "provider": "<id>", "count": <n> }` |
| `router::provider::changed` | the registry changes (declare / availability flip) | `{ "provider": "<id>", "op": "register" \| "available" \| "unavailable" }` |
| `router::ready` | the router finishes booting; providers re-declare on it | `{}` |

```ts
iii.registerFunction({ id: 'my-worker::onModelsChanged' }, async (payload) => {
  console.log('catalog changed:', payload); // { provider, count }
  return {};
});

iii.registerTrigger({
  type: 'router::models::changed',
  function_id: 'my-worker::onModelsChanged',
  config: {},
});
```

## Writing a provider worker

A provider worker must:

1. Register `provider::<id>::stream` honouring the channel-writer contract:
   forward upstream output as `AssistantMessageEvent` frames into the
   `writer_ref` it receives, ending with one terminal frame.
2. Declare itself at startup via `router::provider::register` — retrying with
   backoff until acknowledged (covers provider-before-router boot order) —
   and re-declare on the `router::ready` event after a router restart.
3. Resolve credentials per request via `router::provider::resolve`; never
   read keys directly.
4. Treat closure of its stream channel as cancellation: abort the upstream
   request and stop writing frames.
5. Map upstream failures to the shared `ErrorKind` taxonomy on its `error`
   frames. Transport retries (429 / 5xx / connect) are the router's job, not
   the provider's.

The first real provider implementing this protocol is
[`provider-anthropic/`](../provider-anthropic/) — useful as a reference
implementation alongside the scripted provider in the integration tests.
[`provider-openai/`](../provider-openai/) follows the same structure for the
OpenAI Chat Completions API (native structured output, reasoning_effort).

## Local development & testing

```bash
cargo test                       # unit suite, no engine needed
cargo test --test integration    # engine-backed suite; self-skips without an engine
```

The integration suite spawns a throwaway engine per test when `iii` is on
`PATH` (or `III_ENGINE_BIN` points at a binary) and covers the chat relay,
cancellation, abort, restart recovery, registration token gating, paste-a-key
discovery, and event delivery end to end.

To run the worker locally against an engine:

```bash
cargo run -- --url ws://127.0.0.1:49134
```

`--url` defaults to `ws://127.0.0.1:49134` and honours the `III_WS_URL`
environment variable when the flag is not set. `--config` is accepted per
the standard worker CLI but ignored with a warning — operator config lives
in the engine's `llm-router` configuration entry (see Configuration above).
