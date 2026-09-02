---
type: index
name: iii-directory
description: >-
  Discovery entry point for the engine — search the live function catalog,
  read the skills, system prompts, and agent profiles that installed workers
  ship off local disk, browse the public iii workers registry over HTTP, and
  install new worker bundles. Reach for it first to find out which workers
  exist and how to call them.
---

# iii-directory

The directory worker is how an agent finds its way around the engine. It exposes
five surfaces: function search (`directory::search_functions`), installed-worker
docs (`directory::skills::*`), chat identity prompts
(`directory::system-prompts::*`), reusable agent profiles
(`directory::agents::*`), and the public worker catalogue
(`directory::registry::*`). A download pulls a bundle
onto disk, and each filesystem-backed family also takes direct `create` /
`update` / `delete` calls — those are this worker's only writes. Everything
else here is read-only.

Two kinds of id flow through this worker and they must not be mixed up. A
**callable id** uses `::` (`directory::skills::get`) and goes in the `function:`
field of `agent_trigger`. A **skill id** uses `/` (`iii-sandbox`,
`agent-memory/observe`) and names a *document* — pass it as the `id` argument to
`directory::skills::get`. The ids that `list` and `index` print are skill ids; a
worker's overview is the bare worker name (`iii-sandbox`, not `iii-sandbox/index`,
and the `iii-` prefix is never dropped). Use the id you were given — do not
invent one.

Only **installed** workers are visible. `index`, `list`, and `get` show on-disk
skills for installed workers, plus this worker and the `iii` engine which are
always present. A skill you know exists stays invisible until its worker is
downloaded, so when one is missing, install it and look again. With
`auto_download` enabled the worker subscribes to the engine `worker` add event
and pulls a newly added worker's skills automatically, so freshly installed
workers can appear without a manual download. System-installed agent skills
under the read-only `agents_skills_folder` (`.agents/skills` under
`III_COMPOSE_DIR`, or the process current directory when standalone) are
also always visible in `list`/`get` — they are skills, not workers, so they
never appear in `index` and `update`/`delete` refuse them.

## When to Use

- You need functions for one to six unmet capabilities — `directory::search_functions`.
- You need to see which workers are installed — `directory::skills::index` (token-light; start here).
- You need to read a worker's overview or a deeper doc it linked to — `directory::skills::get`.
- You need to find a skill across the repo with filters — `directory::skills::list`.
- You need the system prompts the chat picker offers as an identity override — `directory::system-prompts::list` / `get`.
- You need a reusable agent profile (display name, emoji logo, skill selection, and its system prompt) — `directory::agents::list` / `get`.
- You are about to build against a worker you have **not** installed — `directory::registry::workers::info` returns the same schema shape you would get after install.
- You need to install a published worker's skills — `directory::skills::download_from_registry`.
- You can only reach the `directory::` namespace but need one engine function's exact schema — `directory::engine::functions::info`.

## Boundaries

- Only installed workers are visible. If the engine daemon is unreachable at boot, filtering is skipped and everything on disk is shown instead.
- Writes are downloads plus the per-family `create` / `update` / `delete` calls; every read function leaves disk untouched.
- Skills under `agents_skills_folder` are read-only: `update`/`delete` refuse them (`D116`), and `create` refuses ids in their namespaces (`D115`). Edit them with their owning tool, or copy one into `skills_folder` on disk to fork it.
- Not the live-connection view. `directory::*` reflects what is on disk or in the registry, not what is connected right now. For that, call the engine directly (`engine::functions::list`, `engine::workers::list`, …); daemon-managed providers (`http`, `cron`, `state`) open no WebSocket, so merge `worker::list` by `name`.
- Do not put a skill id (`/`) in `agent_trigger`'s `function:` field, and do not pass a function id (`::`) to `directory::skills::get`.
- System prompt files without a `description:` in frontmatter are silently skipped by `directory::system-prompts::list`.
- Skills and system prompts share `skills_folder`; a `system-prompts/` path component selects the prompt family, other `prompts/` paths are ignored, and `agents/` is reserved. Agent profiles are direct `<agents_folder>/<id>.md` files.
- An agent profile's `skills:` list is curation, not enforcement: it narrows what a skill index shows, grants no access, and unknown ids are reported by `get` as `unknown_skills` warnings rather than errors. `agents_folder` profiles are unrelated to the read-only `agents_skills_folder` (`~/.agents/skills`), which holds external tools' skills.
- Registry answers (`registry::workers::list` / `info`) are cached ~60 s per unique input by default (`registry_cache_ttl_ms`) — change a parameter to refresh.

## Functions

