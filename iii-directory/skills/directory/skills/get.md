---
type: how-to
function_id: directory::skills::get
title: Read one skill body by id
---

# When to use

Call `directory::skills::get` whenever you need the **body** of one
skill — the markdown a worker publishes to teach the agent when and
why to use its functions. It returns the body alongside the same
`title`, `description`, and `modified_at` fields each
`directory::skills::list` row already carries, so the API mirrors
`directory::prompts::get` exactly.

Reach for it when:

- You hit an `iii://...` link inside another skill and need its
  contents inlined.
- You're building a picker UI that resolved an id from
  `directory::skills::list` and the user selected one row.
- You want a deeper sub-skill (`iii://resend/email/send`) that wasn't
  inlined into the system-prompt bootstrap (which loads root skills
  only).

There is no batching. Call once per id; consumers that need several
bodies issue one `get` per id.

# Inputs

```json
{ "id": "agent-memory/observe" }
```

`id` is required. It must be the same string `directory::skills::list`
returned (a path under `skills_folder` with `.md` stripped). Each
segment must satisfy `[a-z0-9_-]{1,64}` and the depth is unbounded.

For ergonomics the legacy `iii://{id}` link form is also accepted —
the prefix is stripped before validation:

```json
{ "id": "iii://agent-memory/observe" }
```

Any other URI scheme (`https://`, `ftp://`, ...) is rejected.

# Outputs

```json
{
  "id":          "agent-memory/observe",
  "title":       "How to observe",
  "description": "Record an event in agent memory.",
  "body":        "# How to observe\n\n...",
  "modified_at": "2026-05-01T12:34:56+00:00"
}
```

- `id` echoes the resolved id (the same string accepted as input,
  with any `iii://` prefix stripped).
- `title` is the first `# H1` line in the body, falling back to `id`
  when the file has no H1.
- `description` is the first non-heading paragraph, empty when the
  file has only headings.
- `body` is the raw markdown post-frontmatter from disk.
- `modified_at` is the file mtime as RFC 3339 (empty if the FS
  doesn't expose it).

The shape is intentionally identical to
`directory::prompts::get` (with `id` standing in for that surface's
`name`) so a single client struct can target either reader.

# Worked example

The agent loaded a worker skill that links to a deeper sub-skill at
`iii://resend/email/send`. To inline the linked body:

```json
{ "id": "resend/email/send" }
```

Same response either way:

```json
{ "id": "resend/email/send", "title": "...", "description": "...", "body": "...", "modified_at": "..." }
```

# Related

- `directory::skills::list` — discover the ids that resolve via
  `directory::skills::get` (already carries `title` + `description`,
  so a picker UI doesn't need a `get` per row).
- `directory::skills::download` — populate `skills_folder` so there's
  something to fetch.
- `directory::engine::functions::info` — for the **structured** view
  of one function (schemas + how_guide + related_skills) instead of a
  raw skill body.
