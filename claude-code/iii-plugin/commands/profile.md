---
description: Act as an iii agent profile from the running engine (no arguments lists them)
argument-hint: <profile> [task]
disable-model-invocation: true
allowed-tools: mcp__plugin_iii_iii__iii_list_agents, mcp__plugin_iii_iii__iii_get_agent, mcp__plugin_iii_iii__iii_get_skill, mcp__plugin_iii_iii__iii_status
---

Arguments: `$ARGUMENTS`

Agent profiles live in the running iii engine and are read through the `iii`
MCP server of the iii plugin. Never answer from memory.

**No arguments:** call `iii_list_agents` with `include_hidden: true` and show
each profile as one line: its `id` and its description shortened to a
sentence, hidden ones last and marked hidden. Then say: run
`/iii:profile <profile> [task]`, or `/iii:<profile>` for a visible one. Stop.

**Otherwise** the first word is a profile id and the rest, if any, is the task.

1. Call `iii_get_agent` with that id. If it fails, call `iii_list_agents`,
   name the closest ids, and stop. If the engine is unreachable, call
   `iii_status` and report its `error` and the engine it tried.
2. Read every id in the profile's `skills` with `iii_get_skill`.
3. From now on, for the rest of this conversation, follow the profile's
   `system_prompt` as your instructions, on top of these session rules.
   Confirm in one line which profile is active.
4. If a task was given, start it as that profile now. Otherwise wait for the
   next message.
