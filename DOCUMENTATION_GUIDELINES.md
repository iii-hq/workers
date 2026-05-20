# Documentation guidelines

This guide describes how to author a worker's skill bundle so the docs stay
consistent across workers. When in doubt, mirror the structure of an existing
bundle in your project alongside this guide.

A skill bundle is a folder of markdown files that explains, from the agent's
point of view, **when** and **why** to call each function a worker exposes.
The structure has three load-bearing parts:

1. A predictable folder layout (so an agent can resolve `iii://...` links).
2. YAML frontmatter on every file (so the directory reader can extract title,
   type, description without parsing the body).
3. A fixed section order in each how-to (so an agent always finds inputs,
   outputs, and related calls in the same place).

## 1. Folder layout

One top-level folder per worker, named after the worker. Inside it:

```text
skills/
  <worker-name>/
    index.md                  <- worker overview, type: index
    skills/
      <ns>/
        <sub-ns>/
          <function>.md       <- one how-to per function
        <namespace>.md        <- OR one file for a whole namespace (see 4.)
```

Rules:

- The path under `skills/<worker>/skills/` mirrors the function namespace.
  `<worker>::<ns>::<sub-ns>::<fn>` lives at
  `skills/<worker>/skills/<ns>/<sub-ns>/<fn>.md` — each `::` becomes a `/`.
- File and folder segments must match `[a-z0-9_-]{1,64}`. No spaces, no
  uppercase, no dots other than the trailing `.md`.
- Closely paired siblings (typically `list` + `get`, or `list` + `info`)
  MAY collapse into a single file named after the namespace, e.g. one
  `skills/<worker>/skills/<ns>/<sub-ns>.md` can cover both
  `<worker>::<ns>::<sub-ns>::list` and `<worker>::<ns>::<sub-ns>::get`.
  See section 5 for the layout that file uses.

## 2. `index.md` — worker overview

Every worker folder has exactly one `index.md` at its root. This is the file
the directory reader's index function extracts into the per-worker bootstrap
snippet.

Frontmatter:

```yaml
---
type: index
title: <worker-name>
---
```

Body shape:

- `# <worker-name>` H1.
- 1-3 paragraph overview. The first paragraph is what the directory reader's
  index function will surface in agent bootstraps, so keep it self-contained
  and prose-only (no bullet lists in the first paragraph).
- A bulleted list naming each sub-namespace (e.g. `my-worker::foo::*`,
  `my-worker::bar::*`) with one sentence of context.
- `## How-tos` heading.
- One `### <namespace>::*` subsection per sub-namespace, each containing a
  bullet list of `[function::id](iii://<worker>/<path>)` links with a
  one-line rationale per function.

Copy-paste template:

````markdown
---
type: index
title: my-worker
---

# my-worker

One-paragraph overview that explains what this worker does, ending with
links to its sub-namespaces:

- **Foo** (`my-worker::foo::*`) — short description.
- **Bar** (`my-worker::bar::*`) — short description.

## How-tos

### `my-worker::foo::*`

