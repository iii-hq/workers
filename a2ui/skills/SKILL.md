---
name: a2ui
description: >-
  Generate safe interactive A2UI surfaces when a Harness response is clearer as
  a dashboard, form, summary, comparison, or decision interface than prose.
---

# a2ui

Use `a2ui` when the result should be an interface the person can scan or act on. Describe the desired experience and supply the underlying data; the worker delegates composition through `llm-router`, validates A2UI v0.9.1, and returns a compact receipt while the Console renders the surface.

Surfaces belong to the current Harness session. The Harness pre-call hook stamps that identity authoritatively, and Console actions return to the same conversation as structured user interactions.

## When to Use

- Present a dashboard, status overview, comparison, form, approval, or decision surface.
- Keep a result live while later function calls update its components or data model.
- Let a person respond through buttons or bound inputs without asking them to author JSON.
- Revise an existing interface through a plain-language patch while preserving unspecified content.
- Export a finished surface as portable A2UI JSON for reuse outside the current conversation.
- Reuse, undo, pin, duplicate, import, or export a surface as a runnable React app or a data-serving iii worker template.
- Materialize the React app into the selected workspace when the person wants to inspect and edit its files in Shell, then preview it through Browser.
- Bind surface data to a worker's event function (optionally with its worker's read-only query), an exact state entry (`state::on-changed`), or Shell change events (`shell::on-changed`) when the UI should stay live.

## Boundaries

- Do not use A2UI for plain answers that are clearer as a short message.
- Generated surfaces use the fixed Console catalog; they cannot execute HTML, JavaScript, CSS, or arbitrary components.
- Use `canvas` for diagrams and freeform drawing, and use a domain worker directly when no visual surface is needed.
- `a2ui::action`, `a2ui::binding::apply`, `a2ui::binding::refresh`, `a2ui::stamp-session`, and `a2ui::on-config-change` are internal Console/Harness lifecycle functions, not agent tools.
- Live bindings observe another worker's private event function (registered with `private: true`), `state::on-changed` or `shell::on-changed`, and never `browser::*`, `harness::*`, `engine::*`, `observability::*`, hook, `http::on-request`, `cron::on-tick`, or `queue::on-message`. A binding passes `args` only to a function that registered an `args_format`; otherwise it selects calls with `filters` on `input.<field>`. A binding `query` must be a function registered by the observed function's own worker with metadata `{"read_only": true}`.
- Do not create new `stream::on-changed` bindings: iii-stream is deprecated and will be removed in an upcoming release. Existing ones keep working while iii-stream runs. Bind the producing worker's own event function.

## Functions

- `a2ui::generate` — compose and persist a complete surface from compact intent and optional data.
- `a2ui::surface::apply` — validate and atomically apply A2UI v0.9.1 messages to one surface.
- `a2ui::surface::get` — read one surface with its component graph and data model.
- `a2ui::surface::list` — list compact surface summaries for the current session.
- `a2ui::surface::delete` — remove a surface from the current session.
- `a2ui::surface::patch` — replace a surface from a natural-language change request with optional revision protection.
- `a2ui::surface::export` — return a session-free, replayable A2UI JSON package.
- `a2ui::surface::history`, `undo`, `duplicate`, and `pin` — manage the surface library and revisions.
- `a2ui::surface::import` and `export-code` — move surfaces between sessions or into source code.
- `a2ui::binding::set` and `delete` — manage safe declarative live-data bindings. Example: `{ "id": "clicks-count", "function_id": "demo::on-counter-changed", "args": { "counter": "clicks" }, "target_path": "/count", "query": { "function_id": "demo::counter::get", "payload": { "counter": "clicks" }, "result_path": "/value" } }`.
- `a2ui::template::*` — save, list, read, apply, and delete reusable session templates.
- `a2ui::action` — internal Console action ingress and optional Harness forwarding.
- `a2ui::stamp-session` — internal `harness::hook::on-pre-call` observer that stamps authoritative turn context.
- `a2ui::on-config-change` — internal authoritative configuration reload handler.
