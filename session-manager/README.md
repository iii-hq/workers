# session-manager

Durable, reactive store for conversations. A session is an append-only
log of typed message entries (user, assistant, function_result, custom
— with optional fork branches) plus a small metadata record (title,
description, coarse status, app-defined metadata). Any worker or client
appends and reads over the bus; every mutation calls one of six event
functions that consumers observe, so a chat UI, bot, or dashboard renders
live with no polling and no separate publish call. It runs no agent logic —
it is the conversation database the [harness](https://github.com/iii-hq/workers/tree/main/harness) family drives, and
it is independently useful as a real-time chat store for any app.

## Install

```bash
iii call compose::add worker=session-manager
```

`iii call compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

## Quickstart

Create a session, append a message, read the transcript back:

```rust
use iii_sdk::{register_worker, CallRequest, InitOptions};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://localhost:49134", InitOptions::default());

    let created = iii
        .call(CallRequest::new(
            "session::create",
            json!({ "title": "Weather question", "metadata": { "owner": "u_1" } }),
        ))
        .await?;
    let session_id = created["session_id"].as_str().unwrap().to_string();

    iii.call(CallRequest::new(
        "session::append",
        json!({
            "session_id": session_id,
            "message": {
                "role": "user",
                "content": [{ "type": "text", "text": "What's the weather?" }],
                "timestamp": 1717800000000_i64
            }
        }),
    ))
    .await?;

    let transcript = iii
        .call(CallRequest::new(
            "session::messages",
            json!({ "session_id": session_id }),
        ))
        .await?;

    println!("{transcript:#?}");
    Ok(())
}
```

To render live instead of polling, register a handler function and observe
an event function with it:

```rust
use iii_sdk::{Deliver, Error, RegisterFunction, Source};
use serde_json::{json, Value};

iii.register_function(
    "my-ui::on-message-updated",
    RegisterFunction::new_async(|event: Value| async move {
        // full updated message + monotonic revision; keep the highest
        println!("{} rev {}", event["entry_id"], event["revision"]);
        Ok::<_, Error>(json!({ "ok": true }))
    }),
);

// one session's assistant messages, as they stream
iii.observe(
    Source::function("session::on-message-updated")
        .filters(json!({
            "input.session_id": { "$eq": session_id },
            "input.message.role": { "$in": ["assistant"] },
        }))
        .deliver(Deliver::Output),
    "my-ui::on-message-updated",
);
```

Streaming an assistant reply uses the same primitives: append an
(initially empty) assistant message, then call `session::update-message`
as tokens arrive — each update calls `session::on-message-updated` with an
incremented `revision`.

The full function surface (21 `session::*` functions: lifecycle,
messages, branching/fork, attachments) is documented in the spec and on each
function's registered schema — see `iii worker info session-manager`.
`session::messages` items carry the entry `origin` (hook annotations —
e.g. which memory bank fed a generate), so consumers can render
provenance without a second lookup.

Sessions can be mentioned in a chat as `@session(id="<session_id>")`
(see [`crates/mention-contract`](../crates/mention-contract/README.md)).
Two internal functions provide it: `session::mention::search` ranks
sessions by title words or id (top-level chats before sub-agents, e2e
sessions and the asking session left out) and `session::mention::get`
returns the pill, preview card and agent summary — never the parked
draft — and declares the provider, whose details call is `session::get`.

## Event functions

The worker hosts one function per kind of mutation. Each is private (only
this worker calls it), and its handler returns the event it is called with,
so an observer receives the event. The worker calls a function only while
something observes it. None takes args (an observer that passes args gets
`INVALID_ARGS` from the engine); observers select events with filters on the
event's fields. iii delivers each call to every observer whose filters match.

Every event carries `session_metadata`: the session's `SessionMeta.metadata`
(the tenancy hook, e.g. `{ "owner": "u_1" }`) as of the mutation, omitted
when the session has none. `session::on-meta-updated` carries the metadata
after the update, `session::on-deleted` the metadata the session had before
deletion.

| Function | Args | Called when | Event (what observers receive) |
|---|---|---|---|
| `session::on-created` | none; filter on `input.session_id`, `input.session_metadata.<key>` | A session is created (create / ensure / fork) | `{ session_id, title, description, status, forked_from?, created_at, session_metadata? }` |
| `session::on-message-added` | none; filter on `input.session_id`, `input.message.role`, `input.session_metadata.<key>` | An entry is appended | `{ session_id, entry_id, parent_id, message?, custom?, origin?, timestamp, session_metadata? }` |
| `session::on-message-updated` | none; filter as `session::on-message-added` | A message's content changes (streaming deltas) | `{ session_id, entry_id, message, revision, origin?, timestamp, session_metadata? }` |
| `session::on-status-changed` | none; filter on `input.session_id`, `input.session_metadata.<key>` | Status moves between idle/working/done/error, or its reason changes | `{ session_id, status, previous_status, status_reason?, timestamp, session_metadata? }` |
| `session::on-meta-updated` | none; filter as `session::on-status-changed` | Title/description/metadata change | `{ session_id, title, description, metadata?, timestamp, session_metadata? }` |
| `session::on-deleted` | none; filter as `session::on-status-changed` | A session and its entries are removed | `{ session_id, timestamp, session_metadata? }` |
| `session::store::on-events` | none | Internal, fs-mode instances only: once per event, for bridged instances (see Storage adapters) | `{ function_id, payload, session_metadata? }`: `function_id` names the event function, `payload` is its event without `session_metadata` |

A custom entry (`kind: "custom"`) carries `custom` and no `message`, so a
filter on `input.message.role` never matches it; a custom *message* has
role `custom`.

```rust
// one session's transcript, every role
iii.observe(
    Source::function("session::on-message-added")
        .filters(json!({ "input.session_id": { "$eq": session_id } }))
        .deliver(Deliver::Output),
    "my-ui::on-message-added",
);