- `directory::search_functions` — find compact installed and installable function candidates for one to six required capabilities.
- `directory::skills::index` — token-light per-worker overview, one block per installed worker; truncates and tells you to call `list` when large.
- `directory::skills::list` — enumerate every visible skill with id/title/type/description/bytes/modified_at; narrow with `search`, `prefix`, `type`, or `include_description`.
- `directory::skills::get` — read one skill doc by its skill id; forgiving about short names, a trailing `.md`, an `iii://` prefix, and `SKILL.md` filenames. The response's `path` is the absolute on-disk file; its parent directory is the skill's base directory, where payload the body references by relative path (`scripts/`, `reference/`) lives.
- `directory::skills::download_from_registry` — install a published worker's skills from the registry; `worker` required, pin with `version` XOR `tag` (default `tag: latest`).
- `directory::skills::download_from_repo` — pull one skill folder from a GitHub repo; `repo` + `skill` required, `branch` defaults to `main`.
- `directory::skills::download` — flexible alias accepting either source set; prefer the two explicit forms so the source is unambiguous.
- `directory::skills::update` — overwrite one EXISTING skill with new full-file markdown (frontmatter included); never creates — author with `directory::skills::create` or materialize a bundle with a download first.
- `directory::skills::create` — create a NEW skill at `<skills_folder>/<id>.md` from full-file content; refuses an id that already resolves in the visible set, an existing target path, and ids the visibility filter (or a system-installed agents namespace) would hide.
- `directory::skills::delete` — permanently remove one EXISTING skill by id (same forgiving id forms as `get`); cleans up parent directories left empty.
- `directory::system-prompts::list` — list the system prompts the chat picker offers; the response array is named `prompts`.
- `directory::system-prompts::get` — read one system prompt's body by name; `raw: true` also returns the full on-disk file for round-tripping.
- `directory::system-prompts::create` — create a NEW system prompt at `<skills_folder>/system-prompts/<name>.md`; refuses an existing name or target path.
- `directory::system-prompts::update` — overwrite one EXISTING system prompt; the frontmatter must keep a non-empty `description` (a declared `name` renames it).
- `directory::system-prompts::delete` — permanently remove one EXISTING system prompt by name.
- `directory::agents::list` — list agent profiles (id, display name, description, emoji logo, `extends`, `skill_count` where null means every skill; `model`/`reasoning_effort`/`skill_count` are resolved through `extends`). The bundled bases `iii` (the harness default identity) and `iii-minimal` (the compact directory-first identity) are always listed with `builtin: true`; a row with `inheritance_error` has a broken `extends` chain.
- `directory::agents::get` — read one agent profile by id: its RESOLVED system prompt (ancestors' bodies root-first via `extends`, then its own; blank bodies are skipped, so a prompt-less profile serves its parent chain, or `""` when it has no parent), skill filter + `unknown_skills`, and `model` (the profile's default model id — use it when spawning/sending with this profile; `null` = caller decides); `raw: true` returns the profile's OWN file for editing. `inheritance_error` set = fix `extends` before running it.
- `directory::agents::create` — create a NEW agent profile at `<agents_folder>/<id>.md` from full-file content; frontmatter needs a non-empty `name`, `logo` is emoji-only, and the body — the system prompt — may be empty; an `extends: <id>` that does not resolve is not a write error — `get` reports it as `inheritance_error`.
- `directory::agents::update` — overwrite one EXISTING agent profile (same scanner rules; the id stays the file stem, frontmatter `name` is display-only). Updating the bundled `iii` creates the local file that shadows it.
- `directory::agents::delete` — permanently remove one EXISTING agent profile by id; running sessions are unaffected, profiles extending it stop resolving until fixed. Deleting a local `iii` falls back to the bundled copy.
- `directory::registry::workers::list` — page through published workers in the public registry (`pagination.next_cursor` feeds the next page's `cursor`).
- `directory::registry::workers::info` — full registry detail for one worker, including ones not installed: `api_reference` (functions + triggers with schemas) and `skills_tree`.
- `directory::engine::functions::info` — thin proxy to the engine's `engine::functions::info`; returns request/response schema, metadata, and registered triggers for one function id.

A failed call returns one plain sentence carrying a `Did you mean:` suggestion and a `Next:` function to call (codes `D110`/`D112`/`D210`/`D310`/`D311`, `D410` for a missing agent profile, `D320` when the registry is unreachable, and on the write paths `D213` for content the next scan would skip, `D214`/`D114`/`D414` for a create whose name/id or target path is already taken, `D115` for a skill id the visibility filter or an agents namespace reserves, and `D116` for a write to a read-only system-installed skill) — follow it instead of retrying the same input. Downloads overwrite file-by-file, so hand-edited extra files survive a re-pull.

## Reactive triggers

The worker publishes three custom trigger types, one per kind —
`directory::skills::on-change`,
`directory::system-prompts::on-change`, and `directory::agents::on-change`. Each fires for its own kind only, on any
of: a download that wrote at least one file of that kind (`op: "download"`), that
family's `update`, `create`, or `delete` (`op: "update"` / `"create"` /
`"delete"`), or a change made to
that kind's files directly on disk, outside this worker (`op: "external"` — a
file pasted in, edited in an external editor, deleted, or renamed). Bind one when
a *different* worker must react to the on-disk set changing; the `mcp` worker uses
this to emit `notifications/*_list_changed` to its clients without re-polling.
Direct `<id>.md` profile edits under `agents_folder` fire
`directory::agents::on-change`; nested files there are ignored.

Reach for it when:

- A worker caches the skill, system-prompt, or agent-profile list and must invalidate it on change.
- You want a push the moment new bundles install, instead of polling `directory::skills::list`.

Do not bind when:

- You ran the download yourself — its return payload already lists `skills_written` / `system_prompts_written` / `agents_written`.
- Your own reaction writes `.md` files under `skills_folder` through some path OTHER than this worker's `update` / `create` (a shell or coder worker, say). Writes made through this worker are suppressed, but outside writes are not: your handler would re-trigger itself.

### How to bind

1. Register a handler: `registerFunction('my-worker::on-skills-changed', handler)`.
2. Register the trigger:

```typescript
iii.registerTrigger({
  type: 'directory::skills::on-change',
  function_id: 'my-worker::on-skills-changed',
})
```

Delivery is fire-and-forget (best-effort, at-most-once): a slow or failing
subscriber is logged and skipped so it cannot block the write path. Direct edits
under `skills_folder` DO fire it now, as `op: "external"` — a filesystem watch
supplies that, coalescing a burst into one event per kind. Read calls never fire
it, and neither does this worker's own writing twice (a `create` or `update`
sends its precise op, not an extra `external`). The watch is a doorbell, not a
ledger: every read re-scans disk, so a missed event costs a stale open view until
the next call, never data. For the event payload shape, call `get function info`
on the trigger type.
