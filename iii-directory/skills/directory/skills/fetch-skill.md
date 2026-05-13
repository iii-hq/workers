---
type: how-to
function_id: directory::skills::fetch-skill
title: Batch-read skill bodies by iii:// URI or bare skill path
---

# When to use

Call `directory::skills::fetch-skill` whenever you need the **body** of
one or more skill resources — skill markdown, the auto-rendered skills
index, or the output of a function exposed as a section URI. It's the
canonical read tool for the iii-directory worker; everything else
(`directory::skills::list`, `iii://directory/skills`, the system-prompt
skills bootstrap) ultimately funnels through it.

Reach for it when:

- You hit an `iii://...` link inside another skill and need its
  contents inlined.
- You want the rendered skills index (`iii://directory/skills`) to
  bootstrap an agent's awareness of every available skill.
- You want to call a function and treat the response as a markdown
  section rather than as JSON (`iii://fn/<a>/<b>/...`).

The batch form (`uris: [...]`) is the right call when you have a known
set of links to load up-front — it does one round trip per call and
joins the bodies for you.

# Inputs

Each entry may be either a full `iii://` URI or a bare skill path (the
`id` returned by `directory::skills::list`). Bare paths are
auto-prefixed with `iii://` so callers don't have to repeat the scheme.

Single entry:

```json
{ "uri": "iii://agent-memory/observe" }
```

or, equivalently, a bare path:

```json
{ "uri": "agent-memory/observe" }
```

Batched:

```json
{ "uris": [
  "iii://directory/skills",
  "agent-memory",
  "agent-memory/observe",
  "iii://fn/agent-memory/health"
] }
```

When both `uri` and `uris` are provided, `uris` wins (matches the TS
reference implementation). Empty / blank entries are dropped; a string
that contains `://` but does not start with `iii://` is rejected.

URI shapes accepted:

| URI                                | Resolves to                                                                 |
|------------------------------------|-----------------------------------------------------------------------------|
| `iii://directory/skills`           | Auto-rendered tree-shaped index of every fs-backed skill.                   |
| `iii://{id}`                       | Body of `<skills_folder>/{id}.md`. Any depth. First segment may not be `fn`. |
| `iii://fn/{a}/{b}/.../{leaf}`      | Trigger function `a::b::...::leaf` with `{}` and serve its output as text.  |

# Outputs

A plain markdown string. When multiple URIs are passed, sections are
wrapped as `# {uri}\n\n{body}` and joined with `\n\n---\n\n`.

```text
# iii://agent-memory

## agent-memory router

- [`agent-memory`](iii://agent-memory)
  - [`mem::observe`](iii://agent-memory/observe) — record an event

---

# iii://agent-memory/observe

# How to observe

...
```

There's no JSON envelope; downstream callers should treat the body as
opaque markdown and only inspect / split on the `\n\n---\n\n` boundary
when they passed multiple URIs.

# Worked example

Bootstrap an agent on every root skill in one round trip:

```json
{ "uris": ["iii://directory/skills", "agent-memory", "shell"] }
```

Inline a function-backed section URI from a how-to:

```json
{ "uri": "iii://fn/agent-memory/health" }
```

# Related

- `directory::skills::list` — discover the ids that resolve via
  `iii://{id}` (or pass to this function as bare paths).
- `directory::skills::download` — populate `skills_folder` so there's
  something to fetch.
- `directory::engine::functions::info` — for the **structured** view of
  one function (schemas + how_guide + related_skills) instead of a raw
  `iii://fn/...` body.
