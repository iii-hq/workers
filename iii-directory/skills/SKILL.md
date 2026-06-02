---
type: index
title: iii-directory
description: Read skills and prompts off local disk, and browse the public iii workers registry over HTTP. Functions live under directory::skills::*, directory::prompts::*, directory::registry::workers::*, plus the directory::engine::functions::info introspection proxy. Self-contained skill — meant for system-prompt injection; do not re-fetch.
functions:
  - directory::skills::list
  - directory::skills::get
  - directory::skills::index
  - directory::skills::download_from_registry
  - directory::skills::download_from_repo
  - directory::skills::download
  - directory::prompts::list
  - directory::prompts::get
  - directory::registry::workers::list
  - directory::registry::workers::info
  - directory::engine::functions::info
---

# iii-directory

This worker does three things:

1. **Skills** (`directory::skills::*`) — read the markdown docs that workers ship, off local disk. A skill is the "what this worker is and how to use it" doc.
2. **Prompts** (`directory::prompts::*`) — read slash-command templates a human runs (`/send-email`, `/triage`).
3. **Registry** (`directory::registry::*`) — browse the public catalogue of workers at `api.workers.iii.dev`, even ones you have not installed.

## How to call any function here

Every function below is called the same way: pass its **callable id** to `agent_trigger`.

```jsonc
// agent_trigger { function: "directory::skills::list", payload: { } }
```

**Two kinds of id. Do not mix them up.**

| Kind | Looks like | Where it goes |
|------|-----------|---------------|
| **Callable id** (a function) | `directory::skills::get` — uses `::` | the `function:` field of `agent_trigger` |
| **Skill id** (a document) | `iii-sandbox` or `agent-memory/observe` — uses `/` | the `id:` argument you pass to `directory::skills::get` |

The strings `directory::skills::list` returns under `id` are **skill ids** (documents). To READ one, pass it to `directory::skills::get`. Never put a skill id in the `function:` field, and never put a `::` function id into `get`.

## The 3 calls you will use most

**1. See which workers are installed (start here):**
```jsonc
// agent_trigger { function: "directory::skills::index", payload: { } }
```
Returns one short block per installed worker. Pick the worker you want.

**2. Read a worker's overview:**
```jsonc
// agent_trigger { function: "directory::skills::get", payload: { "id": "iii-sandbox" } }
```
The `id` is the bare worker name exactly as `index` printed it.

**3. Read a deeper doc the overview linked to:**
```jsonc
// agent_trigger { function: "directory::skills::get", payload: { "id": "iii-sandbox/exec" } }
```
Use the exact `id` the overview gave you. Do not invent ids.

**Worker not showing up?** It is probably not installed. Install it, then go back to step 1:
```jsonc
// agent_trigger { function: "directory::skills::download_from_registry", payload: { "worker": "iii-sandbox" } }
```

## Which function for which question

| You want to… | Call this |
|--------------|-----------|
| List installed workers (token-light) | `directory::skills::index` |
| List every on-disk skill (with filters) | `directory::skills::list` |
| Read one skill doc | `directory::skills::get` |
| Install a published worker's skills | `directory::skills::download_from_registry` |
| Pull a skill folder from a GitHub repo | `directory::skills::download_from_repo` |
| List prompt templates | `directory::prompts::list` |
| Read one prompt | `directory::prompts::get` |
| Browse published workers in the registry | `directory::registry::workers::list` |
| Full registry detail for one worker | `directory::registry::workers::info` |
| Schemas + triggers for one engine function | `directory::engine::functions::info` |

## Rules a dumb agent gets wrong (read these)

### Rule 1 — Use the id you were given. Do not guess.
The canonical id is whatever `list` or `index` printed. For a worker overview that is the **bare worker name** (`iii-sandbox`, `iii-database`, `agent-memory`). It is NOT `iii-sandbox/index`, and the `iii-` prefix is NOT removed. When in doubt, call `index` or `list` first and copy the id.

### Rule 2 — `get` is forgiving, but a redirect means you guessed.
If your `id` does not match exactly, `get` tries to help instead of failing:
- A short/colloquial name resolves to the full worker: `sandbox` → `iii-sandbox`, `memory` → `agent-memory`.
- A made-up sub-path built from a function id (e.g. `iii-sandbox/sandbox/create`) collapses to that worker's overview.
- A trailing `.md`, an `iii://` prefix, or a `SKILL.md`/`SKILLS.md` filename are all accepted.

When `get` redirects, the body starts with `> Note: no skill <x>. Showing <y> instead.` That note is telling you the id you asked for was wrong and you are now reading the worker overview. Read it, then follow its links with the correct ids.

