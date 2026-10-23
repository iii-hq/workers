---
name: tmux
description: >-
  See and manage the host's tmux sessions and windows from iii: which pane runs
  Claude Code, Codex or another agent CLI and whether it is working, waiting
  for the person or idle; which pane runs `iii compose` and on which ports its
  engine and workers listen; every listening TCP port traced to its pane; and
  create, rename, select or kill sessions and windows.
---

# tmux

The tmux worker drives the `tmux` binary on the host and reads the same tmux
server the person uses in their terminal. It reports every session, window
and pane with the foreground command, working directory and title, and adds
what tmux itself does not know: the agent CLI running in a pane (matched in
the pane's process tree) with a working / waiting / idle state read from the
pane's last lines, the `iii compose` daemon running in a pane with its
engine's WebSocket port and its workers' ports, and every TCP port this
user's processes listen on, each traced back to the pane that started it.

Changes are tmux's own semantics. Selecting a window switches every client
attached to that session, the person's terminal included. Creating a window
makes it current unless `select: false` is passed. Killing a session or
window kills the programs in it. The Console page's terminal is a real tmux
client that only the Console drives.

## When to Use

- A person asks which Claude Code or Codex sessions are running, which of
  them is busy, or which one is waiting for an answer or an approval.
- A person asks whether `iii compose` is up, on which port the engine
  listens, or which ports the http, ade or other workers use.
- A person asks what is listening on a port, or which tmux window started a
  server.
- A person wants a new tmux session or window, optionally running a command
  in a given directory, or wants one renamed or closed.

## Boundaries

- Read functions (`status`, `sessions::list`, `ports::list`) are allowed by
  default. Creating, renaming, selecting and killing change the person's
  tmux and are approval-gated; killing ends whatever runs there, so name
  what will stop before asking. `panes::select` is approval-gated too: a
  zoom shows on every client looking at that window.
- `panes::capture` returns whatever is on a pane's screen, which can include
  anything the person typed or a program printed; it needs approval and its
  text should not be repeated beyond what the question needs.
- `subagents::transcript` returns a sub-agent's conversation, tool calls and
  their output; it needs approval for the same reason.
- The worker never types into panes. `tmux::terminal::*` is the Console
  page's own terminal and is denied to agents.
- Agent state is a reading of the screen: `waiting` means a prompt that needs
  the person is visible, not that the agent's process is blocked. Patterns
  live in the worker configuration and can be tuned per agent.
- Ports are TCP listeners of this user's processes only, and tmux locations
  only exist for processes started inside tmux.
- tmux is Unix-only; on a host without tmux, `status` reports
  `available: false` and the lists come back empty.

## Functions

- `tmux::status` — whether tmux is installed, its version and path, and whether a server is running.
- `tmux::sessions::list` — every session → window → pane with command, cwd, title, size, agent state and session title, compose daemon and ports.
- `tmux::sessions::create` — start a detached session with an optional name, directory, first window name and command.
- `tmux::sessions::rename` / `tmux::sessions::kill` — rename or kill a session by id (`$5`) or exact name.
- `tmux::windows::create` — add a window to a session with an optional name, directory, command and `select`.
- `tmux::windows::select` — make a window current in its session (every attached client follows).
- `tmux::windows::rename` / `tmux::windows::kill` — rename or kill a window by id (`@3`) or index.
- `tmux::panes::select` — make a pane (`%5`) active; `zoom: true` zooms the window on it, `zoom: false` unzooms (every client of that window follows).
- `tmux::panes::capture` — a pane's visible screen with colour escapes, its size, cursor and alternate-screen flag.
- `tmux::subagents::transcript` — what one sub-agent of a pane's Claude Code or Codex has done (`pane`, `subagent` from `agent.subagents[].id`): its task, messages, reasoning, tool calls and results, oldest first.
- `tmux::ports::list` — listening TCP ports with process, iii role, worker, compose namespace and pane, plus each `iii compose` stack.

## Reactive triggers

Bind `tmux::changed` instead of polling `sessions::list` when something must
react to tmux: an agent that finishes a turn (`working` → `idle`), one that
starts waiting for the person, a compose stack that comes up, a session that
appears or goes away. It fires the full overview (`sessions` and `ports`,
the same shapes the two list functions return) once when bound and then only
when something changed; the worker polls tmux only while a binding exists.

Register a handler, then bind it with an empty config:

```ts
iii.registerFunction('my-worker::on-tmux', async ({ sessions, ports }) => {
  // compare agent states with the previous overview
})
iii.registerTrigger({ type: 'tmux::changed', function_id: 'my-worker::on-tmux', config: {} })
```
