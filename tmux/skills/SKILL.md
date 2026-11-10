---
name: tmux
description: >-
  See and manage the host's tmux sessions and windows from iii: which pane runs
  Claude Code, Codex or another agent CLI and whether it is working, waiting
  for the person or idle; which pane runs `iii compose` and on which ports its
  engine and workers listen; every listening TCP port traced to its pane; and
  create, rename, select or kill sessions and windows, move a window to
  another session, and split, restart or kill panes.
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
  in a given directory, or wants one renamed, moved to another session or
  closed.

## Boundaries

- Read functions (`status`, `sessions::list`, `ports::list`,
  `install::status`) are allowed by default. Creating, renaming, selecting,
  moving, splitting, respawning and killing change the person's tmux and are
  approval-gated. Killing a session, window or pane, and `panes::respawn` with
  `kill: true`, end whatever runs there, so name what will stop before asking.
  Moving a session's last window closes that session, and the person's
  terminals attached to it end, so say so too. `panes::select` is
  approval-gated too: a zoom shows on every client looking at that window.
- `panes::capture` returns whatever is on a pane's screen, which can include
  anything the person typed or a program printed; it needs approval and its
  text should not be repeated beyond what the question needs.
- `subagents::transcript` returns a sub-agent's conversation, tool calls and
  their output; it needs approval for the same reason.
- The worker never types into panes. `tmux::terminal::*` is the Console
  page's own terminal: it is denied to agents and cannot be observed
  (`NOT_OBSERVABLE`), since its calls carry the terminal's access key and
  what the person types. Its `upload-image` only writes an image the person
  pastes to a file under their cache folder (`~/.cache/iii/tmux/paste` or
  `~/Library/Caches/iii/tmux/paste`, kept 7 days) and answers the path; the
  page pastes that path, the worker types nothing.
- Agent state is a reading of the screen: `waiting` means a prompt that needs
  the person is visible, not that the agent's process is blocked. Patterns
  live in the worker configuration and can be tuned per agent.
- Ports are TCP listeners of this user's processes only, and tmux locations
  only exist for processes started inside tmux.
- tmux is Unix-only; on a host without tmux, `status` reports
  `available: false` and the lists come back empty. `installable: true` there
  means `install` can try.
- `install` puts the tmux program on the computer and can run the system's
  package manager as root, so it needs approval; say so before asking. It never
  waits for a password: without root or passwordless sudo it uses tmux's
  official build in `~/.local/bin`. It runs in the background, and it never
  talks to the person's tmux server. Only install tmux when the person asks.

## Functions

- `tmux::status` — whether tmux is installed, its version and path, and whether a server is running.
- `tmux::sessions::list` — every session → window → pane with command, cwd, title, size, agent state and session title, compose daemon and ports.
- `tmux::sessions::create` — start a detached session with an optional name, directory, first window name and command.
- `tmux::sessions::rename` / `tmux::sessions::kill` — rename or kill a session by id (`$5`) or exact name.
- `tmux::windows::create` — add a window to a session with an optional name, directory, command and `select`.
- `tmux::windows::select` — make a window current in its session (every attached client follows).
- `tmux::windows::rename` / `tmux::windows::kill` — rename or kill a window by id (`@3`) or index.
- `tmux::windows::move` — move a window to another session (after its last window) or next to one of its windows (`before` / `after`, which also reorder it inside its own session); it keeps its id and programs, and moving a session's last window closes that session (`source_session_closed`).
- `tmux::panes::select` — make a pane (`%5`) active; `zoom: true` zooms the window on it, `zoom: false` unzooms, no `zoom` keeps the window's zoom; it never toggles, and a one-pane window can't zoom (every client of that window follows).
- `tmux::panes::split` — split a pane `right` or `below`, in its current directory unless `cwd`, with the default shell unless `command`, active unless `select: false`.
- `tmux::panes::kill` — kill a pane and its program; `window_closed` / `session_closed` say when that closed its window and session.
- `tmux::panes::respawn` — start a pane's program again (its own command unless `command`); a running one needs `kill: true`.
- `tmux::panes::capture` — a pane's visible screen with colour escapes, its size, cursor and alternate-screen flag.
- `tmux::subagents::transcript` — what one sub-agent of a pane's Claude Code or Codex has done (`pane`, `subagent` from `agent.subagents[].id`): its task, messages, reasoning, tool calls and results, oldest first.
- `tmux::ports::list` — listening TCP ports with process, iii role, worker, compose namespace and pane, plus each `iii compose` stack.
- `tmux::install` — install the tmux program on this computer in the background (package manager, else tmux's official build); answers at once, and starting during a run joins it.
- `tmux::install::status` — where that install is: `idle`, `running` with its step and words, `succeeded` with path and version, or `failed` with one sentence and the log.

## Reacting to tmux

Observe `tmux::on-changed` instead of polling `sessions::list` when something
must react to tmux: an agent that finishes a turn (`working` → `idle`), one
that starts waiting for the person, a compose stack that comes up, a session
that appears or goes away. Each observer receives the full overview
(`sessions` and `ports`, the same shapes the two list functions return) once
when it is added and then only when something changed; the worker polls tmux
only while the function is observed. It takes no args. Overviews can arrive
out of order, so keep the one with the highest `sessions.generated_at_ms`.

Register a handler, then observe the function with it:

```ts
iii.registerFunction('my-worker::on-tmux', async ({ output: { sessions, ports } }) => {
  // compare agent states with the previous overview
})
iii.observe({ function_id: 'tmux::on-changed', capture: ['output'] }, 'my-worker::on-tmux')
```
