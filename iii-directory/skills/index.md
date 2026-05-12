---
type: index
title: iii-directory
---

# iii-directory

Engine introspection, workers registry proxy, and filesystem-backed
skill + prompt reader for the [iii engine](https://github.com/iii-hq/iii).
Hosts four MCP-agnostic surfaces:

- **Skills** (`skills::*`, `skill::fetch`) — markdown documents under
  `iii://{id}` plus an `iii://skills` index. Use for "when and why to
  use my worker's tools".
- **Prompts** (`prompts::*`) — static prompt templates listed by
  `prompts::list` and read by `prompts::get`. Parametric command
  templates the *user* invokes.
- **Directory** (`directory::*`) — read-side enrichment over
  `engine::functions::list`, `engine::workers::list`,
  `engine::trigger-types::list`, and `engine::triggers::list`.
  "What's connected to the engine right now?"
- **Registry** (`registry::*`) — HTTP proxy over `api.workers.iii.dev`
  with the same row shape as `directory::*`. "What's published in the
  public registry?"

`directory::*` and `registry::*` share the same `worker-list` /
`worker-info` envelope shape, so callers can switch between the local
engine view and the published-registry view without re-learning the API.

Skills and prompts are sourced from a single configured folder on disk
(`skills_folder`); see [the README](../README.md) for the install,
configuration, and `skills::download` flow.

## How-tos

### `directory::*` — what's connected to the engine

- [`directory::function-list`](skills/directory/function-list.md) — list functions registered with the engine; filter by search/prefix/worker.
- [`directory::function-info`](skills/directory/function-info.md) — inspect one function's schemas, owner, and how-to skill.
- [`directory::trigger-list`](skills/directory/trigger-list.md) — list trigger types registered with the engine.
- [`directory::trigger-info`](skills/directory/trigger-info.md) — inspect one trigger type's schemas + live instance count.
- [`directory::registered-trigger-list`](skills/directory/registered-trigger-list.md) — list registered trigger instances (subscriber rows).
- [`directory::registered-trigger-info`](skills/directory/registered-trigger-info.md) — inspect one registered trigger (instance + type + function).
- [`directory::worker-list`](skills/directory/worker-list.md) — list workers connected to the engine; same row shape as `registry::worker-list`.
- [`directory::worker-info`](skills/directory/worker-info.md) — inspect one connected worker's full surface.

### `registry::*` — what's published in the public registry

- [`registry::worker-list`](skills/registry/worker-list.md) — search published workers in `api.workers.iii.dev`; same row shape as `directory::worker-list`.
- [`registry::worker-info`](skills/registry/worker-info.md) — full registry detail for one worker (envelope + readme + api_reference + skills_tree).
