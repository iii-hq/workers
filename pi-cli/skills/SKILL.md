---
name: pi-cli
description: >-
  The pi coding agent in a terminal on the iii console. Use when an operator
  wants an interactive pi session next to their engine, or wants pi to build
  iii workers, functions, and triggers from a terminal. For headless pi turns
  over the bus, use the pi worker instead.
---

# pi-cli

## What it is

A console page that always runs the `pi` CLI (pi.dev). The worker installs the
CLI on the terminal host, equips a workspace (iii skills, engine notes, and
the iii activity extension), and opens pi in a `shell::pty` session. Its runs
stream onto `agent::events`, so the console renders them like any other agent
worker's.

## When to use it

- An operator wants pi interactively, with login handled in the terminal.
- An operator wants a second agent harness on the same engine to compare
  against Claude Code.

Use `pi` instead for headless turns over the bus, and `claude-cli` for the
same terminal shape with Claude Code.

## What it produces

`agent::events`, grouped by session: a user message per prompt, an assistant
message per tool call, `function_execution_start`/`end` pairs with durations,
and `turn_end` + `agent_end` when the run finishes.

## Boundaries

- Every function here is console plumbing (`terminal::describe`, `activity`,
  `ui-content`), flagged internal. Do not call them; a terminal is opened by a
  person, from the console page.
- The command is fixed to pi. Use the `shell` worker for anything else —
  including `shell::pty::sessions` to see what a terminal is doing.
- pi loads its project-local extension only in a trusted directory, which is
  why sessions run with `-a`. Removing that flag means answering a trust
  prompt every session and losing the activity stream.
- `pi-cli::auth::status` reports which plan a session spends for the configured
  provider (subscription login vs API key). The page shows it in the status
  bar; agents are denied it.
