---
type: how-to
function_id: directory::skills::index
title: Bootstrap an agent harness with a short per-worker skills index
---

# When to use

Call `directory::skills::index` when an agent harness needs to know
**which workers are installed** and how to read each one's full
reference — without paying the token cost of dumping every individual
skill. The response is intentionally short: one `## <worker title>`
heading + that worker's first overview paragraph + a `Read iii://...`
pointer the agent can follow with `directory::skills::get` when it
actually needs the details.

Reach for it when:

- You're bootstrapping a fresh agent session and want the system
  prompt to list the available workers so the model can plan which
  one to drill into.
- You want a copy-paste-ready overview of *workers* (not every skill)
  for a README, changelog, or chat message.
- You need a stable "what's installed?" snapshot keyed by worker
  rather than by skill id.

Use [`directory::skills::list`](iii://directory/skills/list) instead
when you need **per-skill rows** (`id`, `title`, `type`, `bytes`,
`modified_at`) — e.g. for a picker UI, programmatic filtering, or
anything that wants every skill, not just the worker overviews.
Use [`directory::skills::get`](iii://directory/skills/get) to fetch
the full body of any `iii://<ns>/index` link surfaced in the response.

# Inputs

```json
{}
```

No parameters. The worker re-scans `skills_folder` on every call and
re-reads each `type: index` overview to populate the description, so
edits to a worker's `index.md` are visible immediately (same policy
as `directory::skills::list`).

# Outputs

```json
{
  "body": "# Skills index\n\n2 worker(s).\n\n## agent-memory\n\nPersistent memory tier for agents.\n\nRead [`iii://agent-memory/index`](iii://agent-memory/index) for the full worker reference.\n\n## iii-directory\n\nEngine introspection, workers registry proxy, and filesystem-backed skill + prompt reader for the iii engine. ...\n\nRead [`iii://iii-directory/index`](iii://iii-directory/index) for the full worker reference.\n",
  "workers_count": 2
}
```

- `body` is the rendered markdown document. The harness usually
  pastes this verbatim into a system prompt or message.
- `workers_count` is the number of worker entries rendered (i.e. the
  count of `type: index` skills surviving the filter). Cheap sanity
  check that doesn't require re-parsing the body.

# Rendering rules

Only skills with frontmatter `type: index` appear in the body — one
entry per installed worker. Skills of any other type (`how-to`,
`reference`, untyped, ...) are filtered out. This is important: a
how-to skill that happens to live at `<ns>/index.md` (frontmatter
`type: how-to`) will NOT be mistaken for a worker overview.

The body always starts with:

```markdown
# Skills index

<N> worker(s).
```

Then, for every `type: index` skill (sorted lex by id, same order
`directory::skills::list` returns):

```markdown
## <resolved title>

<first paragraph from the overview>

Read [`iii://<id>`](iii://<id>) for the full worker reference.
```

- `<resolved title>` follows the same precedence as every other
  `directory::skills::*` response: frontmatter `title:` wins, then the
  first body `# H1`, then the bare `id` as a last resort.
- The description paragraph is the first non-heading paragraph from
  the worker's `index.md` body (already extracted by the same helper
  `directory::skills::list` uses, so the text matches what a row in
  that listing would carry).
- When the overview body has no paragraph (heading-only file), the
  description block — and its surrounding blank line — is skipped so
  the section stays compact: `\n## <title>\n\nRead ...`.

There is intentionally no `###`, no per-skill bullets, and no nested
grouping. If you need that level of detail for one specific worker,
follow its `iii://<ns>/index` link with `directory::skills::get`.

# Worked example

Given a `skills_folder` that contains two workers (`agent-memory`
with an `index.md` whose frontmatter declares
`title: agent-memory, type: index` and a one-paragraph overview, plus
this `iii-directory` worker's own `index.md`), the response body
looks like:

```markdown
# Skills index

2 worker(s).

## agent-memory

Persistent memory tier for agents. Records observations and recalls
them on demand via `agent-memory::observe` and `agent-memory::recall`.

Read [`iii://agent-memory/index`](iii://agent-memory/index) for the full worker reference.

## iii-directory

Engine introspection, workers registry proxy, and filesystem-backed
skill + prompt reader for the [iii engine](https://github.com/iii-hq/iii).
Every public function sits under a single `directory::*` namespace,
split into four sub-namespaces (all MCP-agnostic):

Read [`iii://iii-directory/index`](iii://iii-directory/index) for the full worker reference.
```

The harness pastes this into the system prompt; when the agent
decides it needs to call a specific function, it follows the
matching `iii://...` link with `directory::skills::get` to pull the
full reference + how-tos.

# Related

- [`directory::skills::list`](iii://directory/skills/list) — same set
  of skills as structured rows (`{ id, title, type, description,
  bytes, modified_at }`) when you want every skill, not just the
  `type: index` overviews.
- [`directory::skills::get`](iii://directory/skills/get) — fetch the
  full body of any `iii://<ns>/index` link surfaced in the response.
- [`directory::skills::download`](iii://directory/skills/download) —
  populate `skills_folder` so there are workers to index.
