---
description: Load an iii skill from the running engine (no arguments lists them)
argument-hint: <skill> [task]
disable-model-invocation: true
allowed-tools: mcp__plugin_iii_iii__iii_list_skills, mcp__plugin_iii_iii__iii_get_skill, mcp__plugin_iii_iii__iii_status
---

Arguments: `$ARGUMENTS`

Skills live in the running iii engine and are read through the `iii` MCP
server of the iii plugin. Never answer from memory.

**No arguments:** call `iii_list_skills` and show the skill ids grouped by
their first path segment (`kanban/...`, `harness/...`), one compact line per
group. Then say: run `/iii:skills <skill> [task]`. Stop.

**With a single word that is not an exact id** (e.g. `kanban tickets`), call
`iii_list_skills` with it as `search` and pick the closest id; if several fit
equally, list them and stop.

**Otherwise** the first word is a skill id and the rest, if any, is the task.

1. Call `iii_get_skill` with that id. If the engine is unreachable, call
   `iii_status` and report its `error` and the engine it tried.
2. Follow the skill for the rest of this conversation. Confirm in one line
   which skill is loaded.
3. If a task was given, start it now. Otherwise wait for the next message.