// one tenant's sessions appearing and going away
iii.observe(
    Source::function("session::on-created")
        .filters(json!({ "input.session_metadata.owner": { "$eq": "u_1" } }))
        .deliver(Deliver::Output),
    "my-ui::on-session-created",
);
iii.observe(
    Source::function("session::on-deleted")
        .filters(json!({ "input.session_metadata.owner": { "$eq": "u_1" } }))
        .deliver(Deliver::Output),
    "my-ui::on-session-deleted",
);

// every status change (a spinner per session)
iii.observe(
    Source::function("session::on-status-changed").deliver(Deliver::Output),
    "my-ui::on-status-changed",
);
```

Calls are fire-and-forget and unordered, and an event can be lost (the
mutation is durable either way). A reload replays the current state, so the
same entry can arrive again. Reconcile message updates by `revision` (keep
the highest) and transcript order by the parent chain, never by arrival
order; recover with a read-back (`session::messages`, `session::get`).

### What the worker observes

- `configuration::on-updated`, filtered to the `configuration:updated`
  events of its own entry (`input.id`, `input.event_type`), with
  `session::on-config-change` as the target, which re-reads the entry and
  applies it.
- On a bridged instance, the main instance's `session::store::on-events`,
  with a relay function it registers on the main's engine
  (`session::bridge::recv::<uuid>`) as the target.

## Storage adapters

Two adapters, selected by an `adapter` block (a `name` plus a nested
`config`):

- **`fs`** (default) — one append-only JSONL file per session under
  `data_dir` (`<encoded_session_id>.jsonl`): typed `meta` / `entry` /
  `leaf` records, replayed last-wins on startup, file removed on
  session delete. This is the durable, single-instance setup. An append
  is one `append` record (entry, leaf and metadata together) with a
  `commit` marker per committed entry; a build from before these record
  types skips them as malformed lines, so **do not roll back to an older
  session-manager once this version has written to a `data_dir`**: the
  entries appended since would not replay, and older metadata would
  return without an error.
- **`bridge`** — this instance keeps all domain logic (idempotency,
  revisions, branching, locks) but stores through a **main** instance
  running its own session-manager (`adapter name: fs`) on another iii
  engine, via the internal `session::store::*` protocol.

Event propagation in a bridge topology: the main is the single fan-out
point. A bridged instance publishes each mutation's events to the main
(`session::store::publish-events`); the main calls its own event functions
and, once per event, its internal `session::store::on-events` with an
envelope, which every attached bridged instance observes; each bridge calls
its local event functions with the envelope's event, and iii applies its
local observers' filters. With several bridged instances attached to one
main, a mutation made anywhere calls every instance's event functions once.
(Consequence: an originator's own observers receive events after the round
trip through the main — the same fire-and-forget, unordered delivery as on
any instance.)

The `session::store::*` functions are deployment plumbing served only
by fs-mode instances — deny them to agents like every other mutating
surface.

## Configuration

Runtime settings live in the **`configuration` worker** under id
**`session-manager`**. At startup the worker registers its JSON Schema,
fetches the live, env-expanded value via `configuration::get`, and observes
`configuration::on-updated` (see What the worker observes) so changes apply
without a restart. Persisted values
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
    data_dir: data/session-manager           # relative to III_COMPOSE_DIR

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
the `session::on-*` event functions so open observers (the console
sidebar/transcript, the harness, ...) stay live without a refetch. A reload that cannot be built
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
III_ENGINE_WS_URL=ws://127.0.0.1:<port> cargo test --test bdd -- --tags @engine
                                 # requires a running `iii` with no session-manager
```

The @engine scenarios register their own session-manager surface on the engine
they reach (`III_ENGINE_WS_URL`, default `ws://127.0.0.1:49134`), so they skip
an engine that already runs session-manager, or whose workers or functions
cannot be listed, rather than write into its store: point them at a bare engine
on a free port. The check is repeated before every @engine scenario (a
session-manager that takes the ids after an engine restart skips the rest of the
run); a takeover in the middle of a scenario can still reach the rest of that
scenario.

The BDD suite under [`tests/features/`](tests/features) is the
behavioural contract: @pure scenarios drive the production handlers
against a real fs store over a tempdir with deterministic ids/clock
(including restart-replay scenarios); @engine scenarios drive the same
code over a live engine, including JSONL persistence readbacks, real
observers with filters, and full bridge propagation (two bridged instances
+ the main, asserting who received what, exactly once).

## Architecture documentation

Deep documentation lives in [`architecture/`](architecture):
[`architecture/integration.md`](architecture/integration.md) is the
self-contained handoff contract for workers that build on this one
(functions, events, filters, error codes, patterns, topologies);
[`architecture/internals.md`](architecture/internals.md) is the
maintainer deep-dive (module map, invariants, storage formats, event
pipeline, testing architecture).
