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

The call returns a compact receipt such as `{ "surface_id": "deployment-approval", "revision": 3, "component_count": 11, "page": "#/ext/a2ui" }`. The Console keeps that receipt visible in the chat feed and expands it into the full surface on selection, while the A2UI page keeps every surface in the active conversation live through an exact-session state subscription.

Patch an existing surface with another natural-language request. Supplying `expected_revision` prevents an older composition from overwriting a newer edit:

```json
{
  "surface_id": "deployment-approval",
  "instruction": "Add an owner field and make the risk more prominent.",
  "expected_revision": 3
}
```

Interactive surfaces automatically submit their complete bound data model with button actions, so form values are persisted and forwarded to the originating Harness turn. The page also supports bounded revision history and undo, pinning, duplication, JSON import/export, and a per-session template library.

`a2ui::binding::set` can bind an exact `state`, `stream`, or `shell::changed` event to a JSON Pointer in the surface data model. Browser events are deliberately excluded because Console-side registration would bypass Browser approval boundaries. Bindings are declarative and cannot invoke arbitrary functions.

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
