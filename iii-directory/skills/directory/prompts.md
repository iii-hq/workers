---
type: how-to
functions: [directory::prompts::list, directory::prompts::get]
title: List and read filesystem-backed prompts
---

# When to use

Use `directory::prompts::*` to surface the static, parametric prompt
templates a worker ships alongside its code. Prompts are the
slash-command counterpart to the function surface — the *user*
invokes them (`/send-email`, `/triage`), and the agent renders them
into a real call.

| Question                                                  | Use this                       |
|-----------------------------------------------------------|--------------------------------|
| What prompt templates are available right now?            | `directory::prompts::list`     |
| What does this one prompt actually contain?               | `directory::prompts::get`      |

Prompts are sourced from the same `skills_folder` as skills. Files at
`<skills_folder>/<ns>/prompts/*.md` with YAML frontmatter declaring
at least `description` are exposed; everything else is treated as a
skill body. Re-reads happen on every call — file edits are visible
immediately, no caching.

The two responses are plain JSON shapes — no MCP envelope, no
role/messages wrapper — so this worker stays agnostic to MCP and any
other adapter. Adapters can shape the response on their own side.

# `directory::prompts::list`

## Inputs

```json
{}
```

No parameters.

## Outputs

```json
{
  "prompts": [
    {
      "name":        "send-email",
      "description": "Compose and send a transactional email.",
      "modified_at": "2026-05-01T12:34:56+00:00"
    }
  ]
}
```

- `name` is the prompt's frontmatter `name`, falling back to the
  file stem (e.g. `send-email.md` → `send-email`). Each name must
  satisfy `[a-z0-9_-]{1,64}`.
- `description` is the frontmatter `description` (required at scan
  time — files without it are silently skipped).
- `modified_at` is the file's mtime as RFC 3339.

Rows are sorted lexicographically by `name`.

# `directory::prompts::get`

## Inputs

```json
{ "name": "send-email" }
```

`name` is required and must match the same `[a-z0-9_-]{1,64}` shape
returned by `directory::prompts::list`.

## Outputs

```json
{
  "name":        "send-email",
  "description": "Compose and send a transactional email.",
  "body":        "# /send-email\n\nCompose an email…",
  "modified_at": "2026-05-01T12:34:56+00:00"
}
```

- `name`, `description`, and `modified_at` mirror the listing row.
- `body` is the raw markdown body **after** the YAML frontmatter is
  stripped — what the user-facing slash command should render.

The shape mirrors `directory::skills::get` exactly (with `name`
standing in for that surface's `id`) so a single client struct can
target either reader.

# Worked example

After `directory::skills::download {worker: "resend"}` (which writes
both the `index.md` skill body and any `prompts/*.md` prompt files
under `<skills_folder>/resend/`):

```json
{}
```

→ `directory::prompts::list` returns one row per prompt the worker
shipped (e.g. `[{"name": "send-email", "description": "...",
"modified_at": "..."}]`).

```json
{ "name": "send-email" }
```

→ `directory::prompts::get` returns that prompt's body alongside
the same `name` / `description` / `modified_at` fields.

# Side effects

After every successful `directory::skills::download` that wrote at
least one prompt markdown, the worker fires
`directory::prompts::on-change` with payload
`{ "op": "download", "namespace": "<ns>", "source": "repo" | "registry" }`.
Subscribers (e.g. the `mcp` worker) use this to forward MCP
`notifications/prompts/list_changed` to their clients without
re-polling.

# Related

- `directory::skills::list` / `directory::skills::get` — same flat
  shapes for the *skill* surface (`id` instead of `name`).
- `directory::skills::download` — the only write path. Pulls both
  skill markdown and prompts into `skills_folder` from the public
  registry or a GitHub repo.
