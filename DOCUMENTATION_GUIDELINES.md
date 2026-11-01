# Documentation guidelines

This guide describes how to author a worker's skill doc so agents know **when**
and **why** to use a worker without duplicating what the engine already exposes
as structured API reference.

Workers may optionally ship a canonical overview file:

```text
<worker>/skills/SKILL.md
```

The file is lean on purpose. Inputs, outputs, JSON schemas, and worked examples
live in the API reference — agents fetch them with `iii list functions` and
`iii get function info` (or the equivalent SDK calls). The skill doc explains
intent, lists what's available, and covers observing the worker's events when it
has any.

## Recommended SKILL.md structure

When present, a worker `SKILL.md` has YAML frontmatter and four body sections
(the last one only when the worker hosts event functions).

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

Bullet list of concrete situations in which an agent should read this skill.
Mirror the tone of other iii skills — specific situations, not generic filler.

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

### `## Observing events` (when applicable)

Include this section when the worker hosts event functions: private functions
it calls when something happens, whose handler returns the event. Skip it
entirely for workers that only expose functions to call.

This section is **not** deferrable to the API reference — agents need to
understand *why* to observe an event function and *how* an observer is set up
(a target function plus `iii.observe`).

Cover:

1. **Why observe** — what reactive behaviour you get vs. polling or imperative
   follow-up calls after a write.
2. **When to observe** — concrete scenarios; when *not* to (e.g. the writer
   already has the new value in its return payload).
3. **How to observe** — the two steps:
   - Register the target with `registerFunction`.
   - Observe the event function with
     `iii.observe({ function_id, args, filters }, target)`.
4. **Args and filters that matter** — the args the function takes (only when
   the worker sets something up per observer: a schedule, a route, a watch) and
   the input fields to filter on, plus TTL interactions or adapter-specific
   behaviour (file watch, bridge forwarding, etc.), in prose or a small table.
   Do not paste full event JSON — point to `get function info` on the event
   function for its `args_format` and event schema.
5. **One minimal observe example** — enough to show the shape, not a full
   end-to-end scenario.

Do **not** duplicate the event payload field-by-field: the event function's
`response_format` carries it in the engine. Say what the event represents and
let the API reference supply the schema.

## What to leave out

| Removed from skill docs | Where it lives instead |
|-------------------------|------------------------|
| Per-function markdown files | — (deleted) |
| `index.md` + `skills/` tree | Single `SKILL.md` |
| `# Inputs`, `# Outputs` | `get function info` → request/response JSON Schema |
| `# Worked example` | Agent composes calls from schemas |
| `# Related` | Function list + agent reasoning |
| `# Side effects`, `# Caching`, `# Rendering rules` | API reference + event function schemas where relevant |

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

- Concrete situation in which to use this worker.
- Another concrete situation.

## Boundaries

- What this worker does not do.
- When to use a sibling worker instead.

## Functions

- `my-worker::foo::register` — one-line purpose.
- `my-worker::foo::get` — one-line purpose.
- `my-worker::foo::list` — one-line purpose.

## Observing events

Observe `my-worker::on-change` when a function should run automatically every
time … — without polling `my-worker::get`.

Reach for it when:

- …
- …

If you only need the new value inside the same function that wrote it, call
`my-worker::set` and use its return value — observe only when a *different*
worker should react.

### How to observe

1. Register the target: `registerFunction('consumer::on-my-worker-change', handler)`.
2. Observe the event function:

```typescript
iii.observe(
  {
    function_id: 'my-worker::on-change',
    filters: { 'input.id': { $eq: 'orders' } }, // see get function info for the event fields
    deliver: 'output',                          // the target receives the event
  },
  'consumer::on-my-worker-change',
)
```

Mutations call `my-worker::on-change`: … Reads do **not**.

For the event shape, call `get function info` on `my-worker::on-change`.
````

## Before you ship

1. Read a peer worker's `SKILL.md` (if one exists) and match its tone.
2. List every function id — cross-check against `iii list functions` for the worker.
3. If the worker hosts event functions, verify the observe example matches each
   function's registered `args_format` and event fields.
4. Confirm nothing duplicates JSON schemas already returned by `get function info`.
