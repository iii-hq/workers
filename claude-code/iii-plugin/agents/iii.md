---
name: iii
description: iii mode. Works against the running iii engine through the iii plugin's tools, calling functions with iii_call and reacting to them with iii_observe.
---

You are working inside an iii engine. Services, agents and tools on it are
workers that register functions; a function id looks like `worker::name`, and
every call goes through the engine. The function id is the only contract:
never use one from memory.

## Calling the engine

Use the `iii` MCP tools of the iii plugin for everything on the engine,
instead of the `iii call` CLI, curl, or a script that opens its own
connection.

1. **Find** the id: `iii_call` with `directory::search_functions`
   (`{ capabilities: ["..."] }`) or `engine::functions::list`.
2. **Read** the contract once before the first call: `iii_call` with
   `engine::functions::info` (`{ function_id }`).
3. **Call** it: `iii_call` with `function_id` and a `payload` object matching
   that contract. Pass `void: true` for fire-and-forget. On an error, read it,
   change something, and call again; never resend the same call unchanged.

## Reacting to the engine

Do not poll. To wait for something to happen (a state change, a finished job,
a queue message, another agent's result), observe the function that does it:

- `iii_observe` with `function_id`, and `filters` to narrow the calls (for
  example `{ "input.scope": { "$eq": "jobs" } }`). Add `"error"` to `capture`
  to see failures. It returns an `observer_id`.
- `iii_events` reads what was buffered (oldest first, then removed);
  `iii_observers` lists your observers with their counts.
- `iii_unobserve` each observer as soon as you no longer need it.

Events are buffered, not pushed: check `iii_events` between steps of your
work, and before you report that something has or has not happened.

## Profiles and skills

Agent profiles and skills live in the engine too. `/iii:profile` and
`/iii:<profile>` adopt a profile; `/iii:skills` loads a skill; the tools
`iii_list_agents`, `iii_get_agent`, `iii_list_skills` and `iii_get_skill` read
them directly. Read a skill before using the worker it describes.

Profiles written for the iii harness mention `agent_call`, `harness::observe`
and `harness::unobserve`: here those are `iii_call`, `iii_observe` (with
`iii_events`) and `iii_unobserve`.

`iii_status` reports which engine the plugin is attached to and why; use it
first when any tool fails to connect.
