# Documentation guidelines

This guide describes how to author a worker's skill doc so agents know **when**
and **why** to use a worker without duplicating what the engine already exposes
as structured API reference.

Each worker ships one file:

```text
engine/src/workers/<worker>/skills/SKILL.md
```

The file is lean on purpose. Inputs, outputs, JSON schemas, and worked examples
live in the API reference — agents fetch them with `iii list functions` and
`iii get function info` (or the equivalent SDK calls). The skill doc explains
intent, lists what's available, and covers trigger binding when the worker emits
events.

## SKILL.md structure

Every worker `SKILL.md` has YAML frontmatter and four body sections (the last
one only when the worker exposes a trigger type).

### Frontmatter

```yaml
---
name: <worker-name>
description: >-
  One sentence: what problem this worker solves and when an agent should reach
  for it.
---
```

- `name` must match the worker name exactly (same as the directory).
- `description` is what discovery surfaces in bootstraps — keep it self-contained.

### `# <worker-name>` — overview

1–3 paragraphs in prose (no bullet lists in the opening paragraph):

- What problem the worker solves.
- When and why to use it instead of alternatives.
- Any prerequisites (enabled in `config.yaml`, adapter choice, etc.).

This is the only narrative the agent reads before deciding whether the worker
is relevant.

### `## When to Use`

Bullet list of concrete situations that should trigger reading this skill.
Mirror the tone of other iii skills — specific triggers, not generic filler.

### `## Boundaries`

Bullet list of what this worker does **not** do, common misuses, and pointers
to sibling workers when the task belongs elsewhere.

### `## Functions`

A brief catalogue of every `worker::namespace::function` the worker exposes.
One line per function — **what it is for**, not how to call it.

Do **not** include:

- `# Inputs` / `# Outputs` sections
- JSON schema blocks
- Worked examples
- `# Related` cross-links between functions

Those details are in the API reference. If an agent needs the payload shape,
it should call `get function info` for the function id.

Format:

```markdown
## Functions

- `configuration::register` — declare an id with name, description, JSON Schema, and optional initial value.
- `configuration::set` — replace the value for a registered id; validates against the schema.
- `configuration::get` — read one entry by id.
- `configuration::list` — enumerate every registered id.
- `configuration::schema` — read schema/name/description without returning the value.
```

When two functions are tightly paired (`list` + `get`), you may group them in
one bullet if a single line covers both intents — but still name each function
id.

Optional: a one-sentence note on cross-cutting behaviour that isn't obvious
from function names alone (e.g. "reads expand `${VAR:default}` placeholders
against the live process env").

### `## Reactive triggers` (when applicable)

Include this section when the worker registers a custom trigger type. Skip it
entirely for workers that only expose functions.

This section is **not** deferrable to the API reference — agents need to
understand *why* to bind a trigger and *how* the two-step registration works
(handler function + trigger registration).

Cover:

1. **Why bind** — what reactive behaviour you get vs. polling or imperative
   follow-up calls after a write.
2. **When to bind** — concrete scenarios; when *not* to bind (e.g. the writer
   already has the new value in its return payload).
3. **How to bind** — the two-step pattern:
   - Register a handler with `registerFunction`.
   - Register the trigger with `registerTrigger({ type, function_id, config })`.
4. **Config knobs that matter** — filters, conditions, TTL interactions, or
   adapter-specific behaviour (file watch, bridge forwarding, etc.) described
   in prose or a small table. Do not paste full event JSON — point to
   `get function info` on the trigger type or handler for the payload schema.
5. **One minimal binding example** — enough to show the registration shape,
   not a full end-to-end scenario.

Do **not** duplicate the event payload field-by-field if the trigger type is
registered with `call_request_format` / `trigger_request_format` in the engine.
Say what the event represents and let the API reference supply the schema.

## What to leave out

| Removed from skill docs | Where it lives instead |
|-------------------------|------------------------|
| Per-function markdown files | — (deleted) |
| `index.md` + `skills/` tree | Single `SKILL.md` |
| `# Inputs`, `# Outputs` | `get function info` → request/response JSON Schema |
| `# Worked example` | Agent composes calls from schemas |
| `# Related` | Function list + agent reasoning |
| `# Side effects`, `# Caching`, `# Rendering rules` | API reference + trigger/event schemas where relevant |

## Style checklist

- Function ids always in inline backticks: `` `configuration::set` ``.
- Never link to workspace file paths — agents read `SKILL.md` from the worker bundle.
- No emojis. No filler ("This document explains …").
- Open every section with substance.
- Keep the whole file short enough to read in one pass — typically under 120 lines.

## Copy-paste template

````markdown
---
name: my-worker
description: >-
  Short description of the problem this worker solves and when an agent should
  use it.
---

# my-worker

One paragraph overview: what this worker is, the problem it solves, and why
you would reach for it. Mention prerequisites here if any.

Second paragraph optional — adapter notes, persistence model, or how it fits
next to other workers.

## When to Use

- Concrete situation that should trigger using this worker.
- Another concrete situation.

## Boundaries

- What this worker does not do.
- When to use a sibling worker instead.

## Functions

- `my-worker::foo::register` — one-line purpose.
- `my-worker::foo::get` — one-line purpose.
- `my-worker::foo::list` — one-line purpose.

## Reactive triggers

Register a `my-worker` trigger when a function should run automatically every
time … — without polling `my-worker::get`.

Reach for it when:

- …
- …

If you only need the new value inside the same function that wrote it, call
`my-worker::set` and use its return value — register a trigger only when a
*different* worker should react.

### How to bind

1. Register a handler: `registerFunction('stream::on-change', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'my-worker',
  function_id: 'stream::on-change',
  config: {
    // optional filters — see get function info on the trigger type
  },
})
```

Mutations that fire triggers: … Reads do **not** fire triggers.

For the event payload shape, call `get function info` on the trigger type or
handler function id.
````

## Before you ship

1. Read a peer worker's `SKILL.md` (if one exists) and match its tone.
2. List every function id — cross-check against `iii list functions` for the worker.
3. If the worker has a trigger type, verify the binding example matches the
   registered `trigger_request_format`.
4. Confirm nothing duplicates JSON schemas already returned by `get function info`.