### Rule 3 — Only INSTALLED workers are visible.
`list`, `index`, and `get` only show skills for workers that are currently installed (plus this `directory` worker and the `iii` engine, which are always visible). A skill you "know" exists will be invisible until its worker is downloaded. If a worker is missing, run `directory::skills::download_from_registry { worker: "<name>" }`, then look again. (If the engine daemon is unreachable at boot, filtering is skipped and everything on disk is shown.)

### Rule 4 — Errors are plain sentences that tell you the fix. Never retry the same input.
A failed call returns ONE sentence, not JSON:

```
D110 not_found: skill "iii-sanbox" does not exist. Did you mean: iii-sandbox. Next: call directory::skills::list to browse skill ids; or directory::skills::index to see the per-worker overview.
```

Do exactly what it says: use an id from `Did you mean:`, or call the function named after `Next:`. Codes you may see:

| Code | Meaning | What to do |
|------|---------|------------|
| `D110` | skill id not found | pick one from `Did you mean:`, or call `list` / `index` |
| `D111` | id was empty/invalid | pass a non-empty skill id |
| `D112` | you passed a FUNCTION id (`a::b`) to `get` | `get` wants a skill id with `/`; to CALL `a::b`, pass it to `agent_trigger` instead |
| `D210` | prompt name not found | call `directory::prompts::list` |
| `D310` | registry worker not found | call `directory::registry::workers::list` |

### Rule 5 — Downloading is the ONLY write. Everything else is read-only.
Three ways in, same engine. Prefer the two explicit ones so the source is unambiguous:
- **From the registry:** `download_from_registry { worker: "<name>" }`. Optionally pin `version: "1.2.3"` (exact) OR `tag: "latest"` — one or the other, not both. Default is `tag: "latest"`.
- **From GitHub:** `download_from_repo { repo: "https://github.com/<org>/<repo>", skill: "<folder>" }`. `branch` defaults to `"main"` (pass `"master"` for old repos).
- `download` is a flexible alias accepting either set; the two above are clearer.

Downloads overwrite file-by-file, so hand-edited extra files survive a re-pull. A write fires `directory::skills::on-change` / `directory::prompts::on-change` so subscribers (like the `mcp` worker) refresh without re-polling.

### Rule 6 — Prompts need a `description` in frontmatter or they vanish.
A prompt file at `<skills_folder>/<ns>/prompts/*.md` must have YAML frontmatter with at least `description:`. Files without it are silently skipped by `directory::prompts::list`. The body `get` returns is the markdown after the frontmatter.

### Rule 7 — Registry answers are cached for 60s.
`registry::workers::list` and `registry::workers::info` cache each unique input for ~60s. Repeating the same call returns the same cached answer. To refresh, wait it out or change a parameter.

### Rule 8 — Before writing code against a worker you have NOT installed, read its registry info.
`registry::workers::info { name: "<worker>" }` returns `api_reference` (functions + triggers with request/response schemas) and `skills_tree` (the docs the bundle ships). This is the same schema shape `engine::functions::info` gives after install, so you can build against it ahead of time.

## `directory::skills::list` filters

`list` returns every visible skill with `id` / `title` / `type` / `description` / `bytes` / `modified_at`. It reads disk live (no cache). Narrow it with optional args:
- `search`: case-insensitive substring over id, title, and description.
- `prefix`: exact id prefix — scope to one worker, e.g. `prefix: "iii-sandbox/"`.
- `type`: exact frontmatter `type:` (`index`, `how-to`, `reference`, …).
- `include_description`: set `false` for a token-light list of just `id` + `title` + `type`.

`directory::skills::index` is the token-light cousin: it shows only `type: index` docs, one per worker, and truncates if the output gets large (it tells you to call `list` when it does).

## Engine introspection

To learn a single engine function's exact schema, this worker wraps ONE engine call:

### `directory::engine::functions::info`
A thin proxy to the engine's native `engine::functions::info`. Use it when you can only reach the `directory::` namespace.
- **Input:** `{ "function_id": "sandbox::create" }` (fully-qualified id, required).
- **Output:** `function_id`; `worker_name`; `description`; `request_schema` / `response_schema` (JSON Schema or null); `metadata`; `registered_triggers` (each with `id`, `trigger_type`, `config`).

```json
{
  "function_id": "sandbox::create",
  "worker_name": "sandbox",
  "description": "Boot a sandbox to run untrusted code.",
  "request_schema": { "type": "object" },
  "response_schema": null,
  "metadata": null,
  "registered_triggers": []
}
```

For "what is connected RIGHT NOW?" there is no `directory::` wrapper — call the engine directly:
- `engine::functions::list` — registered functions.
- `engine::workers::list` / `engine::workers::info` — workers with an open WebSocket.
- `engine::triggers::list` / `engine::trigger-types::list` — registered trigger instances and types.

