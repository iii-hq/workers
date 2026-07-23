# iii-directory

Workers registry HTTP proxy and filesystem-backed skill + prompt
reader for the [iii engine](https://github.com/iii-hq/iii). Every
public function sits under a single `directory::*` namespace, split
into three sub-namespaces (all MCP-agnostic):

| Surface | What clients see | When to use it |
|---|---|---|
| **Skills** (`directory::skills::*`) | Enriched listing via `directory::skills::list` (`{ id, title, type, description, bytes, modified_at }` per row), a single-skill reader `directory::skills::get { id }` returning `{ id, title, type, description, body, modified_at }`, and `directory::skills::index` which renders a short per-worker overview document (one `## <title>` + first paragraph + `read more` link per `type: index` skill). `title` prefers the YAML frontmatter `title:` over the body H1; `type` is lifted from frontmatter `type:` (e.g. `index`, `how-to`, `reference`) and serialised as `null` when absent. | Orientation: "when and why to use my worker's tools" |
| **Prompts** (`directory::prompts::*`) | Static prompt templates listed by `directory::prompts::list` and read by `directory::prompts::get` | Parametric command templates the *user* invokes |
| **Registry** (`directory::registry::*`) | HTTP proxy over `api.workers.iii.dev` with `workers::{list,info}`. Rows share the core `name` / `description` / `version` fields with the engine's `engine::workers::list` and add publication metadata (`type`, `config`, `supported_targets`, `total_downloads`, `dependencies`, optional `image`). `workers::list` is cursor-paginated with a server-authored page size. | "What's published in the public registry?" |

Engine introspection (functions / triggers / registered triggers /
workers) is served by the engine natively at
`engine::functions::*`, `engine::triggers::*`,
`engine::registered-triggers::*`, and `engine::workers::*`. Earlier
versions of this crate wrapped those calls under `directory::engine::*`
helpers; the wrappers have been removed — call the engine ids
directly.

Skills and prompts are sourced from a single configured folder on disk
(`skills_folder`). The only write path is the
**`directory::skills::download`** function, which pulls markdown into
`skills_folder` from either the
[workers registry](https://workers.iii.dev) or a GitHub repo. Once
downloaded, files belong to the developer — edit them however you want.

`directory::registry::workers::*` and the engine's `engine::workers::*`
share the core `name` / `description` / `version` fields so a parser
that touches only those keys works against either surface; the
registry view also surfaces publication metadata (`type`, `config`,
`supported_targets`, `total_downloads`, `dependencies`, optional
`image`) and the engine view adds runtime / connection state.

## Table of contents

1. [Install](#install)
2. [Configuration](#configuration)
3. [Quickstart: download some skills](#quickstart-download-some-skills)
4. [On-disk layout](#on-disk-layout)
5. [Skill ids](#skill-ids)
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

## Skills

Install the `iii-directory` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill iii-directory
```

Browse or install every worker skill at once:

```bash
npx skills add iii-hq/workers --list
npx skills add iii-hq/workers --all
```

---

## Configuration

Runtime settings live in the **`configuration` worker** under id
**`iii-directory`** (the same pattern `database` and `storage` use). At boot
the worker registers its JSON Schema, reads the live value via
`configuration::get` (the configuration worker env-expands `${VAR}`), and binds
a `configuration` trigger so it re-fetches on change.

Persisted values default to `./data/configuration/iii-directory.yaml` (fs
adapter). Edit that file directly, call `configuration::set id=iii-directory`,
or use the console Workers tab — all three propagate without a redeploy.

### Fields

```yaml
# TOPOLOGY — changing any of these requires a worker restart.
skills_folder: ~/.iii/skills          # read/write root for skills + prompts
local_skills_folder: ./.iii/skills    # project-scoped overrides (whole-namespace local-wins)
auto_download: true                   # subscribe to worker-add + run the boot reconcile
inject_guidance: false                # bind a pre-generate hook that points agent prompts
                                      # at directory::skills::index (opt-in)

# TUNABLE — hot-reload live on `configuration:updated`.
registry_url: https://api.workers.iii.dev   # workers registry base URL
download_timeout_ms: 60000                   # per git-clone / HTTP request timeout (ms)
registry_cache_ttl_ms: 60000                 # in-process TTL for registry::workers::* responses
filter_unregistered: true                    # hide skills whose namespace isn't an installed worker
```

The `skills_folder` is created on first download if it doesn't exist.

### Prompt guidance hook

With `inject_guidance: true`, the worker binds a `harness::hook::pre-generate`
trigger (`on_error: fail_open`) at boot and appends a short pointer to
`directory::skills::index` to the agent system prompt while it is connected.
Pointer only: the index body costs context solely when an agent calls the
function. The hook is a no-op when the base prompt is empty or already
mentions the index function. Off by default so a stock deployment leaves
prompts untouched.

### Zero-config default + seed

With no seed and no stored value the worker uses built-in defaults
(`skills_folder: ~/.iii/skills`, `registry_url: https://api.workers.iii.dev`).
Pass `--config <path>` to supply a YAML seed: when present and no value is
stored yet, its contents become `initial_value` on `configuration::register`
(see [`config.yaml.example`](config.yaml.example)). Engine-managed deployments
inline the config under the worker entry; the engine delivers it via `--config`.

### Hot reload

On `configuration::set` (or an external edit to the persisted file), the worker
re-fetches the authoritative value. Tunable changes apply in place and the
registry caches are cleared so a repointed `registry_url` takes effect
immediately. Topology changes (`skills_folder` / `local_skills_folder` /
`auto_download`) are refused with a "restart required" log; the previous
configuration is kept until the worker restarts.

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
trigger types so that subscribers like the [`mcp`](https://github.com/iii-hq/workers/tree/main/mcp) worker can
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
  stripped. Each segment must satisfy `[a-z0-9_-]{1,64}`.
- **Skill frontmatter is optional.** When present, the reader honours
  two keys: `title:` (used by `directory::skills::list` and
  `directory::skills::get` in preference to a body `# H1`) and
  `type:` (free-form classifier surfaced verbatim on both responses).
  Any other YAML keys are ignored.
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

## Skill ids

Skills are addressed by their relative path under `skills_folder` with
`.md` stripped — e.g. `<skills_folder>/agent-memory/observe.md` →
id `"agent-memory/observe"`. The same string is what
`directory::skills::list` returns and what `directory::skills::get`
expects in `{ "id": ... }`. The legacy `iii://{id}` link form is still
accepted on `get` (the prefix is auto-stripped), but the worker no
longer parses any other `iii://` URI shape — bodies are read solely by
id, and the auto-rendered tree-shaped index that previous releases
served at `iii://directory/skills` is gone. Consumers that want a
tree-shaped picker iterate `list` rows themselves and indent by
`id.matches('/').count()`.

---

## Functions

Sixteen functions, all under `directory::*`. All registrations are
namespace-clean; this worker is intentionally agnostic to MCP and any
other adapter.

### `directory::skills::*` (filesystem reader)

| Function ID | Description |
|---|---|
| `directory::skills::download` | Pull markdown into `skills_folder`. Either `{repo, skill, branch?}` (defaults `branch=main`) or `{worker, version?|tag?}` (defaults `tag=latest`). |
| `directory::skills::list` | Enriched listing of every fs-backed skill: `{ id, title, type, description, bytes, modified_at }` per row. `title` prefers the YAML frontmatter `title:` over the body H1, `type` is lifted from frontmatter `type:` (`null` when absent), and `description` is the first paragraph of the body — so consumers can render a picker without a follow-up `get` per row. |
| `directory::skills::get` | Fetch one skill by id. Returns `{ id, title, type, description, body, modified_at }` — same shape `directory::skills::list` rows use, plus the raw markdown `body`. Same title-resolution and `type` precedence as `list`. Accepts a bare id or the same id prefixed with `iii://`. |
| `directory::skills::index` | Render one short markdown entry per installed worker (skills with frontmatter `type: index`). Returns `{ body, workers_count }` where `body` is a ready-to-paste page: `# Skills index`, then one `## <worker title>` heading + the worker's first overview paragraph + a `Read iii://<ns>/index` pointer the agent can follow with `directory::skills::get`. Token-light by design; use `directory::skills::list` for per-skill rows. |

### `directory::prompts::*` (filesystem reader)

| Function ID | Description |
|---|---|
| `directory::prompts::list` | Metadata-only listing of every fs-backed prompt. |
| `directory::prompts::get` | Fetch one prompt's body + `{name, description, modified_at}`. Plain shape, no envelope. |

### Engine introspection (native)

Engine introspection is no longer wrapped here. Call the engine's
native ids directly — every one takes the same filters
(`prefix`, `search`, `worker`, `include_internal` where applicable):

| Function ID | Description |
|---|---|
| `engine::functions::list` | List functions registered with the engine. |
| `engine::functions::info` | Single-function detail: schemas, owning worker. |
| `engine::triggers::list` | List trigger TYPES (the providers, e.g. `http`, `cron`). |
| `engine::triggers::info` | Single trigger-type detail: configuration schema, return schema. |
| `engine::registered-triggers::list` | List trigger INSTANCES (subscriber rows). |
| `engine::registered-triggers::info` | Single registered-trigger detail. |
| `engine::workers::list` | List workers with an open engine WS connection. Daemon-managed providers (`iii-http`, `iii-cron`, `iii-state`) won't appear — call `worker::list` from the supervisor to see those. |
| `engine::workers::info` | One worker's detail by `name`. |

### `directory::registry::*` (workers registry HTTP proxy)

| Function ID | Description |
|---|---|
| `directory::registry::workers::list` | Browse / search published workers in `api.workers.iii.dev`. Optional free-text `search` (matched fuzzy by `pg_trgm`) and opaque `cursor` for pagination; page size is server-authored. Response is `{ workers: [...], pagination: { next_cursor, has_more, page_size } }`. Shares the core `name` / `description` / `version` fields with the engine's `engine::workers::list`. |
| `directory::registry::workers::info` | Full registry detail for one worker. Fans out two parallel registry calls — `GET /w/{slug}` for the worker envelope (publication metadata + readme + functions + triggers) and `GET /w/{slug}/skills` for the skills/prompts tree — and merges them into `{ worker, readme, api_reference, skills_tree }`. The user-facing input still accepts `version:` (semver) or `tag:` (e.g. `latest`); both go on the wire as `?version=…`. |

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
# --config is an optional YAML seed (see config.yaml.example); omit it to
# rely on the value stored in the `configuration` worker (or built-in defaults).
cargo run --release -- --url ws://127.0.0.1:49134 --config ./config.yaml.example
```

### Tests

```bash
# Fast, offline — exercises the pure helpers (markdown / id validators
# / fs source) without needing an iii engine.
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
