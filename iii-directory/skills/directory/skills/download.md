---
type: how-to
function_id: directory::skills::download
title: Download skills + prompts into skills_folder
---

# When to use

Call `directory::skills::download` when you want to populate
`skills_folder` with markdown — either from the public workers registry
(`api.workers.iii.dev`) or from a GitHub repo. This is the **only**
write path on the iii-directory worker; everything else
(`directory::skills::list`, `directory::skills::get`,
`directory::prompts::*`) reads from whatever ends up on disk.

Reach for it when:

- You're provisioning a fresh machine and need a worker's bundle pulled
  locally so `directory::skills::get` can serve it.
- You want to pin a worker's skills to a known semver instead of always
  tracking `tag: "latest"`.
- You want to vendor an out-of-registry skill bundle from a GitHub repo
  for prototyping.

Re-pulling the same source overwrites files **file-by-file** — siblings
outside the response set survive, so hand-edited additions stick around
across re-pulls.

# Inputs

Exactly one source must be specified.

**Source A — GitHub repo:**

```json
{
  "repo":   "https://github.com/<org>/<repo>",
  "skill":  "<folder-under-skills/>",
  "branch": "main"
}
```

Clones with `git clone --depth 1 --branch <branch>` and copies
`skills/<skill>/...` into `<skills_folder>/<skill>/`.

`branch` is optional and defaults to `"main"`. Pass `"master"` (or any
other branch name) for repos whose default branch is not `main`.

**Source B — workers registry:**

```json
{
  "worker":  "agent-memory",
  "version": "1.2.3"
}
```

or

```json
{
  "worker": "agent-memory",
  "tag":    "latest"
}
```

or simply:

```json
{ "worker": "agent-memory" }
```

`version` and `tag` are mutually exclusive. With neither, the call
defaults to `tag: "latest"` (matching
`directory::registry::workers::info`).

# Outputs

```json
{
  "namespace":       "agent-memory",
  "skills_written":  ["index.md", "observe.md", "recall.md"],
  "prompts_written": ["summarize.md"],
  "source":          { "kind": "registry", "worker": "agent-memory", "tag": "latest" }
}
```

For the GitHub source, `source` includes the resolved `branch`:

```json
{ "kind": "repo", "repo": "...", "skill": "frontend-design", "branch": "main" }
```

`namespace` is the destination folder under `skills_folder`.
`skills_written` / `prompts_written` are paths relative to that
namespace (excluding the `prompts/` segment for prompts).

# Side effects

After every successful download the worker fires:

- `directory::skills::on-change` if at least one skill markdown was
  written, with payload
  `{ "op": "download", "namespace": "<ns>", "source": "repo" | "registry" }`.
- `directory::prompts::on-change` if at least one prompt markdown was
  written (same payload shape).

Subscribers (e.g. the `mcp` worker) use these to forward MCP
`notifications/list_changed` to their clients without re-polling.

# Worked example

Pin `agent-memory` to a known semver:

```json
{ "worker": "agent-memory", "version": "1.2.3" }
```

Pull whatever's tagged `latest` (the default when no version/tag is
given):

```json
{ "worker": "agent-memory" }
```

Pull a single subfolder from a public GitHub repo on `main`:

```json
{
  "repo":  "https://github.com/anthropics/skills",
  "skill": "frontend-design"
}
```

Same, but from a `master`-default repo:

```json
{
  "repo":   "https://github.com/<org>/<repo>",
  "skill":  "<folder>",
  "branch": "master"
}
```

# Related

- `directory::skills::list` — verify what landed on disk after the
  download.
- `directory::skills::get` — read a downloaded body by id.
- `directory::registry::workers::list` /
  `directory::registry::workers::info` — discover what's available
  before pulling.
