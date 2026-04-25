# iii-a2a-client

A2A (Agent-to-Agent) **client-side** worker for the iii engine. Where
[`iii-a2a`](../a2a) exposes the local engine's functions as A2A skills
*outbound*, this worker pulls in skills from any external A2A agent and
registers each one as a local iii function. Once registered, the rest of the
engine — workflows, harnesses, other workers — can call a remote skill the
exact same way they call any local function: `iii.trigger(<id>, payload)`.

That symmetry is what unblocks cross-protocol harness building. An iii
worker, an MCP server through `iii-mcp-client`, and a remote A2A agent
through this crate all show up in `iii.list_functions()` as plain old
function ids. The harness doesn't need to know the wire protocol underneath.

## How it works

1. On startup, fetch each `--connect <URL>`'s
   `<URL>/.well-known/agent-card.json`.
2. Derive a session name from `provider.organization + agent.name`
   (sanitised — lowercase, alnum-plus-underscore).
3. For every skill in the card, register a local iii function with id
   `<namespace-prefix>.<session_name>::<skill.id>`.
4. Each registration's handler is a closure that POSTs `message/send` to
   `<URL>/a2a` with a data part shaped as
   `{"function_id": <skill.id>, "payload": <handler input>}`.
5. On `Completed`, return the first artifact's first part — JSON-decoded if
   it's a `text` part with `mediaType: application/json`, otherwise the raw
   `data`/`text`.
6. A poll loop re-fetches the card every `--poll-interval` seconds and
   reconciles: registers new skills, calls `FunctionRef::unregister()` for
   skills that disappeared.

## CLI

```text
--engine-url <URL>            WebSocket URL of the iii engine
                              [default: ws://localhost:49134]
--connect <URL>               Base URL of an external A2A agent. Repeatable.
--namespace-prefix <PREFIX>   Local-function prefix [default: a2a]
--poll-interval <SECONDS>     Card refresh cadence [default: 30]
--debug                       Verbose logging
```

`--connect` accepts the agent's base URL (anything resolvable to
`<URL>/.well-known/agent-card.json` and `<URL>/a2a`). The `<URL>` is
trimmed of any trailing slash.

## Example

Bring up an `iii-a2a` agent locally on port 3111 (per the
[`iii-a2a` README](../a2a/README.md)). Then point the client at it:

```bash
./target/release/iii-a2a-client \
  --engine-url ws://127.0.0.1:49134 \
  --connect http://127.0.0.1:3111
```

In another shell, list the functions the local engine now knows about:

```bash
curl -s http://127.0.0.1:3111/api/introspect/functions | jq '.functions[].function_id'
```

You'll see the engine's own functions plus an `a2a.iii_iii_engine::*`
entry per skill the remote `iii-a2a` advertised. Trigger one:

```bash
curl -sX POST http://127.0.0.1:3111/api/iii/trigger \
  -H 'content-type: application/json' \
  -d '{"function_id":"a2a.iii_iii_engine::pricing::quote","payload":{}}'
```

## config.yaml

Currently no per-worker config file; everything is CLI-driven. Future
additions (auth headers, per-agent overrides) will land here.

## Limitations

- **Streaming requires the remote to advertise `capabilities.streaming`.**
  When unset, `Session::stream_message` returns a typed
  `"Remote agent does not advertise streaming capability"` error before
  opening the SSE stream — avoiding the opaque `EventSource::InvalidContentType`
  wrapper a JSON-only endpoint would otherwise produce. The registered
  handler always uses sync `message/send`; streaming is exposed only on
  the `Session` API for callers that want to drive it themselves.
- **Push-notifications receiver is deferred** to the Phase 3 push landing.
  No webhook endpoint is registered today.
- **Cancellation forwarding is deferred.** iii-sdk 0.11.3 does not expose
  a public hook for "this local invocation was cancelled by the engine,"
  so we have nowhere to call `tasks/cancel`. The bookkeeping (in
  `src/task.rs`) is in place; the wiring waits on a future SDK release.
- **`FunctionRef::unregister` race window.** The poll loop unregisters
  vanished skills before re-checking the card. If the same skill id
  re-appears the same poll tick, the engine may still see the old
  registration for ~50 ms. Re-registration retries on the next tick. If
  this matters in practice, increase `--poll-interval`.

## Tests

```bash
cargo test                    # mock-A2A round-trip (engine-free)
cargo test -- --ignored       # also exercise the live-engine path
                              # (requires ws://127.0.0.1:49134)
```

The engine-free tests boot a small `axum` mock A2A server in-process and
exercise `Session::send_message` and `Session::stream_message` against
it. The `#[ignore]`-gated tests additionally bring the iii engine into
the loop, registering skills and invoking them through `iii.trigger`.

## SDK + stack

- `iii-sdk =0.11.3`
- `reqwest 0.12` for the JSON-RPC POST
- `reqwest-eventsource 0.6` for `message/stream` SSE parsing
- `dashmap 6` for the skill-id → `FunctionRef` registry map
- `tokio 1`, `clap 4`, `serde`, `tracing`, `anyhow`
