# iii-directory

Engine introspection, workers registry proxy, and filesystem-backed
skill + prompt reader for the [iii engine](https://github.com/iii-hq/iii).
Every public function sits under a single `directory::*` namespace,
split into four sub-namespaces (all MCP-agnostic):

| Surface | What clients see | When to use it |
|---|---|---|
| **Skills** (`directory::skills::*`) | Markdown documents under `iii://{id}` plus an `iii://directory/skills` index | Orientation: "when and why to use my worker's tools" |
| **Prompts** (`directory::prompts::*`) | Static prompt templates listed by `directory::prompts::list` and read by `directory::prompts::get` | Parametric command templates the *user* invokes |
| **Engine** (`directory::engine::*`) | Read-side enrichment over `engine::functions::list`, `engine::workers::list`, `engine::trigger-types::list`, `engine::triggers::list` | "What's connected to the engine right now?" |
| **Registry** (`directory::registry::*`) | HTTP proxy over `api.workers.iii.dev` with the same `workers::{list,info}` shape as `directory::engine::workers::*` | "What's published in the public registry?" |

Skills and prompts are sourced from a single configured folder on disk
(`skills_folder`). The only write path is the
**`directory::skills::download`** function, which pulls markdown into
`skills_folder` from either the
[workers registry](https://workers.iii.dev) or a GitHub repo. Once
downloaded, files belong to the developer — edit them however you want.

`directory::engine::workers::*` and `directory::registry::workers::*`
share the same envelope shape so callers can switch between the local
engine view and the published-registry view without re-learning the API.

## Table of contents

1. [Install](#install)
2. [Configuration](#configuration)
3. [Quickstart: download some skills](#quickstart-download-some-skills)
4. [On-disk layout](#on-disk-layout)
5. [URI scheme](#uri-scheme)
6. [Functions](#functions)
7. [Custom trigger types](#custom-trigger-types)
8. [Local development & testing](#local-development--testing)
9. [Migration from skills v0.2.x](#migration-from-skills-v02x)

---

## Install

```bash
iii worker add iii-directory
```

`iii worker add` fetches the binary, writes a config block into
`~/.iii/config.yaml`, and the engine starts the worker on the next
`iii start`.

---

## Configuration

```yaml
# Folder that backs every read (`iii://`, `directory::skills::fetch-skill`,
# `directory::skills::list`, `directory::prompts::*`) and every write
# from `directory::skills::download`. Relative paths are resolved
# against the process current working directory; absolute paths are
# used as-is.
skills_folder: ./skills

# Workers registry base URL — used by `directory::skills::download`
# and the `directory::registry::*` proxies when a `worker=` source is
# specified. Override for self-hosted deployments.
registry_url: https://api.workers.iii.dev

# Timeout for a single download (`git clone` or HTTP request) in ms.
download_timeout_ms: 60000
```

The folder is created on first download if it doesn't exist.

---

## Quickstart: download some skills

```bash
# Pull a specific worker's skills + prompts at a fixed semver from
# the registry. Files land under `<skills_folder>/agent-memory/`.
iii trigger --function-id=directory::skills::download \
  --payload='{"worker": "agent-memory", "version": "1.2.3"}'

# Same, but always fetch whatever's tagged `latest` (also the default
# when neither version nor tag is given).
iii trigger --function-id=directory::skills::download \
  --payload='{"worker": "agent-memory"}'

# Pull a single subfolder out of a public GitHub repo via
# `git clone --depth 1 --branch main`. Files land under
# `<skills_folder>/frontend-design/`. The `branch` field defaults to
# `main`; pass `"master"` for older repos that haven't migrated.
iii trigger --function-id=directory::skills::download \
  --payload='{
    "repo": "https://github.com/anthropics/skills",
    "skill": "frontend-design"
  }'
```

The response is `{ namespace, skills_written, prompts_written, source }`
where `skills_written` and `prompts_written` are arrays of relative
paths / prompt names that were materialised in this run.

After every successful download the worker fires the
`directory::skills::on-change` and/or `directory::prompts::on-change`
trigger types so that subscribers like the [`mcp`](../mcp/) worker can
forward MCP `notifications/list_changed` to their clients.

---

## On-disk layout

The worker assumes a fixed layout under `skills_folder`:

```text
skills_folder/
  <namespace>/                 # one folder per `directory::skills::download` namespace
    index.md                   # → iii://<namespace>/index
    contacts.md                # → iii://<namespace>/contacts
    emails/send-email.md       # → iii://<namespace>/emails/send-email
    prompts/                   # ← magic marker for prompts
      send-email.md            # ← MCP slash-command (needs YAML frontmatter)
      triage.md
```

A few rules:

- **Skill ids** are the relative path under `skills_folder` with `.md`
  stripped. Each segment must satisfy `[a-z0-9_-]{1,64}` and the first
  segment must not be the literal `fn` (reserved for section URIs).
- **Prompts** live under any `*/prompts/*.md` path. They must start with
  a YAML frontmatter block declaring at least `description`; `name`
  is optional and overrides the file-stem default.
- Files anywhere else (i.e. *not* in a `prompts/` segment) are skills.

The download function namespaces by source:

| Source | Destination |
|---|---|
| `repo=URL skill=NAME branch?=main` | `<skills_folder>/<NAME>/...` |
| `worker=NAME version=…` | `<skills_folder>/<NAME>/...` |
| `worker=NAME tag=…` (default `tag=latest`) | `<skills_folder>/<NAME>/...` |

Re-pulling the same source overwrites files **file-by-file** —
existing siblings outside the response set are preserved (so
hand-edited additions survive a re-pull).

---

## URI scheme

Same scheme as previous releases, anchored now on the filesystem:

| URI | Returns |
|---|---|
| `iii://directory/skills` | Auto-rendered markdown index of every skill in `skills_folder`. |
| `iii://{id}` | The body at `<skills_folder>/{id}.md`. Any depth. First segment must NOT equal `fn`. |
| `iii://fn/{a}/{b}/.../{leaf}` | Trigger function `a::b::...::leaf` with `{}` and serve its output. Each `/` after `fn/` becomes `::`. |

The `directory::skills::fetch-skill` function is a thin batched wrapper
over the same resolver. Pass either `uri` (single) or `uris` (array) —
each entry may be a full `iii://` URI or a bare skill path (the `id`
returned by `directory::skills::list`, auto-prefixed with `iii://`).
Sections are wrapped as `# {uri}\n\n{body}` and joined with
`\n\n---\n\n`.

There is no recursion guard on `iii://fn/<path>` URIs — the resolver
will trigger any function the engine exposes, including `state::*` and
`engine::*` infra. Adapters that surface
`directory::skills::fetch-skill` to untrusted callers should add their
own filtering.

---

## Functions

Fifteen functions, all under `directory::*`. All registrations are
namespace-clean; this worker is intentionally agnostic to MCP and any
other adapter.

### `directory::skills::*` (filesystem reader)

| Function ID | Description |
|---|---|
| `directory::skills::download` | Pull markdown into `skills_folder`. Either `{repo, skill, branch?}` (defaults `branch=main`) or `{worker, version?|tag?}` (defaults `tag=latest`). |
| `directory::skills::list` | Metadata-only listing of every fs-backed skill. |
| `directory::skills::fetch-skill` | Batched read across one or more `iii://` URIs or bare skill paths (returns plain markdown). |

### `directory::prompts::*` (filesystem reader)

| Function ID | Description |
|---|---|
| `directory::prompts::list` | Metadata-only listing of every fs-backed prompt. |
| `directory::prompts::get` | Fetch one prompt's body + `{name, description, modified_at}`. Plain shape, no envelope. |

### `directory::engine::*` (engine introspection)

| Function ID | Description |
|---|---|
| `directory::engine::functions::list` | List functions registered with the engine; filter by search/prefix/worker. |
| `directory::engine::functions::info` | Single-function detail: schemas, owning worker, registered triggers, bundled how-to. |
| `directory::engine::triggers::list` | List trigger TYPES registered with the engine; filter by search/prefix/worker. |
| `directory::engine::triggers::info` | Single trigger-type detail: configuration schema, return schema, instance count. |
| `directory::engine::registered-triggers::list` | List registered trigger INSTANCES (subscriber rows). |
| `directory::engine::registered-triggers::info` | Composite: instance + trigger-type detail + function detail. |
| `directory::engine::workers::list` | List workers connected to the engine; same row shape as `directory::registry::workers::list`. |
| `directory::engine::workers::info` | One worker's `worker` envelope + functions + trigger types + registered triggers. |

### `directory::registry::*` (workers registry HTTP proxy)

| Function ID | Description |
|---|---|
| `directory::registry::workers::list` | Search published workers in `api.workers.iii.dev`. Same row shape as `directory::engine::workers::list`. |
| `directory::registry::workers::info` | Full registry detail for one worker: `worker` envelope (matching `directory::engine::workers::info.worker`) plus `readme`, `api_reference`, `skills_tree`. |

Both `directory::registry::*` responses are cached in-process for
`registry_cache_ttl_ms` (default 60s).

There is **no** `directory::skills::register` /
`directory::prompts::register` — see
[Migration](#migration-from-skills-v02x) below.

---

## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `directory::skills::on-change` | After a `directory::skills::download` that wrote at least one skill markdown file | `{ "op": "download", "namespace": "<ns>", "source": "repo" \| "registry" }` |
| `directory::prompts::on-change` | After a `directory::skills::download` that wrote at least one prompt markdown file | `{ "op": "download", "namespace": "<ns>", "source": "repo" \| "registry" }` |

Dispatches are fire-and-forget (Void), so the download path doesn't
block on downstream latency.

---

## Local development & testing

### Run from source

```bash
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml
```

### Tests

```bash
# Fast, offline — exercises the pure helpers (markdown / URI / validators)
# without needing an iii engine.
cargo test --lib

# Full BDD suite — requires an iii engine on ws://127.0.0.1:49134
# (or III_ENGINE_WS_URL). The git-backed download scenarios spin up
# a local fixture repo via `git init`; the registry-backed scenarios
# point a wiremock server at the worker's `registry_url` config.
cargo test

# One feature group at a time. Available tags:
#   @engine  @read  @prompts  @download  @download_repo  @download_registry
cargo test --test bdd -- --tags @download
```

The BDD harness lives under [tests/](tests/). Feature files mirror the
modules in [src/functions/](src/functions/). Step definitions under
[tests/steps/](tests/steps/) drive each feature through the same
`iii.trigger` path the production binary uses.
