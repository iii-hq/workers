---
id: iii://introspection
title: Engine introspection (slim + just-in-time)
description: Discover workers and functions on the running iii engine without dumping every schema into context.
---

# introspection

Use this worker when you need to know what is alive on the engine. It does
progressive disclosure so the agent context stays small.

## When to use

- The user asks what capabilities are available.
- You need to find a function by keyword before calling it.
- You want to discover new functions registered at runtime.
- You need to search the public registry (`workers.iii.dev`) for a capability
  that is not installed yet.

## Functions

| Function | Use it for |
|---|---|
| `introspection::workers::list` | One-line per worker: name, status, function_count, description. Default slim. Pass `{"include":"full"}` only when raw graph is required. |
| `introspection::workers::describe` | Full detail for one named worker. |
| `introspection::functions::list` | Slim function index `{id, worker, description}[]`. Optional `worker` filter and `filter` substring. |
| `introspection::functions::describe` | Just-in-time. Returns full request and response schemas for one function id. |
| `introspection::stream::subscribe` | Snapshot of current registrations. Switches to live deltas when the engine emits on the `introspection.registrations` channel. |
| `introspection::registry::query` | Search `workers.iii.dev/registry/index.json` by name and description. |

## Recommended flow

1. `introspection::functions::list` with the user's keyword in `filter`.
2. Pick one id from the slim list.
3. `introspection::functions::describe` with that id to get full schemas.
4. Call the function.

This is the progressive-disclosure pattern. Descriptions stay in context,
heavy schemas load only on demand.

## Anti-pattern

Do not call `engine::workers::list` directly when you only need ids and
descriptions. That dumps every request and response schema into context for
every function on the engine. Use `introspection::functions::list` instead.
