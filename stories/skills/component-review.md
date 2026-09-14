---
name: component-review
description: >-
  Review UI component changes with the stories worker: build the two lines,
  list what changed directly and indirectly, verify every affected state
  with deterministic tree and pixel diffs (varying props from the inferred
  controls), read screenshots only when the verdict asks for it, and write
  a report a human can act on. Use after a branch, a PR or a chat turn
  touched React components.
type: how-to
---

# Component review with `stories`

Two lines go in (older `a`, newer `b`); a report comes out that says which
components changed, how, whether the change is visible, and where. Default
pairs: `a: "main"` / `b: "worktree"` for a branch review, `a: "HEAD"` for
"what did I just change", `a: "prev"` for "what did the last rebuild
change", and `a: "turn:<session>/<turn>/before"` / `b: "turn:<session>/<turn>/after"`
for one chat turn.

## 1. Orient

```json
stories::workspaces {}
```

Pick the workspace (default: the first). Note `branch`, `head`, `dirty`,
and which `lines` are already built. If a component list is empty or stale,
build the working tree first.

## 2. Build both lines

```json
stories::builds::create { "workspace": "<ws>", "line": "worktree", "wait": true }
stories::builds::create { "workspace": "<ws>", "line": "main", "wait": true, "wait_ms": 300000 }
```

A ref line is cached by commit sha, so a second review of the same base is
free. When a call returns `status: "running"`, poll
`stories::builds::get { "build_id" }` or bind `stories:build`. A function
that answers "is still building (build b-…)" is the same situation: wait
and retry.

## 3. Scope the impact

```json
stories::compare { "workspace": "<ws>", "a": "main", "b": "worktree" }
```

Read `summary` first, then `changes`. For each change:

- `kind: "direct"` — the story file or a direct import changed. Review every
  state.
- `kind: "indirect"` — a shared input changed (`files[].hop >= 2`). Review
  the states most likely to exercise it; at least the default state.
- `kind: "new"` — no baseline. Render each state once and check it against
  the intent of the change.
- `kind: "removed"` — confirm the removal was intended.

`files` lists the inputs that differ, closest hop first;
`stories::diff::file { "a", "b", "path" }` gives the unified diff of one of
them when the source matters to the review. `stories::components::list`
with `filter: { change: "any", project, group, tags, path, query }` narrows
the same set when the list is long.

## 4. Verify each affected state

```json
stories::components::get { "workspace": "<ws>", "id": "ui-button", "project": "client-app" }
```

`component.states[]` carries the args and the inferred `controls`
(`select` with `options`, `boolean`, `number`, `text`, `object`;
`function` and `element` are placeholders and cannot be overridden). Build a
small matrix per state: the state as authored, plus one render per option of
each `select` and both values of each `boolean` that the change plausibly
touches. Keep the matrix under about twelve renders per component; a
reviewer can widen it on request.

For every cell:

```json
stories::diff {
  "workspace": "<ws>", "id": "ui-button", "project": "client-app",
  "state": "ui-button--primary", "a": "main", "b": "worktree",
  "args": { "size": "lg" }, "globals": { "theme": "dark" }
}
```

Read the verdict before anything else:

| Verdict | Meaning | Do |
|---|---|---|
| `identical` | no tree change, no pixel change | note and move on |
| `invisible` | tree changed, pixels did not | check `tree[]`: aria, renamed classes, off-screen — usually fine, sometimes a11y regressions |
| `visual` | both changed, explained | read `tree[]` (path, property, before → after) and `pixels.regions[]` (box, ratio, elements); open the PNGs only when the change is not obvious from text |
| `unexplained` | pixels changed, tree did not | fonts, images, anti-aliasing; open `pixels.a_path`, `pixels.b_path`, `pixels.diff_path` and look |

`same_version: true` means the component's inputs are byte-identical in both
lines; skip the rest of its matrix. `stories::tree` returns the rendered DOM
with computed styles, the React tree, the accessibility outline and the
actions log of one render when the diff needs more context;
`stories::screenshot` returns one PNG when a human asked to see a state.

## 5. Report

Write for the person who will merge. Lead with the counts from `summary`,
then one block per component, direct changes first:

```
## Components — main → worktree
3 direct, 2 indirect, 1 new, 0 removed

### UI/Button (client-app) — direct
Files: client-app/src/Button.tsx (hop 1)
- Primary: visual — button > span.label font-weight 500 → 600; region 12×18 px on the label
- Primary (size: lg): visual — same change
- Ghost: identical
- Disabled: invisible — aria-disabled added (expected)
Screenshots: <pixels.a_path>, <pixels.b_path>, <pixels.diff_path> for Primary

### Admin/Status (admin-app) — indirect via packages/ui/src/Badge.tsx (hop 2)
- Ok, Warn: identical
```

Name a finding only when a verdict is `visual` with an unexpected property
or `unexplained`; everything else is a line in the table. Put the report
where the change is reviewed: a kanban ticket comment, a PR review, or the
chat. Paths of the PNGs are absolute on the host; attach them when the
channel can show images.

## 6. Keep watching

To react to later edits without polling, bind the trigger in the
conversation:

```json
{ "trigger_type": "stories:changed", "config": { "workspace": "<ws>", "kinds": ["direct", "indirect"] },
  "label": "stories-changes", "metadata": { "action": "components changed in the working tree" } }
```

Each delivery carries the classified `changes` against the previous build;
run steps 4–5 for the components it names.

## Pitfalls

- Do not override `function` or `element` args; choose another state.
- A component with `error` set failed to evaluate (a singleton, a missing
  mock): report the error text, do not retry.
- Two hosts with different fonts disagree at the pixel level; run both
  lines' renders on the same host, which `stories::diff` always does.
- `worktree` is rebuilt by the watcher after saves; when a rebuild is in
  flight, `stories:build` reports it and calls return "still building".
