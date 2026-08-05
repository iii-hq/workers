# slack — architecture

Two surfaces in one worker:

1. **Slack Web API surface** — every meaningful Slack method registered as a
   typed `slack::*` function, plus a generic `slack::call` escape hatch. Pure
   outbound HTTPS to `api.slack.com`; needs only a bot token; no ingress.
2. **Harness bridge** — inbound Slack events become `harness::send` turns whose
   output is streamed back into the thread. Additive: active only when a public
   engine URL and signing secret are configured and the harness stack is present.

## Module map

| Module | Role |
|---|---|
| `config.rs` | `WorkerConfig` schema (tokens, scoping, bridge settings); env expansion; `bridge_enabled()` |
| `configuration.rs` | `configuration::register` + Tier-1 hot-reload; reconciles ingress on change |
| `clients/slack.rs` | Slack Web API client (`call`, `call_user`, `auth_test`) |
| `clients/{harness,router,state,approval}.rs` | engine sibling clients (bridge) |
| `functions/*` | typed `slack::*` Web API functions + `notify`, `events`, `interactions`, `bindings` |
| `response.rs` | `SlackResponse` typed response wrapper |
| `httpio.rs` | raw HTTP over engine channels (request_body read channel + response write channel); signature gate |
| `signing.rs` | Slack v0 HMAC verification (constant-time, replay window) |
| `dispatch.rs` | event routing + mention gating + context capture; interaction → approval |
| `turn.rs` | model resolution, thread backfill, `harness::send` |
| `stream.rs` | native `chat.startStream`/`appendStream`/`stopStream` (+ `chat.update` fallback) |
| `kv.rs` | thread↔session mapping, pending context buffer, approval callbacks (state) |
| `ingress.rs` | register/unregister the events + interactions HTTP routes per config |
| `surface.rs` | typed function catalog for golden schema tests |

See [`internals.md`](internals.md) for the request/turn/stream flows and
[`configuration.md`](configuration.md) for the config surface.

## Slack API notes

Slack-specific facts the implementation relies on (Socket Mode is intentionally
not used; signing requires the raw body; `files.upload` is sunset) are recorded
in [`slack-api.md`](slack-api.md).
