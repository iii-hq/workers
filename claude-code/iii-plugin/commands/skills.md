---
description: Load an iii skill from the running engine (no arguments lists them)
argument-hint: <skill> [task]
disable-model-invocation: true
allowed-tools: mcp__plugin_iii_iii__iii_list_skills, mcp__plugin_iii_iii__iii_get_skill, mcp__plugin_iii_iii__iii_status
---

Arguments: `$ARGUMENTS`

Skills live in the running iii engine and are read through the iii plugin's
`iii` MCP server (the engine's `mcp-server` worker). Never answer from
memory.

**No arguments:** call `iii_list_skills` and show the skill ids grouped by
their first path segment (`kanban/...`, `harness/...`), one compact line per
group. Then say: run `/iii:skills <skill> [task]`. Stop.

**With a single word that is not an exact id** (e.g. `kanban tickets`), call
`iii_list_skills` with it as `search` and pick the closest id; if several fit
equally, list them and stop.

**Otherwise** the first word is a skill id and the rest, if any, is the task.

1. Call `iii_get_skill` with that id.
2. Follow the skill for the rest of this conversation. Confirm in one line
   which skill is loaded.
3. If a task was given, start it now. Otherwise wait for the next message.

If the `iii` MCP tools are missing or cannot connect, stop and say so: the
iii plugin reaches the engine through the `mcp-server` worker, and none
answers at the plugin's `mcp_url` option (default
`http://127.0.0.1:3111/mcp`). Run `mcp-server` on the engine, behind
rbac-proxy (see its README), or set `mcp_url` to where it runs.

While you work, act on the engine with `iii_call` (any function id, with a
payload) instead of the `iii` CLI, and react to it with `iii_observe` (then
`iii_observations_poll`, and `iii_unobserve` when done) instead of polling.
Where a profile or skill says `agent_call` or `harness::observe`, use
`iii_call` and `iii_observe`.
