# bridge

Connects a local iii engine to a remote iii instance over a long-lived
`iii-sdk` WebSocket connection so functions on either side can call across the
boundary. `expose` entries make a local function callable *from* the remote
engine; `forward`/`invoke` entries make a remote function callable *from* this
engine. There are no trigger types.

## Install

```bash
iii worker add bridge
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker the next time it boots.

## Function surface

| Function | Input | Output |
|---|---|---|
| `bridge.invoke` | `{ function_id, data?, timeout_ms? }` | remote function's return value, waits up to `timeout_ms` (default `30000`) |
| `bridge.invoke_async` | `{ function_id, data?, timeout_ms? }` | `null`, immediately — fire-and-forget; `timeout_ms` is ignored |
| forward entry (`local_function`) | whatever the remote `remote_function` expects | remote function's return value, waits up to the entry's `timeout_ms` (default `30000`) |
| expose entry (registered on the remote, name defaults to `local_function`) | whatever the local `local_function` expects | local function's return value; a local failure's real `code`/`message`/`stacktrace` is forwarded untouched |

`bridge.invoke`/`bridge.invoke_async`/forward calls collapse any remote-side
failure to `code: "bridge_error"` — the underlying remote error code is never
surfaced through those three paths. A malformed `bridge.invoke*` input
(missing `function_id`) fails with `code: "deserialization_error"` instead.

## Configuration

Configuration is owned by the `configuration` worker — edit it from the
console (**Configuration → Workers → bridge**) or seed it once via
`--config <file>.yaml` on first boot:

| Field | Default | Description |
|---|---|---|
| `url` | `ws://0.0.0.0:49134` | Remote engine WebSocket URL. Fallback chain: `config.url` → `III_URL` env var → the default above. |
| `expose[]` | `[]` | `{ local_function, remote_function? }` — local functions the remote engine may call; `remote_function` is the name registered on the remote (defaults to `local_function`). |
| `forward[]` | `[]` | `{ local_function, remote_function, timeout_ms? }` — local aliases that proxy outbound to a remote function; `timeout_ms` overrides the per-call default (`30000`). |

Hot-reload semantics: a `url` change connects a new remote client, re-registers
every `expose` entry on it, then swaps it in and gracefully shuts the old
client down — no restart needed. An unchanged `url` with new `expose`/`forward`
entries registers just the additions live. **Removing** a `forward`/`expose`
entry does not un-register its handler — the SDK has no unregister — so the
function id stays live but its handler returns `bridge_error` until the worker
is restarted.

## Requires removing the legacy built-in bridge worker

The legacy built-in bridge worker registers the same function ids
(`bridge.invoke`, `bridge.invoke_async`, plus any configured forward/expose
names). Two workers registering the same function id on one engine collide —
whichever registers last wins — so this worker requires the legacy built-in to be
absent: omit it from the engine's `config.yaml` (a config that doesn't list a
worker won't run it). This is a duplicate-function-id collision, not a trigger
type conflict — `bridge` registers no trigger types.

On boot, this worker queries the engine for connected workers and refuses to
start with a clear error if the legacy built-in is still active, so a stale config
fails loudly instead of silently racing the built-in worker for ownership of
`bridge.invoke`/`bridge.invoke_async`.

The engine's own state/queue/stream/configuration **bridge adapters** are
unaffected by this worker or by removing the legacy built-in: those adapters open
their own direct SDK connections to bridge engine-internal subsystems and never
depended on the legacy built-in worker.

## Parity vs builtin

| Behavior | Builtin | This worker |
|---|---|---|
| `bridge.invoke` / `bridge.invoke_async` | exact paths, codes `deserialization_error`/`bridge_error` | same |
| Default timeout | 30s (`timeout_ms` overrides) | same |
| `invoke_async` result | `NoResult` (absent) | `null` (SDK functions must return a value) |
| Forward functions | one local function per entry, description `Forward to remote function {id}` | same |
| Expose functions | registered on the remote engine, name defaults to local, real error body forwarded | same |
| Remote URL | `config.url` → `III_URL` env → `ws://0.0.0.0:49134` | same |
| Trigger types | none | none |
| Config source | engine `config.yaml` (restart to change) | `configuration` worker entry `bridge`, hot-reload |
| Removing forward/expose entries | restart re-registers cleanly | entry disabled (handler errors); full removal needs worker restart |
