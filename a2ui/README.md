# a2ui

The A2UI worker turns compact agent intent into validated A2UI v0.9.1 surfaces, stores them under the originating Harness conversation, renders them inline in chat and on an injectable Console page, and sends user actions back to the same conversation.

## Install

```bash
iii call compose::add worker=a2ui
```

## Quickstart

Call `a2ui::generate` from a Harness turn. The Harness hook supplies the authoritative session, and the worker reads that turn's routed model, so the agent only sends intent and data:

```json
{
  "description": "A deployment approval card with service, version, risk, and approve or reject actions.",
  "data": {
    "service": "payments-api",
    "version": "2026.08.20",
    "risk": "medium"
  },
  "surface_id": "deployment-approval"
}
```

The call returns a compact receipt such as `{ "surface_id": "deployment-approval", "revision": 3, "component_count": 11, "page": "a2ui" }`. `page` is the console page id: an agent opens it with `console::workspace::open { screen: "ext:a2ui" }`. The Console keeps that receipt visible in the chat feed and expands it into the full surface on selection, while the A2UI page keeps every surface in the active conversation live by observing `state::on-changed` filtered on the session's own entry (`{ "input.scope": { "$eq": "a2ui" }, "input.key": { "$eq": <session_id> } }`).

Patch an existing surface with another natural-language request. Supplying `expected_revision` prevents an older composition from overwriting a newer edit:

```json
{
  "surface_id": "deployment-approval",
  "instruction": "Add an owner field and make the risk more prominent.",
  "expected_revision": 3
}
```

Interactive surfaces automatically submit their complete bound data model with button actions, so form values are persisted and forwarded to the originating Harness turn. The page also supports bounded revision history and undo, pinning, duplication, JSON import/export, and a per-session template library.

### Live bindings

