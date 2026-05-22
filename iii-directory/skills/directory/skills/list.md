---
type: how-to
function_id: directory::skills::list
title: Enumerate every skill on disk with title and description
---

> **Function id:** `directory::skills::list` — pass this to `agent_trigger { function: "directory::skills::list" }` (NOT the skill path you saw in `directory::skills::list`; that's a documentation id, not a callable function id).

# When to use

Call `directory::skills::list` when you need an enumeration of every
markdown skill the iii-directory worker is currently serving from its
`skills_folder`. One row per file (recursive `**/*.md`, `prompts/`
segments excluded), sorted lex by `id`. Each row already carries
`title` (frontmatter `title:` when present, else the body H1),
`type` (frontmatter `type:` — e.g. `index`, `how-to`, `reference`),
and `description` (first paragraph), so a picker / table-of-contents
UI doesn't need a follow-up `directory::skills::get` per row.

This is the single "what's on disk?" call. Use it when:

- You want to verify a `directory::skills::download` actually wrote
  what you expect.
- You're building a picker / autocomplete UI and need a flat list of
  ids + labels rather than bodies.
- You want to discover root-level skill ids (no `/`) to bootstrap a
  system prompt.
- You want to render an indented tree client-side (depth =
  `id.matches('/').count()`).

# Inputs

```json
{}
```

No parameters. The worker scans `skills_folder` on every call and
reads each body to populate `title` + `description` — file edits are
visible immediately, no caching.

# Outputs

```json
{
  "skills": [
    {
      "id":          "agent-memory/observe",
      "title":       "How to observe",
      "type":        "how-to",
      "description": "Record an event in agent memory.",
      "bytes":       1234,
      "modified_at": "2026-05-01T12:34:56+00:00"
    }
  ]
}
```

- `id` is the relative path under `skills_folder` with `.md` stripped
  (e.g. `agent-memory/observe.md` → `agent-memory/observe`). Same
  string `directory::skills::get` accepts.
- `title` resolves in this order: YAML frontmatter `title:` (when
  present and non-empty after trim), then the first `# H1` line in
  the body, with the bare `id` as a final fallback.
- `type` is the YAML frontmatter `type:` field (free-form classifier;
  common values are `index`, `how-to`, `reference`). `null` when the
  file has no frontmatter or omits the key.
- `description` is the first non-heading paragraph, empty when the
  file has only headings.
- `bytes` is the on-disk file size (raw, including frontmatter).
- `modified_at` is the file's mtime as RFC 3339 (empty if the FS
  doesn't expose it).

Rows are sorted lexicographically by `id`.

# Worked example

After `directory::skills::download {worker: "agent-memory"}` (defaults
to `tag: "latest"`):

```json
{}
```

Returns one entry per markdown file the registry shipped under
`<skills_folder>/agent-memory/...`, each with title + description
already populated.

To render a tree-shaped picker, walk the rows in order and indent each
by `2 * id.matches('/').count()` spaces — the lex-sort already places
each child immediately after its parent.

# Related

- `directory::skills::get` — read one body by id (returns the same
  `id` / `title` / `type` / `description` / `modified_at` plus `body`).
- `directory::skills::download` — populate `skills_folder` from the
  registry or a GitHub repo.