Note: `engine::workers::list` only sees workers with an open WebSocket. Daemon-managed providers (`iii-http`, `iii-cron`, `iii-state`) do NOT open one — list them with `worker::list` from the supervisor daemon and merge by `name`. See [`iii://iii/index`](iii://iii/index).

## Recipe: a worker says a function/trigger is "unknown"

If `engine::functions::info` (or `trigger-types::info`, `workers::info`) says "not found" but you believe the capability exists, the worker's skill bundle is almost certainly not on disk yet. Recover in order:

1. `directory::registry::workers::list { search: "<worker-name>" }` — confirm it exists in the public registry.
2. `directory::skills::download_from_registry { worker: "<worker-name>" }` — install its bundle. Re-run `directory::skills::index`; the worker now appears.
3. `directory::skills::get { id: "<worker-name>" }` — read the full reference, including any custom trigger types it ships.
4. Still missing from `engine::workers::list` but `worker::list` shows it `running: true`? That is the WebSocket-view vs daemon-view split (Rule above) — merge by `name`.

This is the single most common failure when wiring a new worker into an engine.

## Recipe (advanced): calling an HTTP route you registered

> Skip unless you registered an `http` trigger and need to hit the route. This is about the `iii-http` and `sandbox` workers, not the directory.

After `iii.registerTrigger({ type: 'http', http_method, api_path, ... })` returns OK, the route is served by the `iii-http` worker on ITS host/port — not the engine WebSocket port.

**Find the base URL:**
1. `iii-http` won't show in `engine::workers::list` (no WebSocket). Confirm it is alive with `worker::list` (`running: true`).
2. Get its port from your engine config's `iii-http: { config: { host, port } }` block (harness default `127.0.0.1:3111`), or from `directory::registry::workers::info { name: "iii-http" }`.
3. URL = `<scheme>://<host>:<port><api_path>` → default config + `api_path: "/todos"` = `http://127.0.0.1:3111/todos`.

**Make the request — use `web::fetch`, not shell `curl`.** `web::fetch` returns a parsed `{ ok, status, headers, body }` envelope with size/timeout caps and SSRF protection a shell `curl` lacks:

```jsonc
// agent_trigger { function: "web::fetch", payload: { "url": "http://127.0.0.1:3111/todos" } }
```

**From INSIDE a sandbox:** `127.0.0.1` is the guest's own loopback, not the host. The sandbox daemon rewrites any env value containing `://localhost:<port>` or `://127.0.0.1:<port>`, but **only at `sandbox::create` time** — so pass the iii-http base in as env when you create the sandbox:

```jsonc
// sandbox::create
{
  "image":   "node",
  "network": true,
  "env": [
    "III_ENGINE_URL=ws://127.0.0.1:49134",
    "III_HTTP_BASE=http://127.0.0.1:3111"
  ]
}
// then read $III_HTTP_BASE inside the guest (it resolves to e.g. http://100.96.0.1:3111)
```

**Pitfalls:**
- Guessing a port and calling `127.0.0.1:<port>` from inside a sandbox fails twice — wrong port AND it skips the rewrite.
- `sandbox::exec` timeouts are capped (~30s) by the agent gateway; use the detached-launch pattern for long probes.

## Registry details

- `registry::workers::list`: pages through published workers. With no `search`, rows order by `total_downloads DESC`; with `search`, by fuzzy similarity. Pass `pagination.next_cursor` back verbatim as `cursor:` for the next page; it is `null` on the last page.
- Registry rows share `name` / `description` / `version` with `engine::workers::list`, so a parser reading only those keys works against either. The registry view adds publication metadata (`type`, `config`, `supported_targets`, `total_downloads`, `dependencies`, optional `image`); the engine view adds live connection state.

## Skill / prompt id grammar (the precise rules)

- A skill `id` is the file's path under `skills_folder` with `.md` removed (`agent-memory/observe.md` → `agent-memory/observe`).
- Each `/`-separated segment must match `[a-z0-9_-]{1,64}`; depth is unbounded. Prompt `name` follows the same rule.
- Title shown for a skill: frontmatter `title:` → first `# H1` in the body → the bare id. Description: the first non-heading paragraph (empty if the file is headings only).

## Related

- [`iii://iii/index`](iii://iii/index) — the engine itself: WebSocket model, functions/triggers, "trust runtime probes over introspection".
- [`iii://sandbox/index`](iii://sandbox/index) — sandbox deployment, `network: true`, and the loopback rewrite the HTTP recipe relies on.
- [`iii://web/index`](iii://web/index) — `web::fetch`: the full request/response envelope and the `ok`-vs-`status` rule.
