# a2ui

The A2UI worker turns compact agent intent into validated A2UI v0.9.1 surfaces, stores them under the originating Harness conversation, renders them inline in chat and on an injectable Console page, and sends user actions back to the same conversation.

## Install

```bash
iii trigger compose::add worker=a2ui
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

The call returns a compact receipt such as `{ "surface_id": "deployment-approval", "revision": 3, "component_count": 11, "page": "a2ui" }`. `page` is the console page id: an agent opens it with `console::workspace::open { screen: "ext:a2ui" }`. The Console keeps that receipt visible in the chat feed and expands it into the full surface on selection, while the A2UI page keeps every surface in the active conversation live through an exact-session state subscription.

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

`a2ui::binding::set` binds a JSON Pointer in the surface data model to live data. The Console page registers the binding for its own view and stores each delivered value, so a binding is active while the surface is open. Bindings are declarative and cannot invoke arbitrary functions. A surface holds at most 32 bindings.

- **Worker-owned trigger types** (recommended): `trigger_type` is any trigger type a worker registers, for example `orders::changed`. The type must be registered when the binding is set, and `config` is validated against the configuration schema the provider registered (with `trigger_request_format`). In event mode, each event payload (or `event_path` inside it) is applied. With a `query`, the event is only a notification. The page reads the provider's query when it opens the surface (after binding) and again after each notification, through the Console-only `a2ui::binding::refresh`. That covers the initial read, recovery after a reconnect and latest-state semantics. The query function must be registered by the same worker that owns the trigger type, with metadata `{"read_only": true}`, and its payload is validated against the function's request schema.
- **`state`** (`scope`, `key`) and **`shell::changed`** (`path`) keep their exact configs.
- Reserved types are rejected: `engine::*`, `harness::*`, `browser::*` (Console-side registration would bypass Browser approval boundaries), `a2ui::*`, `iii::*`, any `hook` type, `http`, `cron`, `queue`, `durable:subscriber`, `subscribe`, `stream:join` and `stream:leave`.
- **`stream`** (`stream_name`, `group_id`, optional `item_id`) is deprecated legacy compatibility. It is still accepted and keeps working on engines that run iii-stream. The receipt carries a `deprecation` notice, the worker logs a warning, and the page marks the surface. Move these bindings to a worker-owned trigger type; see the guide "Migrate from iii-stream and pubsub".

Example binding to a worker-owned trigger type served by the runnable provider in `examples/owned_trigger_provider.rs` (`cargo run --example owned_trigger_provider -- ws://127.0.0.1:49134`):

```json
{
  "surface_id": "clicks",
  "binding": {
    "id": "clicks-count",
    "trigger_type": "demo::counter-changed",
    "config": { "counter": "clicks" },
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

## Configuration

The `configuration` worker stores this worker's live settings. An optional `--config` YAML file seeds them on first boot:

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
