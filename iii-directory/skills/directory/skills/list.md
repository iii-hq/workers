---
type: how-to
function_id: directory::skills::list
title: List filesystem-backed skills (id, bytes, modified_at)
---

# When to use

Call `directory::skills::list` when you need a flat, metadata-only
enumeration of every markdown skill the iii-directory worker is
currently serving from its `skills_folder`. One row per file (recursive
`**/*.md`, `prompts/` segments excluded), sorted lex by `id`.

This is the cheap "what's on disk?" call. Use it when:

- You want to verify a `directory::skills::download` actually wrote
  what you expect.
- You're building a picker / autocomplete UI and need just ids + sizes
  rather than bodies.
- You want to discover root-level skill ids (no `/`) to bootstrap a
  system prompt.

For the **rendered** index that humans / LLMs actually read (tree shape
with titles + descriptions), fetch `iii://directory/skills` via
`directory::skills::fetch-skill` instead.

# Inputs

```json
{}
```

No parameters. The worker scans `skills_folder` on every call — there is
no caching, so file edits are visible immediately.

# Outputs

```json
{
  "skills": [
    {
      "id":          "agent-memory/observe",
      "bytes":       1234,
      "modified_at": "2026-05-01T12:34:56+00:00"
    }
  ]
}
```

`bytes` is the on-disk file size (raw, including frontmatter).
`modified_at` is the file's mtime as RFC 3339 (empty if the FS doesn't
expose it).

Rows are sorted lexicographically by `id`. The `id` is the relative path
under `skills_folder` with `.md` stripped (e.g. `agent-memory/observe.md`
→ `agent-memory/observe`). The same id is what
`directory::skills::fetch-skill` accepts as a bare path (auto-prefixed
to `iii://agent-memory/observe`).

# Worked example

After `directory::skills::download {worker: "agent-memory"}` (defaults
to `tag: "latest"`):

```json
{}
```

Returns one entry per markdown file the registry shipped under
`<skills_folder>/agent-memory/...`.

# Related

- `directory::skills::fetch-skill` — read one or more bodies by URI or
  bare skill path.
- `directory::skills::download` — populate `skills_folder` from the
  registry or a GitHub repo.
- `iii://directory/skills` — auto-rendered tree-shaped index with
  titles + descriptions.
