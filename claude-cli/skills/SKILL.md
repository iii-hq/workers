---
name: claude-cli
description: >-
  Claude Code in a terminal on the iii console. Use when an operator wants an
  interactive Claude Code session next to their engine, or wants Claude to
  build iii workers, functions, and triggers from a terminal. For headless
  Claude turns over the bus, use the claude-code worker instead.
---

# claude-cli

## What it is

A console page that always runs the Claude Code CLI. The worker installs the
CLI on the terminal host, equips a workspace (iii skills, engine notes,
activity hooks), and opens Claude in a `shell::pty` session. Its turns stream
onto `agent::events`, so the console renders them like any other agent
worker's.

## When to use it

- An operator wants to talk to Claude Code interactively, with login handled
  in the terminal.
- An operator wants an agent that can scaffold and register new iii workers
  from inside the engine.

Use `claude-code` instead for headless turns over the bus, and `pi-cli` for
the same terminal shape with the pi agent.

## What it produces

`agent::events`, grouped by the CLI's session id: a user message per prompt,
an assistant message per tool call, `function_execution_start`/`end` pairs
with durations, and `turn_end` + `agent_end` when Claude stops.

## Boundaries

- Every function here is console plumbing (`terminal::describe`, `activity`,
  `ui-content`), flagged internal. Do not call them; a terminal is opened by a
  person, from the console page.
- The command is fixed to Claude Code. Use the `shell` worker for anything
  else — including `shell::pty::sessions` to see what a terminal is doing.
- The workspace must be reachable by the `shell` worker: it owns the session.
