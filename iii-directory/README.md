# iii-directory

Workers registry HTTP proxy and filesystem-backed skills, system prompts,
and agent profiles for the [iii engine](https://github.com/iii-hq/iii). Every
public function sits under a single `directory::*` namespace, split
into five surfaces (all MCP-agnostic):

| Surface | What clients see | When to use it |
|---|---|---|
| **Skills** (`directory::skills::*`) | Enriched listing via `directory::skills::list` (`{ id, title, type, function_id, disable_model_invocation, description, bytes, modified_at }` per row), a single-skill reader `directory::skills::get { id }` returning `{ id, title, type, function_id, disable_model_invocation, path, body, modified_at }` (the full body instead of the list teaser), and `directory::skills::index` which renders a short per-worker overview document (one `## <title>` + first paragraph + `read more` link per `type: index` skill). Authored by `create`, edited by `update`, removed by `delete`. `title` prefers the YAML frontmatter `title:` (then `name:`) over the body H1; `type` is lifted from frontmatter `type:` (e.g. `index`, `how-to`, `reference`) and serialised as `null` when absent. System-installed agent skills under the read-only `agents_skills_folder` are served too (see [On-disk layout](#on-disk-layout)). | Orientation: "when and why to use my worker's tools" |
| **System prompts** (`directory::system-prompts::*`) | Identity prompts listed by `list`, read by `get`, authored by `create`, edited by `update`, and removed by `delete`. The list response keeps its `prompts` field name. Stored under any `system-prompts/` path segment; `create` writes `<skills_folder>/system-prompts/<name>.md`. | What the chat's system-prompt picker offers as an identity prompt (enrich or replace) |
| **Agent Profiles** (`directory::agents::*`) | Reusable session identities whose file body is the system prompt, with display `name`, emoji `logo`, a `skills` filter, and optional `model` + `reasoning_effort` in required frontmatter. `list` rows carry the display/configuration metadata and `get` adds `system_prompt` and `unknown_skills`. Stored as direct `<agents_folder>/<id>.md` files. See [Agent profile storage](../docs/architecture/agent-profile-storage.md). | A named identity selected with `harness::send { options: { agent } }` |
| **Search** (`directory::search_functions`) | One to six external capabilities → compact function-id candidates (installed, plus registry workers under `installable`), with a conditional pre-generate hint pointing agents at it. | "Which functions do I call for this task?" |
| **Registry** (`directory::registry::*`) | HTTP proxy over `api.workers.iii.dev` with `workers::{list,info}`. Rows share the core `name` / `description` / `version` fields with the engine's `engine::workers::list` and add publication metadata (`type`, `config`, `supported_targets`, `total_downloads`, `dependencies`, optional `image`). `workers::list` is cursor-paginated with a server-authored page size. | "What's published in the public registry?" |

Engine introspection (functions / triggers / registered triggers /
workers) is served by the engine natively at
`engine::functions::*`, `engine::triggers::*`,
`engine::registered-triggers::*`, and `engine::workers::*`. Call the
engine ids directly. One wrapper survives for callers that can only
reach the `directory::` namespace: `directory::engine::functions::info`
proxies a single function's schema (see its row below).

This worker is where an agent's identity text LIVES. `skills/system-prompts/iii-runtime.md`
ships here and is the one copy of "how to work against a live iii engine" —
`claude-code` and `pi` fetch it with `directory::system-prompts::get
name=iii-runtime` (plus `directory::skills::index`) for both their headless
turns and their console terminals, rather than compiling a prompt of their own.
Edit that file and every agent reads the edit; no worker release involved.

Skills and system prompts are sourced from `skills_folder`; agent profiles
use the dedicated `agents_folder`. Writes are the
**`directory::skills::download*`** functions, which pull markdown from either
the [workers registry](https://workers.iii.dev) or a GitHub repo and route each
supported family to its configured root, plus the
per-kind single-file editors — `directory::skills::{create,update,delete}`,
`directory::system-prompts::{create,update,delete}` and
`directory::agents::{create,update,delete}`. Once downloaded, files
belong to the developer — edit them however you want, in the editor of
your choice: a change made directly on disk fires the matching
`on-change` with `op: "external"` (see [Custom trigger
types](#custom-trigger-types)).

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
7. [Function search & pre-generate hint](#function-search--pre-generate-hint)
8. [Custom trigger types](#custom-trigger-types)
9. [Local development & testing](#local-development--testing)
10. [Migration from skills v0.2.x](#migration-from-skills-v02x)

---

## Install

```bash
iii trigger compose::add worker=iii-directory
```

`iii trigger compose::add` resolves the worker and its dependencies, writes
exact declarations to `worker-compose.yaml`, and reconciles the Compose project.

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
or use the Console's global Settings modal — all three propagate without a redeploy.

### Fields

```yaml
# TOPOLOGY — changing any of these requires a worker restart.
skills_folder: skills                  # under III_COMPOSE_DIR, or the process cwd standalone
local_skills_folder: skills/iii        # project-scoped overrides (whole-namespace local-wins)
agents_folder: agents                  # direct <id>.md agent profiles
agents_skills_folder: .agents/skills   # READ-ONLY agent skills, with the same relative-path base
auto_download: true                   # subscribe to worker-add + run the boot reconcile

# TUNABLE — hot-reload live on `configuration:updated`.
registry_url: https://api.workers.iii.dev   # workers registry base URL
download_timeout_ms: 60000                   # per git-clone / HTTP request timeout (ms)
registry_cache_ttl_ms: 60000                 # in-process TTL for registry::workers::* responses
filter_unregistered: true                    # hide skills whose namespace isn't an installed worker
inject_hint: false                           # bind the directory::pre-generate search-hint hook (off: the harness identity prompt already teaches directory-first discovery)
hint_min_workers: 2                          # minimum surface width before the hint fires (0 = always)
registry_search: true                        # include installable registry workers in every search
```

The writable `skills_folder` and `agents_folder` roots are created when needed.

### Zero-config default + seed

With no seed and no stored value the worker uses built-in defaults. Relative
folder paths use `III_COMPOSE_DIR` when set and the process current directory
otherwise (`skills_folder: skills`, `agents_folder: agents`,
`registry_url: https://api.workers.iii.dev`).
Pass `--config <path>` to supply a YAML seed: when present and no value is
stored yet, its contents become `initial_value` on `configuration::register`
(see [`config.yaml.example`](config.yaml.example)). Engine-managed deployments
inline the config under the worker entry; the engine delivers it via `--config`.

### Hot reload

On `configuration::set` (or an external edit to the persisted file), the worker
re-fetches the authoritative value. Tunable changes apply in place and the
registry caches are cleared so a repointed `registry_url` takes effect
immediately. Topology changes (`skills_folder` / `local_skills_folder` /
`agents_folder` / `agents_skills_folder` / `auto_download`) are refused with a "restart
required" log; the previous configuration is kept until the worker restarts.

The writable `skills_folder`, `local_skills_folder`, and `agents_folder` are
watch roots, and the watcher creates each one at boot if it is missing.
Direct `<id>.md` edits under `agents_folder` fire
`directory::agents::on-change` with `op: "external"`; nested files are ignored.
`agents_skills_folder` is a watch root too,
but only when it already exists — the worker never creates (or writes)
anything under it. Install your first agents skill while the worker is
running and that root stays unwatched until the next restart: reads still
serve it, since every read re-scans disk, but the live `external` doorbell
is missing until then. `local_skills_folder` defaults to `skills/iii` under
`III_COMPOSE_DIR`, or under the process current directory when the worker runs
standalone. An empty local root shadows nothing. Because the roots are
restart-required, the watch
roots are fixed for the process lifetime.

---

## Quickstart: download some skills

```bash
# Pull a specific worker's directory bundle at a fixed semver from
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

The response is
`{ namespace, skills_written, system_prompts_written, agents_written, source }`.
The three `*_written` fields list the files materialised in this run.
Registry entries shaped exactly as `agents/<id>.md` land in
`<agents_folder>/<id>.md`; repo downloads do not install agent profiles.

After every successful download the worker fires the
`directory::skills::on-change`, `directory::system-prompts::on-change`,
and/or `directory::agents::on-change` trigger types so that
subscribers like the [`mcp`](https://github.com/iii-hq/workers/tree/main/mcp) worker can
forward MCP `notifications/list_changed` to their clients.

---

## On-disk layout

The worker uses separate roots for directory content and agent profiles:

```text
skills_folder/
  <namespace>/                 # one folder per `directory::skills::download` namespace
    index.md                   # → iii://<namespace>/index
    contacts.md                # → iii://<namespace>/contacts
    emails/send-email.md       # → iii://<namespace>/emails/send-email
    system-prompts/            # ← magic marker for system prompts
      reviewer.md              # ← identity prompt (needs YAML frontmatter)
  system-prompts/              # ← where system-prompts::create writes
    pirate.md

agents_folder/                 # ← where agents::create writes
  release-captain.md           # ← agent profile (needs YAML frontmatter)
  frontend-design.md           # ← `extends: iii` builds on the bundled base
```

Two base agent profiles ship inside the worker binary: `iii` (the harness
default identity, verbatim) and `iii-minimal` (the minimal directory-first
identity — the same text as the bundled system prompt of that name). Each is
always listed (`builtin: true`), a local `agents_folder/<id>.md` shadows it,
`update` on it copy-on-writes that local file, and deleting the file falls
back to the bundled copy. No file is ever seeded on disk.

A second, READ-ONLY root — `agents_skills_folder` (default `.agents/skills`
under the same Compose or standalone base) — serves agent skills. It is
scanned **shallowly**: only `<skill-dir>/SKILL.md` becomes an entry
(id `<skill-dir>/index`, displayed as `<skill-dir>`); a skill's
`reference/`, `scripts/`, or other support payload is never listed. A
missing directory is silently empty. These skills bypass
`filter_unregistered` (their namespaces are skills, not workers), stay
out of `directory::skills::index` (a per-WORKER surface), and are
refused by `update`/`delete` — edit them with their owning tool, or
copy one into `skills_folder` on disk to fork it. Precedence is
`local_skills_folder` > `skills_folder` > `agents_skills_folder`,
namespace-wise: a top-level namespace directory in a higher root
shadows the same namespace below.

A few rules:

- **Skill ids** are the relative path under `skills_folder` with `.md`
  stripped. Each segment must satisfy `[a-z0-9_-]{1,64}`.
- **Skill frontmatter is optional.** When present, the reader recognises
  `title:` (preferred title), `name:` (title fallback), `type:` (free-form
  classifier), `function_id:` (canonical bus function id surfaced by `list`
  and `get`), `description:` (preferred `list` teaser), and
  `disable-model-invocation:` (boolean, default `false`, surfaced as
  `disable_model_invocation` by both responses). The body H1 and first
  paragraph are the title and list-description fallbacks, respectively.
  Disabled skills remain visible to ordinary directory clients; model
  index consumers decide whether to filter them. Any other YAML keys are ignored.
- **System prompts** live under any `*/system-prompts/*.md` path, with
  YAML frontmatter (`description` required, `name` optional). A path carrying
  both a `prompts` and a `system-prompts` segment is a system prompt because
  `system-prompts` is classified first. Other paths containing a `prompts`
  segment are ignored by scans and downloads.
- **What a system prompt can do, by design.** The console's chat picker
  can send a selected system prompt with
  `system_prompt_strategy: "override"`, which replaces the harness's
  built-in identity prompt with that file's body verbatim. Files reach
  `system-prompts/` either by local authoring or via
  `directory::skills::download` from a git repo or the registry, so a
  downloaded bundle can supply one. This is an accepted property, not a
  hole: it takes a deliberate selection in the UI, the same UI already
  accepts arbitrary typed text, and the operator owns `skills_folder`.
  Worth knowing before you point `skills_folder` at a directory other
  people can write to.
- **Agent profiles** are direct `<agents_folder>/<id>.md` files (unrelated to
  the read-only `agents_skills_folder` above, which holds external tools'
  *skills*). Nested profiles and the former
  `<skills_folder>/**/agents/*.md` layout are ignored. Frontmatter is required
  and must declare a non-empty `name`
  (the display name — the id is always the file stem); `description` may
  be empty, `logo` is emoji-only (≤16 bytes, no path characters),
  `skills:` filters the skill index (absent/empty = every skill), and
  `model:` names a model id for sessions using this profile (absent = the
  send decides), and optional `reasoning_effort:` names its provider-native
  effort. Both are stored verbatim and resolved against the live model
  catalog where they are used. The body is the
  system prompt, verbatim, and may be empty — a profile with no prompt of
  its own contributes only its parent chain (if any) and its non-prompt
  settings. Unknown `skills` ids are
  warnings surfaced by `get`, never load failures.
- **Agent profiles inherit.** `extends: <id>` names one parent profile
  (chains allowed, at most 8 hops). The resolved system prompt served by
  `get` is the parent's resolved prompt followed by a blank line and this
  file's body — a blank body contributes nothing, so a profile with no
  prompt of its own serves its parent chain unchanged; `skills`, `model` and `reasoning_effort` fall back to the
  nearest ancestor that sets them when omitted (a non-empty `skills` list
  replaces, never unions); `name`, `description`, `logo`, `icon` and
  `color` are always the profile's own. A chain that does not resolve
  (unknown parent, loop, too deep) is reported by `list`/`get` as
  `inheritance_error` (`D415` text) while the profile still serves its own
  file (so the editor can fix it); the harness refuses to run it.
- Under a skills root, files outside `prompts/`, `system-prompts/`, and
  reserved `agents/` segments are skills.

The download function namespaces by source:

| Source | Destination |
|---|---|
| `repo=URL skill=NAME branch?=main` | `<skills_folder>/<NAME>/...` |
| `worker=NAME version=…` | Skills/prompts under `<skills_folder>/<NAME>/...`; exact `agents/<id>.md` entries under `<agents_folder>/<id>.md` |
| `worker=NAME tag=…` (default `tag=latest`) | Same routing as the version form |

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

Functions sit under `directory::*`. All registrations are
namespace-clean; this worker is intentionally agnostic to MCP and any
other adapter.

### `directory::skills::*` (filesystem reader + editor)

| Function ID | Description |
|---|---|
| `directory::skills::download` | Download directory content. Flexible alias accepting either source set: `{repo, skill, branch?}` (defaults `branch=main`) or `{worker, version?\|tag?}` (defaults `tag=latest`). Prefer the two explicit forms below so the source is unambiguous. |
| `directory::skills::download_from_repo` | Repo-only form: `{repo, skill, branch?}`. Copies one skill folder out of a GitHub repo; paths under `prompts/` and `agents/` are ignored. |
| `directory::skills::download_from_registry` | Registry-only form: `{worker, version?\|tag?}`. Installs a published worker's bundle from `api.workers.iii.dev`, routing exact `agents/<id>.md` entries to `agents_folder`. |
| `directory::skills::list` | Enriched listing of every fs-backed skill: `{ id, title, type, function_id, disable_model_invocation, description, bytes, modified_at }` per row. `title` prefers the YAML frontmatter `title:` over the body H1, `type` is lifted from frontmatter `type:` (`null` when absent), `function_id` identifies the documented bus function when present, and `description` is the frontmatter description or first body paragraph — so consumers can render a picker without a follow-up `get` per row. |
| `directory::skills::get` | Fetch one skill by id. Returns `{ id, title, type, function_id, disable_model_invocation, path, body, modified_at }`. It shares the list row's identity, classification, function, and invocation metadata, but returns the raw markdown `body` and absolute on-disk `path` instead of the teaser and byte count; there is no `description` field. The path's parent directory is the skill's base directory, where payload like `scripts/` and `reference/` lives, and is meaningful only to callers on this worker's machine. Accepts a bare id or the same id prefixed with `iii://`. Pass `raw: true` to additionally get the FULL on-disk file (frontmatter included) as `raw` — the round-trip form `update` takes. |
| `directory::skills::update` | Overwrite one EXISTING skill file with new full-file content: `{ id, content }` where `content` is the edited `raw` from `get { raw: true }`. Validated against the read invariants (size cap, non-empty body after frontmatter); atomic write; fans out `directory::skills::on-change` with `op: "update"`. Never creates files (use `create`). Refuses read-only system-installed skills under `agents_skills_folder` (`D116`). |
| `directory::skills::create` | Create a NEW skill file at `<skills_folder>/<id>.md` from full-file content: `{ id, content }`. Frontmatter is optional (same rules as `update`: size cap, non-empty body). Refuses an `id` that already resolves in the visible set — including the `<id>` → `<id>/index` overview alias and system-installed agents skills — or a target path that already exists on disk (`D114`); an `id` in a namespace reserved by a system-installed agents skill (`D115`); and, while `filter_unregistered` is on, an `id` the visibility filter would immediately hide (`D115`). Atomic write; fans out `directory::skills::on-change` with `op: "create"`. Returns the same shape as `update`. |
| `directory::skills::delete` | Permanently remove one EXISTING skill file by `{ id }` (same id forms as `get`). Resolves against the same visible set as `list`/`get`, refuses read-only system-installed skills under `agents_skills_folder` (`D116`), removes the file plus any parent directories left empty (so a deleted namespace can't keep shadowing a lower-precedence root), and fans out `directory::skills::on-change` with `op: "delete"`. Returns `{ id }` (the resolved on-disk id). |
| `directory::skills::index` | Render one short markdown entry per installed worker (skills with frontmatter `type: index`). Returns `{ body, workers_count }` where `body` is a ready-to-paste page: `# Skills index`, then one `## <worker title>` heading + the worker's first overview paragraph + a `Read iii://<ns>/index` pointer the agent can follow with `directory::skills::get`. Token-light by design; use `directory::skills::list` for per-skill rows. |

### `directory::system-prompts::*` (filesystem reader + editor)

| Function ID | Description |
|---|---|
| `directory::system-prompts::list` | Metadata-only listing of every fs-backed system prompt. |
| `directory::system-prompts::get` | Fetch one system prompt's body + `{name, description, modified_at}`. Plain shape, no envelope. Pass `raw: true` to additionally get the FULL on-disk file (frontmatter included) as `raw`. |
| `directory::system-prompts::update` | Overwrite one EXISTING system prompt file with new full-file content: `{ name, content }`. The frontmatter must keep a non-empty `description` (and a valid `name` when declared) — the same rules the scanner enforces. Atomic write; fans out `directory::system-prompts::on-change` with `op: "update"`. Returns the system prompt's effective name after the write. |
| `directory::system-prompts::create` | Create a NEW system prompt file at `<skills_folder>/system-prompts/<name>.md` from full-file content: `{ name, content }`, where `content` is the FULL file including frontmatter. The frontmatter must carry a non-empty `description` (and a `name` matching the request, when declared) — the same rules `update` enforces. Refuses a `name` that already exists anywhere in the merged system-prompt scan, and a target path that already exists on disk even if the scanner would skip it. Atomic write; fans out `directory::system-prompts::on-change` with `op: "create"`. Returns `{ name, description, bytes, modified_at }`. |
| `directory::system-prompts::delete` | Permanently remove one EXISTING system prompt file by `{ name }`. Resolves against the same merged scan as `list`/`get`, fans out `directory::system-prompts::on-change` with `op: "delete"`, and returns `{ name }`. |

### Agent Profiles — `directory::agents::*` (filesystem reader + editor)

| Function ID | Description |
|---|---|
| `directory::agents::list` | Metadata-only listing of every agent profile — fs-backed plus the bundled `iii` / `iii-minimal` bases (`builtin: true` until a local file shadows one): `{ id, name, description, logo, skill_count, model, reasoning_effort, icon, color, extends, modified_at }` per row, `skill_count`/`model`/`reasoning_effort` resolved through `extends` (`skill_count: null` = every skill; `model: null` = the send decides). A row whose chain does not resolve carries `inheritance_error`. |
| `directory::agents::get` | Fetch one agent profile by `{ id }`: the RESOLVED `system_prompt` (each ancestor's body root-first, then this file's body), `skills` + `unknown_skills` (filter entries matching no visible skill — warnings), `model` (`null` = the send decides), provider-native `reasoning_effort`, display `icon`/`color`, `extends`, `builtin`, `modified_at`, and `inheritance_error` when the chain does not resolve (own file served meanwhile). Pass `raw: true` to additionally get this profile's FULL on-disk file as `raw`. |
| `directory::agents::update` | Overwrite one EXISTING agent profile file with new full-file content: `{ id, content }`. Same rules the scanner enforces (required frontmatter with non-empty `name`, emoji-only `logo`; the body — the system prompt — may be empty); the id stays the file stem. Updating a bundled profile creates the local file that shadows it. Atomic write; fans out `directory::agents::on-change` with `op: "update"`. |
| `directory::agents::create` | Create a NEW agent profile at `<agents_folder>/<id>.md` from full-file content: `{ id, content }`. Refuses an `id` that already exists in the configured agent-profile root, and a target path that already exists on disk even if the scanner would skip it; creating a bundled id shadows the bundled copy. Atomic write; fans out `directory::agents::on-change` with `op: "create"`. Returns `{ id, name, description, logo, bytes, modified_at }`. |
| `directory::agents::delete` | Permanently remove one EXISTING agent profile file by `{ id }`. Resolves against the same configured root as `list`/`get`, fans out `directory::agents::on-change` with `op: "delete"`, and returns `{ id }`. Deleting the local shadow of a bundled profile falls back to the bundled copy; a bundled profile with no local file has nothing to delete (`D414`). Sessions already using the profile are unaffected; profiles extending it stop resolving until fixed. |

### Engine introspection (native, plus one wrapper)

Engine introspection is served natively; call these ids directly — every
one takes the same filters (`prefix`, `search`, `worker`,
`include_internal` where applicable). One wrapper is kept for callers
whose policy only admits the `directory::` namespace:

| Function ID | Description |
|---|---|
| `directory::engine::functions::info` | Thin proxy to `engine::functions::info` for a single `function_id`: request/response schemas, metadata, and registered triggers. The one `directory::engine::*` helper that still exists — reach for it only when you cannot call `engine::*` directly. |

The native ids:

| Function ID | Description |
|---|---|
| `engine::functions::list` | List functions registered with the engine. |
| `engine::functions::info` | Single-function detail: schemas, owning worker. |
| `engine::triggers::list` | List trigger TYPES (the providers, e.g. `http`, `cron`). |
| `engine::triggers::info` | Single trigger-type detail: configuration schema, return schema. |
| `engine::registered-triggers::list` | List trigger INSTANCES (subscriber rows). |
| `engine::registered-triggers::info` | Single registered-trigger detail. |
| `engine::workers::list` | List workers with an open engine WS connection. Daemon-managed providers (`http`, `cron`, `state`) won't appear — call `worker::list` from the supervisor to see those. |
| `engine::workers::info` | One worker's detail by `name`. |

### `directory::registry::*` (workers registry HTTP proxy)

| Function ID | Description |
|---|---|
| `directory::registry::workers::list` | Browse / search published workers in `api.workers.iii.dev`. Optional free-text `search` (matched fuzzy by `pg_trgm`) and opaque `cursor` for pagination; page size is server-authored. Response is `{ workers: [...], pagination: { next_cursor, has_more, page_size } }`. Shares the core `name` / `description` / `version` fields with the engine's `engine::workers::list`. |
| `directory::registry::workers::info` | Pre-install card for one worker. Fans out two parallel registry calls — `GET /w/{slug}` for the worker envelope (publication metadata + readme + functions + triggers) and `GET /w/{slug}/skills` for the skills tree — and merges them into `{ worker, api_reference, skills_tree, readme? }`. The card keeps public function/trigger names and descriptions only: request/response schemas, metadata, internal and search-excluded functions are dropped (browser's schemas alone ran 65 KB; the contract is `engine::functions::info` after installing), and `readme` (10–25 KB) is included only with `readme: true`. The full record stays cached in-process for `directory::search_functions`. Input still accepts `version:` (semver) or `tag:` (e.g. `latest`); both go on the wire as `?version=…`. |

Both `directory::registry::*` responses are cached in-process for
`registry_cache_ttl_ms` (default 60s).

There is **no** `directory::skills::register` — see
[Migration](#migration-from-skills-v02x) below.

---

## Function search & pre-generate hint

One-shot function search over the live engine catalog (hybrid by default:
BM25 fused with the local MiniLM model, reranked; `function_search_mode:
lexical` for BM25 only), absorbed from the former `discovery` worker. It returns only compact `{ function_id,
description }` candidates, grouped by worker in rank order. The model chooses
the candidates it needs, then fetches their contracts in one
`engine::functions::info { function_ids: [...] }` call instead of walking the
catalog with `engine::functions::list`.

| Function | Kind | What it does |
|---|---|---|
| `directory::search_functions` | public | `{ capabilities }` → `{ guidance, workers[], installable[]?, latency_ms }`: BM25 rank over the live engine catalog (at most 6 workers / 12 candidates) plus matching NOT-installed registry workers under `installable`. `capabilities` is a required list of one to six non-empty unmet external capability searches. Requests to summarize provided text/content are ignored. |
| `directory::pre-generate` | internal hook | Injects the conditional search hint into a harness generation (at most once per turn). |
| `directory::on-functions-change` | internal | Refreshes the search catalog on the engine's functions-available push. |
| `directory::hint-preview` | internal | The exact hint text per exposure mode, for the configuration UI. |

Ranking pipeline:

1. **Corpus**: the live engine catalog (boot snapshot + push refresh),
   slimmed to name + first description sentence + argument names. `engine::`
   ids, functions published with `metadata.internal`, and the search itself
   never participate — the worker's own public `directory::*` functions are
   searchable like any other capability.
2. **Scoring**: Okapi BM25 (k1 1.2, b 0.75) with the function name indexed at
   3× weight, camelCase segmentation (`presignUrl` → presign + url), a
   22-word grammatical stoplist, conservative plural folding, JSON-key
   stripping from capability text, and a two-distinct-terms minimum match.
3. **Capability queries**: each `capabilities` entry is ranked independently
   against its own leader and is authoritative. Include every currently unmet
   external capability once in the same call. Write every capability in
   English, translating non-English requests while preserving proper names,
   URLs, and function ids. Requests to summarize provided text or content are
   ignored. Results merge round-robin so every capability gets a candidate
   before any gets a rider.
4. **Pruning**: coverage-aware function floor (≥50% of the leader AND full
   term coverage or ≥85% score) drops same-worker family riders; a
   namespace-level floor (40% of the leader) drops trailing workers.
5. **Registry search** (`registry_search`, default on): every call also
   consults the private workers registry in-process with the same capability
   queries plus informative-term retries (all concurrent; every listed worker
   is a candidate — the registry is team-authored). Candidates merge
   round-robin across search variants; their API references are pooled and
   ranked per capability with BM25 fused with the MiniLM dense lane (same
   0.30 admission floor as the installed catalog), so a capability sharing no
   vocabulary with a contract ("retrieve web news articles" → `web::fetch`)
   still surfaces. Returns up to 2 workers / 6 candidates that WOULD match if
   installed, with `compose::add` guidance.
6. **Session memory** (keyed by caller-supplied OTel baggage, fail-open):
   repeat queries omit candidates already delivered.

The pre-generate hook appends one `<discovery_assist>` block pointing the
model at `search_functions`, telling it to derive capabilities from the goal
and current execution state and to batch selected ids through
`engine::functions::info` — at most once per turn, and only when every gate
clears: the function is callable in the surface, no search result is in the
current task window yet, the surface spans at least `hint_min_workers` distinct
workers, the current task (from the latest user message) has no real function
results yet, and it does not already name a callable function id. Measured on
26 pre-existing e2e scenarios: an unconditional hint *induces* redundant
discovery on guided tasks (up to +110% tokens). The hook ships OFF by default
(`inject_hint: false`): the harness identity prompt already teaches
`directory::search_functions` as the default discovery path, so the hint is
for deployments running a custom identity prompt without that doctrine. The
hook's transcript annotations (`origin.directory`) carry only coarse
outcome/reason and counts.

Knobs (`inject_hint`, `hint_min_workers`, `registry_search`) live in the
`iii-directory` configuration entry and hot-apply — see Configuration.

## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `directory::skills::on-change` | After a `directory::skills::download` that wrote at least one skill markdown file, a `directory::skills::update`, `create`, or `delete`, or external (file pasted/edited/deleted directly on disk — including under `agents_skills_folder`) | download: `{ "op": "download", "namespace": "<ns>", "source": "repo" \| "registry" }`; update/create/delete: `{ "op": "<operation>", "namespace": "<ns>", "id": "<id>" }`; external (file pasted/edited/deleted directly on disk): `{ "op": "external" }` |
| `directory::system-prompts::on-change` | After a `directory::skills::download` that wrote at least one system prompt markdown file, a `directory::system-prompts::update`, `create`, `delete`, or external file change | download: `{ "op": "download", "namespace": "<ns>", "source": "repo" \| "registry" }`; update/create/delete: `{ "op": "<operation>", "name": "<name>" }`; external: `{ "op": "external" }` |
| `directory::agents::on-change` | After a `directory::skills::download` that wrote at least one agent profile, a `directory::agents::update`, `create`, `delete`, or external file change | download: `{ "op": "download", "namespace": "<ns>", "source": "repo" \| "registry" }`; update/create/delete: `{ "op": "<operation>", "name": "<id>" }`; external: `{ "op": "external" }` |

Dispatches are fire-and-forget (Void), so the write path doesn't
block on downstream latency.

The `external` op comes from a filesystem watch over `skills_folder`,
`local_skills_folder`, and (when it already exists) the read-only
`agents_skills_folder`. It is a doorbell, not a ledger: every read re-scans
disk, so a missed event costs a stale open view until the next call, never
data. A burst coalesces into one event per kind, and this worker's own writes
are suppressed — a `create` or `update` sends its precise op and never an
extra `external`.

**Loop hazard for subscribers.** Suppression covers writes made *through this
worker*. A subscriber that reacts to `{ "op": "external" }` by writing `.md`
files under `skills_folder` by some other route — a shell or coder worker, a
script — is not suppressed and will re-trigger itself. Either write through
`directory::*::update` / `create`, or make the reaction idempotent and gated.

---

## Local development & testing

### Run from source

On targets with a pinned static ONNX Runtime (Linux glibc x86_64/aarch64,
Apple Silicon macOS, Windows MSVC) every build links it and MiniLM retrieval
and reranking are compiled in. With no `ORT_LIB_PATH`, `ort-sys` downloads the
runtime for the build target at build time (SHA-256-verified). For offline or
cached x86_64 Linux builds, provision it once and export the path it prints:

```bash
export ORT_LIB_PATH="$(scripts/provision-onnxruntime.sh)"   # idempotent, SHA-256 verified
export ORT_PREFER_DYNAMIC_LINK=0
```

Other targets (musl, armv7, Intel macOS) build the BM25-only worker and need
nothing extra.

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
#   @engine  @read  @download  @download_repo  @download_registry
cargo test --test bdd -- --tags @download
```

The BDD harness lives under [tests/](tests/). Feature files mirror the
modules in [src/functions/](src/functions/). Step definitions under
[tests/steps/](tests/steps/) drive each feature through the same
`iii.trigger` path the production binary uses.
