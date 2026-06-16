# session-manager

Durable, reactive store for conversations. A session is an append-only
log of typed message entries (user, assistant, function_result, custom
— with optional fork branches) plus a small metadata record (title,
description, coarse status, app-defined metadata). Any worker or client
appends and reads over the bus; every mutation fires a trigger type
consumers bind to, so a chat UI, bot, or dashboard renders live with no
polling and no separate publish call. It runs no agent logic — it is
the conversation database the [harness](../harness) family drives, and
it is independently useful as a real-time chat store for any app.

## Install

```bash
iii worker add session-manager
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

## Quickstart

Create a session, append a message, read the transcript back:

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let created = iii.trigger(TriggerRequest {
        function_id: "session::create".into(),
        payload: json!({ "title": "Weather question", "metadata": { "owner": "u_1" } }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;
    let session_id = created["session_id"].as_str().unwrap().to_string();

    iii.trigger(TriggerRequest {
        function_id: "session::append".into(),
        payload: json!({
            "session_id": session_id,
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "What's the weather?" }],
                "timestamp": 1717800000000_i64
            }
        }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    let transcript = iii.trigger(TriggerRequest {
        function_id: "session::messages".into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: Some(5_000),
    }).await?;

    println!("{transcript:#?}");
    Ok(())
}
```

To render live instead of polling, register a handler function and bind
it to a trigger type (the two-step reactive pattern):

```rust
use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput};
use serde_json::{json, Value};

iii.register_function(
    "my-ui::on-message-updated",
    RegisterFunction::new_async(|event: Value| async move {
        // full updated message + monotonic revision; keep the highest
        println!("{} rev {}", event["entry_id"], event["revision"]);
        Ok::<_, IIIError>(json!({ "ok": true }))
    }),
);

iii.register_trigger(RegisterTriggerInput {
    trigger_type: "session::message-updated".into(),
    function_id: "my-ui::on-message-updated".into(),
    config: json!({ "session_id": session_id, "roles": ["assistant"] }),
    metadata: None,
})?;
```

Streaming an assistant reply uses the same primitives: append an
(initially empty) assistant message, then call `session::update-message`
as tokens arrive — each update fires `session::message-updated` with an
incremented `revision`.

The full function surface (14 `session::*` functions: lifecycle,
messages, branching/fork) is documented in the spec and on each
function's registered schema — see `iii worker info session-manager`.

## Custom trigger types

Six trigger types cover every mutation. Each binding's `config` filters
what reaches the handler; all configs additionally accept
`metadata` — a subset-equality match against `SessionMeta.metadata`
(the tenancy hook, e.g. `{ "owner": "u_1" }`). Malformed configs are
rejected at registration.

| Trigger type | Fires when | Config filters |
|---|---|---|
| `session::created` | A session is created (create / ensure / fork) | `metadata` |
| `session::message-added` | An entry is appended | `session_id`, `roles`, `metadata` |
| `session::message-updated` | A message's content changes (streaming deltas) | `session_id`, `roles`, `metadata` |
| `session::status-changed` | Status moves between idle/working/done/error | `session_id`, `metadata` |
| `session::meta-updated` | Title/description/metadata change | `session_id`, `metadata` |
| `session::deleted` | A session and its entries are removed | `session_id`, `metadata` |

Delivery is fire-and-forget, at-least-once, and unordered — reconcile
message updates by `revision` (keep the highest) and transcript order
by the parent chain, never by arrival order.

## Storage adapters

Two adapters, selected by an `adapter` block (a `name` plus a nested
`config`):

- **`fs`** (default) — one append-only JSONL file per session under
  `data_dir` (`<encoded_session_id>.jsonl`): typed `meta` / `entry` /
  `leaf` records, replayed last-wins on startup, file removed on
  session delete. This is the durable, single-instance setup.
- **`bridge`** — this instance keeps all domain logic (idempotency,
  revisions, branching, locks) but stores through a **main** instance
  running its own session-manager (`adapter name: fs`) on another iii
  engine, via the internal `session::store::*` protocol.

