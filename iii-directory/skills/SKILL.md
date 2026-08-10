---
name: iii-directory
description: >-
  Discovery entry point for the engine — read the skill and prompt docs that
  installed workers ship off local disk, browse the public iii workers registry
  over HTTP, and install new worker bundles. Reach for it first to find out
  which workers exist and how to call them.
---

# iii-directory

The directory worker is how an agent finds its way around the engine. It does
four things: serves the markdown docs installed workers ship (`directory::skills::*`),
serves the slash-command prompt templates a human runs (`directory::prompts::*`),
serves the system prompts the chat picker offers as an identity override
(`directory::system-prompts::*`), and proxies the public worker catalogue at
`api.workers.iii.dev` (`directory::registry::*`). A download pulls a bundle
onto disk, and each filesystem-backed family also takes direct `create` /
`update` calls — those are this worker's only writes. Everything else here is
read-only.

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
workers can appear without a manual download.

## When to Use

- You need to see which workers are installed — `directory::skills::index` (token-light; start here).
- You need to read a worker's overview or a deeper doc it linked to — `directory::skills::get`.
- You need to find a skill across the repo with filters — `directory::skills::list`.
- You need the slash-command prompt templates a worker ships — `directory::prompts::list` / `get`.
- You need the system prompts the chat picker offers as an identity override — `directory::system-prompts::list` / `get`.
- You are about to build against a worker you have **not** installed — `directory::registry::workers::info` returns the same schema shape you would get after install.
- You need to install a published worker's skills — `directory::skills::download_from_registry`.
- You can only reach the `directory::` namespace but need one engine function's exact schema — `directory::engine::functions::info`.

## Boundaries

- Only installed workers are visible. If the engine daemon is unreachable at boot, filtering is skipped and everything on disk is shown instead.
- Writes are downloads plus the per-family `create` / `update` calls; every read function leaves disk untouched.
- Not the live-connection view. `directory::*` reflects what is on disk or in the registry, not what is connected right now. For that, call the engine directly (`engine::functions::list`, `engine::workers::list`, …); daemon-managed providers (`http`, `cron`, `state`) open no WebSocket, so merge `worker::list` by `name`.
- Do not put a skill id (`/`) in `agent_trigger`'s `function:` field, and do not pass a function id (`::`) to `directory::skills::get`.
- Prompt files without a `description:` in frontmatter are silently skipped by `directory::prompts::list` and `directory::system-prompts::list` alike.
- The two prompt families are split by path segment, not by frontmatter: a `system-prompts/` path component makes a file a system prompt, a `prompts/` component makes it a command template, and `system-prompts/` wins if a path carries both. Names are per-family, so the same name can exist as both kinds.
- Registry answers (`registry::workers::list` / `info`) are cached ~60 s per unique input by default (`registry_cache_ttl_ms`) — change a parameter to refresh.

## Functions

- `directory::skills::index` — token-light per-worker overview, one block per installed worker; truncates and tells you to call `list` when large.
- `directory::skills::list` — enumerate every visible skill with id/title/type/description/bytes/modified_at; narrow with `search`, `prefix`, `type`, or `include_description`.
- `directory::skills::get` — read one skill doc by its skill id; forgiving about short names, a trailing `.md`, an `iii://` prefix, and `SKILL.md` filenames.
- `directory::skills::download_from_registry` — install a published worker's skills from the registry; `worker` required, pin with `version` XOR `tag` (default `tag: latest`).
- `directory::skills::download_from_repo` — pull one skill folder from a GitHub repo; `repo` + `skill` required, `branch` defaults to `main`.
- `directory::skills::download` — flexible alias accepting either source set; prefer the two explicit forms so the source is unambiguous.
- `directory::skills::update` — overwrite one EXISTING skill with new full-file markdown (frontmatter included); never creates, so materialize a bundle with a download first.
- `directory::prompts::list` — list slash-command prompt templates (only files carrying a frontmatter `description`).
- `directory::prompts::get` — read one prompt template's body by name; `raw: true` also returns the full on-disk file for round-tripping.
- `directory::prompts::create` — create a NEW command template at `<skills_folder>/prompts/<name>.md` from full-file content; refuses a name already in the command scan and an existing target path.
- `directory::prompts::update` — overwrite one EXISTING command template; the frontmatter must keep a non-empty `description` (a declared `name` renames it).
- `directory::system-prompts::list` — list the system prompts the chat picker offers (same shape as `prompts::list`, including the `prompts` field name).
- `directory::system-prompts::get` — read one system prompt's body by name; `raw: true` as above.
- `directory::system-prompts::create` — create a NEW system prompt at `<skills_folder>/system-prompts/<name>.md`; same rules and refusals as `prompts::create`, scoped to the system scan.
- `directory::system-prompts::update` — overwrite one EXISTING system prompt; same rules as `prompts::update`.
- `directory::system-prompts::delete` — permanently remove one EXISTING system prompt by name.
- `directory::registry::workers::list` — page through published workers in the public registry (`pagination.next_cursor` feeds the next page's `cursor`).
- `directory::registry::workers::info` — full registry detail for one worker, including ones not installed: `api_reference` (functions + triggers with schemas) and `skills_tree`.
- `directory::engine::functions::info` — thin proxy to the engine's `engine::functions::info`; returns request/response schema, metadata, and registered triggers for one function id.

A failed call returns one plain sentence carrying a `Did you mean:` suggestion and a `Next:` function to call (codes `D110`/`D112`/`D210`/`D310`/`D311`, `D320` when the registry is unreachable, and on the write paths `D213` for content the next scan would skip and `D214` for a create whose name or target path is already taken) — follow it instead of retrying the same input. Both prompt families share those codes; the message names which kind it means ("prompt" vs "system prompt") and its `Next:` stays inside that family. Downloads overwrite file-by-file, so hand-edited extra files survive a re-pull.

## Reactive triggers

The worker publishes three custom trigger types, one per kind —
`directory::skills::on-change`, `directory::prompts::on-change`, and
`directory::system-prompts::on-change`. Each fires for its own kind only, on any
of: a download that wrote at least one file of that kind (`op: "download"`), that
family's `update` or `create` (`op: "update"` / `"create"`), or a change made to
that kind's files directly on disk, outside this worker (`op: "external"` — a
file pasted in, edited in an external editor, deleted, or renamed). Bind one when
a *different* worker must react to the on-disk set changing; the `mcp` worker uses
this to emit `notifications/*_list_changed` to its clients without re-polling.

Reach for it when:

- A worker caches the skill or prompt list and must invalidate it on change.
- You want a push the moment new bundles install, instead of polling `directory::skills::list`.

Do not bind when:

- You ran the download yourself — its return payload already lists `skills_written` / `prompts_written` / `system_prompts_written`.
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