- [`my-worker::foo::list`](iii://my-worker/foo/list) — one-line rationale.
- [`my-worker::foo::get`](iii://my-worker/foo/get) — one-line rationale.

### `my-worker::bar::*`

- [`my-worker::bar::run`](iii://my-worker/bar/run) — one-line rationale.
````

## 3. Per-function how-to

The default shape. One file per function.

### Frontmatter

```yaml
---
type: how-to
function_id: <worker::namespace::function>
title: <imperative-mood title describing what the function does>
---
```

- `type: how-to` is what the directory reader's `functions::info` call looks
  for when it picks the `how_guide` field.
- `function_id` must be the exact dotted id. The directory reader uses this
  (or a body-level `iii://fn/<dotted/path>` link) to associate the file with
  a function.
- `title` should describe the action, not the function name. Good: `"List
  trigger types registered with the engine"`. Bad: `"trigger-list"`.

### Required sections (in this exact order)

1. `# When to use`
2. `# Inputs`
3. `# Outputs`
4. `# Worked example`
5. `# Related`

Optional sections (see section 4) slot in between `# Outputs` and
`# Worked example`.

#### `# When to use`

Opening paragraph in narrative voice describing the scenario that motivates
the call. Followed (optionally) by a `Reach for it when:` or `Common
situations:` bullet list of 2-4 concrete triggers. When a sibling function
overlaps, add a short "Use `<sibling>` instead when ..." pointer so the
agent doesn't pick the wrong one. When two functions answer different
questions on the same namespace, use a decision table:

```markdown
| Question                              | Use this                           |
|---------------------------------------|------------------------------------|
| What is wired into my local instance? | `my-worker::local::items::info`    |
| What is published in the registry?    | `my-worker::registry::items::info` |
```

Use this shape when two surfaces (e.g. local vs. registry) answer different
questions on the same namespace.

#### `# Inputs`

One JSON code block showing the input shape. Use inline `// ...` comments
to document each field — flag required/optional and capture semantics that
aren't obvious from the field name. Follow the block with a short paragraph
stating which fields are required and listing any cross-field constraints
(`mutually exclusive`, `default when omitted`, ...).

#### `# Outputs`

One JSON code block showing the response shape, again with inline
`// ...` comments per field. Follow it with a bullet list that documents:

- Null/empty handling (`null when ...`, `omitted when ...`).
- Sort order (rows are always lex-sorted by `<field>`).
- Resolution precedence (e.g. `title resolves in this order: frontmatter
  title, then body H1, then bare id`).
- Truncation / cap rules.

#### `# Worked example`

One or more concrete sample calls, each preceded by a one-line prose
framing. Show the request payload as a JSON block; only show the response
when it adds information beyond what `# Outputs` already documented.

#### `# Related`

Bullet list of sibling functions a caller is likely to need next. Format
each bullet as `` `function::id` `` followed by an en-dash and a short
rationale ending with a period:

```markdown
- `my-worker::foo::list` — find the id you want to inspect.
- `my-worker::bar::info` — group by a different dimension.
```

### Copy-paste template

````markdown
---
type: how-to
function_id: my-worker::foo::get
title: Read one foo by id
---

# When to use

Call `my-worker::foo::get` when you already have a foo id (from
`my-worker::foo::list` or elsewhere) and you need the full body.

Reach for it when:

- You hit an `iii://...` link inside another skill and want it inlined.
- A picker UI surfaced an id that the user selected.

Use [`my-worker::foo::list`](iii://my-worker/foo/list) instead when you
need to enumerate without already knowing an id.

# Inputs

```json
{ "id": "foo-123" }
```

`id` is required and must match `[a-z0-9_-]{1,64}`.

# Outputs

```json
{
  "id":          "foo-123",
  "title":       "Example foo",
  "body":        "# Example foo\n\n...",
  "modified_at": "2026-05-01T12:34:56+00:00"
}
```

- `title` resolves in this order: frontmatter `title`, then first body H1,
  then bare `id` as a final fallback.
- `modified_at` is the file mtime as RFC 3339 (empty when the FS does not
  expose it).

# Worked example

Inline a linked foo:

```json
{ "id": "foo-123" }
```

# Related

- `my-worker::foo::list` — discover the id first.
- `my-worker::foo::download` — populate the store so there's something to
  read.
````

## 4. Optional sections

Insert these between `# Outputs` and `# Worked example` when they apply.
Each has a fixed contract.

### `# Side effects`

**Required** for any function that writes to disk, fires a trigger, or
mutates external state. Document each event/file write with its payload
shape. Subscribers read this to know what to forward.

Use this in any how-to whose function writes to disk or emits an event.

### `# Caching`

**Required** for any function backed by a network or in-process cache.
State the cache key (which inputs partition the cache), the TTL (with the
config knob that controls it), and how to bust the cache.

Use this in any how-to whose function reads through a network or in-process
cache.

### `# Rendering rules`

**Required** when the function returns a rendered markdown document (a
`body` field whose contents the agent is expected to paste verbatim into a
prompt or message). Spell out:

- Which inputs flow into which output sections.
- Which inputs are filtered out (e.g. "only skills with `type: index`
  appear in the body").
- The exact heading levels and template snippets used.

Use this in any how-to whose function returns a `body` field meant to be
pasted verbatim into a prompt or message.

## 5. Multi-function variant

When two or more siblings on the same namespace are tightly coupled
(`list` + `get`, or `list` + `info` + `download`), one file MAY cover the
whole namespace. Use this when the functions share enough context that
splitting them creates duplication.

Frontmatter switches `function_id` for a `functions:` array:

```yaml
---
type: how-to
functions: [my-worker::foo::list, my-worker::foo::get]
title: List and read foos
---
```

Body shape:

- Single `# When to use` covering the whole namespace, ideally with a
  `| Question | Use this |` decision table mapping intents to function ids.
- One `# <function::id>` H1 per function, each containing `## Inputs` and
  `## Outputs` H2 subsections (NOT H1 — only the function names use H1).
- A single `# Worked example` at the end covering the realistic
  list-then-get flow.
- A single `# Related` at the end.
- Any applicable optional sections (`# Side effects`, `# Caching`,
  `# Rendering rules`) come after `# Worked example` and before
  `# Related`, same as the single-function variant.

## 6. Style checklist

- Function ids always go in inline backticks: `` `my-worker::foo::get` ``.
  Never bare, never bolded.
- Cross-skill links use the `iii://<worker>/<path>` URI scheme:
  `[my-worker::foo::get](iii://my-worker/foo/get)`. Don't link to a
  workspace path.
- Every JSON example is **valid JSON shape** (real keys, plausible
  values), not pseudocode like `{ ... }`. Use `"..."` for placeholder
  strings.
- Annotate JSON fields with inline `// ...` comments. Column-align the
  comments when the block has many fields.
- `# Related` bullets are en-dash separated, end with a period:
  `` - `fn::id` — short rationale. ``
- When two surfaces share a shape (engine vs registry, list vs info),
  name the shared core fields explicitly in both files and link to the
  counterpart from `# Related`. This lets a caller write one parser
  against both surfaces.
- Don't repeat the function id as the file title. The `title:` field is
  human-readable, action-oriented prose.
- Don't add emojis. Don't add filler ("This document explains ...",
  "In this section we will ..."). Open every section with the substance.

## 7. Mirror an existing peer

Before writing a new how-to, find the closest match among the existing
bundles in your project and follow its structure:

- Writing a worker `index.md`? Read another worker's overview.
- Writing a standard single-function how-to? Read any file with the five
  required sections and no optional ones.
- Writing a how-to with a decision table? Read a how-to whose
  `# When to use` opens with a `| Question | Use this |` table.
- Writing a write-path function? Read a how-to with a `# Side effects`
  section.
- Writing a network-backed function? Read a how-to with a `# Caching`
  section.
- Writing a function that returns rendered markdown? Read a how-to with a
  `# Rendering rules` section.
- Writing one file for multiple sibling functions? Read a multi-function
  file (its frontmatter has a `functions:` array instead of `function_id:`).