Event propagation in a bridge topology: the main is the single fan-out
point. A bridged instance publishes each mutation's events to the main
(`session::store::publish-events`); the main delivers to its own
subscribers and forwards an envelope to **every** attached bridged
instance over its internal `session::store::events` feed; each bridge
re-emits through its local trigger types with its own subscribers'
filters. With several bridged instances attached to one main, a
mutation made anywhere reaches every instance's subscribers exactly
once. (Consequence: an originator's own subscribers hear events after
the round trip through the main — same at-least-once, unordered
contract as always.)

The `session::store::*` functions are deployment plumbing served only
by fs-mode instances — deny them to agents like every other mutating
surface.

## Configuration

Runtime settings live in the **`configuration` worker** under id
**`session-manager`**. At startup the worker registers its JSON Schema,
fetches the live, env-expanded value via `configuration::get`, and binds a
`configuration` trigger so changes apply without a restart. Persisted values
default to `./data/configuration/session-manager.yaml` (the configuration
worker's `fs` adapter) — edit that file directly or call `configuration::set`
and the change propagates. `adapter` is an adjacently tagged enum, so the
console's worker-config form renders a variant picker (`fs` / `bridge`) and
only the selected adapter's `config` fields (`data_dir`, or `url` /
`timeout_ms`), plus the list limits — all as editable inputs.

```yaml
adapter:
  name: fs                                   # fs | bridge
  config:
    data_dir: ~/.iii/data/session-manager    # fs: one <session_id>.jsonl per session

# adapter:
#   name: bridge
#   config:
#     url: ws://127.0.0.1:49134   # main engine WebSocket URL
#     timeout_ms: 5000            # per store/publish call timeout

default_list_limit: 50   # page size when list/messages omit `limit`
max_list_limit: 500      # hard cap on any requested `limit`
```

**Reload policy.** Every field hot-reloads on `configuration:updated`, no
restart required. `default_list_limit` / `max_list_limit` swap the shared
snapshot the list/messages calls read. A change to the `adapter` (fs↔bridge, a
new `data_dir`, a bridge url/timeout) rebuilds the store and event plumbing and
swaps it in atomically; the new store's current state is then replayed through
the `session::*` triggers so open subscribers (the console sidebar/transcript,
the harness, ...) stay live without a refetch. A reload that cannot be built
(e.g. an unreadable `data_dir`, or a self-referential bridge `url`) keeps the
previous runtime (last-good) and is surfaced by `session::config-status`. An
invalid bridge `config` is still rejected at parse time, and at boot a
misconfigured bridge is fatal (it never silently falls back to a local fs store).
Switching `data_dir` or bridge `url` changes the backing storage immediately and
does not migrate existing sessions.

**First boot.** When no value is stored yet for id `session-manager`, the worker
registers [`WorkerConfig::default()`](src/config.rs) as `initial_value`.
Optionally pass `--config <path>` to seed from a YAML file instead (one-time,
never overwrites an existing stored value). `${VAR:default}` placeholders in
stored values are expanded by the configuration worker on every read.

## Local development & testing

```bash
cargo run --release -- --url ws://127.0.0.1:49134
cargo test                       # unit + manifest + BDD (engine scenarios self-skip)
cargo test --test bdd -- --tags @pure    # no engine required
cargo test --test bdd -- --tags @engine  # requires a running `iii`
```

The BDD suite under [`tests/features/`](tests/features) is the
behavioural contract: @pure scenarios drive the production handlers
against a real fs store over a tempdir with deterministic ids/clock
(including restart-replay scenarios); @engine scenarios drive the same
code over a live engine, including JSONL persistence readbacks, real
trigger fan-out, and full bridge propagation (two bridged instances +
the main, asserting who received what, exactly once).

## Architecture documentation

Deep documentation lives in [`architecture/`](architecture):
[`architecture/integration.md`](architecture/integration.md) is the
self-contained handoff contract for workers that build on this one
(functions, events, filters, error codes, patterns, topologies);
[`architecture/internals.md`](architecture/internals.md) is the
maintainer deep-dive (module map, invariants, storage formats, event
pipeline, testing architecture).
