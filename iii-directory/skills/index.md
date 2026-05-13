---
type: index
title: iii-directory
---

# iii-directory

Engine introspection, workers registry proxy, and filesystem-backed
skill + prompt reader for the [iii engine](https://github.com/iii-hq/iii).
Every public function sits under a single `directory::*` namespace,
split into four sub-namespaces (all MCP-agnostic):

- **Skills** (`directory::skills::*`) — markdown documents under
  `iii://{id}` plus an `iii://directory/skills` index. Use for "when
  and why to use my worker's tools".
- **Prompts** (`directory::prompts::*`) — static prompt templates
  listed by `directory::prompts::list` and read by
  `directory::prompts::get`. Parametric command templates the *user*
  invokes.
- **Engine** (`directory::engine::*`) — read-side enrichment over
  `engine::functions::list`, `engine::workers::list`,
  `engine::trigger-types::list`, and `engine::triggers::list`.
  "What's connected to the engine right now?"
- **Registry** (`directory::registry::*`) — HTTP proxy over
  `api.workers.iii.dev` with the same `workers::{list,info}` shape as
  `directory::engine::workers::*`. "What's published in the public
  registry?"

`directory::engine::workers::*` and `directory::registry::workers::*`
share the same envelope shape, so callers can switch between the local
engine view and the published-registry view without re-learning the API.

Skills and prompts are sourced from a single configured folder on disk
(`skills_folder`); see [the README](../README.md) for the install,
configuration, and `directory::skills::download` flow.

## How-tos

### `directory::skills::*` — filesystem-backed skill reader

- [`directory::skills::list`](iii://directory/skills/list) — enriched listing of every skill on disk (id, title, description, bytes, modified_at).
- [`directory::skills::get`](iii://directory/skills/get) — read one skill body by id (returns the same id/title/description/modified_at as `list` plus `body`).
- [`directory::skills::download`](iii://directory/skills/download) — pull markdown into `skills_folder` from the workers registry or a GitHub repo.

### `directory::prompts::*` — filesystem-backed prompt reader

- [`directory::prompts::*`](iii://directory/prompts) — list and read parametric slash-command templates the *user* invokes; same flat `{ name, description, body, modified_at }` shape `directory::skills::get` uses for skills.

### `directory::engine::*` — what's connected to the engine

- [`directory::engine::functions::list`](iii://directory/engine/functions/list) — list functions registered with the engine; filter by search/prefix/worker.
- [`directory::engine::functions::info`](iii://directory/engine/functions/info) — inspect one function's schemas, owner, and how-to skill.
- [`directory::engine::triggers::list`](iii://directory/engine/triggers/list) — list trigger types registered with the engine.
- [`directory::engine::triggers::info`](iii://directory/engine/triggers/info) — inspect one trigger type's schemas + live instance count.
- [`directory::engine::registered-triggers::list`](iii://directory/engine/registered-triggers/list) — list registered trigger instances (subscriber rows).
- [`directory::engine::registered-triggers::info`](iii://directory/engine/registered-triggers/info) — inspect one registered trigger (instance + type + function).
- [`directory::engine::workers::list`](iii://directory/engine/workers/list) — list workers connected to the engine; same row shape as `directory::registry::workers::list`.
- [`directory::engine::workers::info`](iii://directory/engine/workers/info) — inspect one connected worker's full surface.

### `directory::registry::*` — what's published in the public registry

- [`directory::registry::workers::list`](iii://directory/registry/workers/list) — search published workers in `api.workers.iii.dev`; same row shape as `directory::engine::workers::list`.
- [`directory::registry::workers::info`](iii://directory/registry/workers/info) — full registry detail for one worker (envelope + readme + api_reference + skills_tree).