`a2ui::binding::set` binds a JSON Pointer in the surface data model to live data. A binding names a function to observe, with `args` when the function takes them and `filters` on its calls. The Console page observes it for its own view (`deliver: "output"`, so each event is the function's return value) and stores each delivered value, so a binding is active while the surface is open. Bindings are declarative and cannot invoke arbitrary functions. A surface holds at most 32 bindings.

- **A worker's event function** (recommended): `function_id` is a private function another worker hosts (registered with `private: true`, its handler returning the event), for example `orders::on-changed`. The function must be registered when the binding is set and each time it is refreshed. A function that is not private, such as `state::set`, is rejected. `args` are validated against the `args_format` the function registered, and a function without one takes no args: select its calls with `filters` on `input.<field>`. In event mode, each event (or `event_path` inside it) is applied. With a `query`, the event is only a notification. The page reads the query when it opens the surface (after observing) and again after each notification, through the Console-only `a2ui::binding::refresh`. That covers the initial read, recovery after a reconnect and latest-state semantics. The query function must be registered by the same worker as the observed function, with metadata `{"read_only": true}`, and its payload is validated against the function's request schema.
- **`state::on-changed`** takes no args and exactly the filters `{ "input.scope": { "$eq": <scope> }, "input.key": { "$eq": <key> } }`; the page reads the entry with `state::get` on each event. **`shell::on-changed`** takes exactly the args `{ "path": <path> }`.
- Reserved functions are rejected: `engine::*`, `harness::*`, `browser::*` (Console-side observation would bypass Browser approval boundaries), `a2ui::*`, `observability::*` (each applied value logs and traces again), `iii::*`, any id with a `hook` segment, `http::on-request`, `cron::on-tick`, `queue::on-message`, `publish`, `stream::on-join`, `stream::on-leave` and `console::ui::asset`.
- **`stream::on-changed`** (filters on `input.stream_name` and `input.group_id`, optional `input.id`) is deprecated legacy compatibility. It is still accepted and keeps working on engines that run iii-stream. The receipt carries a `deprecation` notice, the worker logs a warning, and the page marks the surface. Move these bindings to the producing worker's own event function; see the guide "Migrate from iii-stream and pubsub".

A binding stored under iii 0.25, with `trigger_type` and `config`, loads as the function that replaced the type: `state` as `state::on-changed` with `scope` and `key` filters, `shell::changed` as `shell::on-changed` with `config` as args, `stream` as `stream::on-changed` with `stream_name`, `group_id` and `item_id` (`input.id`) filters, and a worker's `W::X` as `W::on-X`. For `W::on-X` the worker reads `engine::functions::info` when the session loads: `config` (without `metadata` and `condition_function_id`) becomes the args when the function registered an `args_format`, and one `{ "input.<field>": { "$eq": <value> } }` filter per field when it takes none. The result passes the same checks as `a2ui::binding::set`. When the function is not registered or a check fails, the binding moves to the surface's `retired_bindings` (`{ id, function_id, reason }`) and the page lists it; `a2ui::binding::set` with the same id replaces it and `a2ui::binding::delete` removes it. The surface's next change writes the result back in the current shape.

The runnable provider in `examples/counter_provider/` (`cargo run --example counter_provider -- ws://127.0.0.1:49134`) hosts one event function:

| Function | Args | Delivery | Event |
|---|---|---|---|
| `demo::on-counter-changed` | `{ counter }`, 1-64 bytes (`INVALID_ARGS` otherwise) | observers whose `args.counter` is the event's `counter`; at most 64 observers: an observer added while 64 are active is rejected with `OBSERVER_LIMIT` | `{ counter, revision }`, called after each committed `demo::counter::bump` |

An observer of it, from any worker:

```typescript
const clicks = iii.observe(
  { function_id: 'demo::on-counter-changed', args: { counter: 'clicks' }, deliver: 'output' },
  'my-app::on-clicks', // receives { counter, revision }
)
await clicks.ready() // rejected with OBSERVER_LIMIT past 64 observers
```

A binding to it:

```json
{
  "surface_id": "clicks",
  "binding": {
    "id": "clicks-count",
    "function_id": "demo::on-counter-changed",
    "args": { "counter": "clicks" },
    "target_path": "/count",
    "query": {
      "function_id": "demo::counter::get",
      "payload": { "counter": "clicks" },
      "result_path": "/value"
    }
  }
}
```

The A2UI page does not replace or embed itself in Shell or Browser. Its workspace action materializes a complete runnable React project under `generated/a2ui/<surface>-r<revision>` in the active Harness working directory. Shell then shows the real source files and Git diffs for editing. Run the generated Vite app in Shell and open its local URL in the Browser worker for preview. The same runnable project is available as a React app ZIP through `a2ui::surface::export-code`; JSON exports remain portable through `a2ui::surface::import`.

## Session stamping

Every session-scoped A2UI function takes its session from the Harness, not from the caller. The worker observes `harness::hook::on-pre-call` with `a2ui::stamp-session` and args `{ functions: [<the A2UI functions>], timeout_ms: 5000, on_error: "fail_closed" }`; before one of those functions runs, the hook writes the turn's `session_id` into its arguments, and a hook failure denies the call.

## Configuration

The `configuration` worker stores this worker's live settings, and the worker observes `configuration::on-updated` for its entry to reload them. An optional `--config` YAML file seeds them on first boot:

```yaml
composer_model: null          # inherit the Harness turn's model
composer_provider: null       # inherit its routed provider
max_output_tokens: 8192       # one composition or repair call
max_composer_input_bytes: 786432
repair_attempts: 1            # bounded validation correction
max_surfaces_per_session: 16
max_history_per_surface: 64
max_templates_per_session: 32
max_components_per_surface: 160
max_description_bytes: 32768
max_data_bytes: 524288
max_surface_bytes: 2097152     # current surface plus bounded history
max_session_bytes: 16777216    # surfaces, histories, and templates
forward_actions: true         # send Console actions to harness::send
```

The worker implements the stable A2UI v0.9.1 envelope with the safe `urn:iii:a2ui:console:v0.1` catalog. The catalog maps declarative components onto the running Console's shared React components and design tokens; it never executes model-provided HTML, JavaScript, or CSS.
