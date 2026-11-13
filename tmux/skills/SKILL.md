---
name: tmux
description: >-
  See and manage the host's tmux sessions and windows from iii: which pane runs
  Claude Code, Codex or another agent CLI and whether it is working, waiting
  for the person or idle; which pane runs `iii compose` and on which ports its
  engine and workers listen; every listening TCP port traced to its pane; and
  create, rename, select or kill sessions and windows, move a window to
  another session, and split, restart or kill panes. Also how much of their
  weekly allowance Claude Code and Codex have used, and Claude Code on several
  accounts: start Claude Code on one, move a pane to another, change the
  global account.
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

It also reads how much of their weekly and 5-hour allowance Claude Code and
Codex have used, and keeps several Claude Code accounts, each a config folder
of its own (`CLAUDE_CONFIG_DIR`); `default` is `~/.claude`. The global
account is the one plain `claude` gets in a new tmux window (tmux's global
environment). A Claude Code pane follows the global account, is pinned to
one, or was started by hand (`manual`); `sessions::list` says which, per pane,
under `claude`.

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
- A person asks how much Claude Code or Codex usage is left this week, when
  it resets, or which account has the most room.
- A person wants Claude Code started on a given account, a running Claude
  Code moved to another account, or another account made global.

## Boundaries

- Read functions (`status`, `sessions::list`, `ports::list`,
  `install::status`, `usage::read`, `accounts::list`) are allowed by default. Creating, renaming, selecting,
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
- `usage::read` and `accounts::list` carry each account's email and
  organization but never a token. Everything
  else under `accounts::` and `panes::account` is approval-gated: a move ends
  Claude Code in that pane (SIGTERM) and starts it again on the other account,
  resuming its conversation, so name the panes that will restart; `global`
  moves every idle pane that follows the global account unless
  `move: new_only`. `rules` with `mode: switch` lets the worker change the
  global account by itself later. `remove` with `delete_files` deletes that
  account's folder, its sign-in included. `login_start` gives the person a
  sign-in link and `login_code` types the code they copied from it; never
  sign in for them, never ask for or repeat a code, and leave `browser` at
  `none` unless the person sits at the worker's computer.
- Starting Claude Code with `claude.skip_permissions: true` lets it run
  commands and change or delete files without asking. Only do it when the
  person asks for it, and say so before asking for approval.
- The worker reads Claude Code's sign-in only to ask its usage; it never
  refreshes, prints or returns a token.
- The worker never types into panes, except to move Claude Code started by
  hand in a shell: once that shell has the terminal back, it types the
  command that starts it on the other account into it (nothing otherwise).
  A move only goes to a signed-in account. `tmux::terminal::*` is the Console
  page's own terminal: it is denied to agents and cannot be observed
  (`NOT_OBSERVABLE`), since its calls carry the terminal's access key and
  what the person types. Its `upload` only writes a file the person pastes
  to a folder of its own under their cache folder (`~/.cache/iii/tmux/uploads`
  or `~/Library/Caches/iii/tmux/uploads`, kept 7 days) and answers the path;
  the page pastes that path, the worker types nothing.
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
- `tmux::sessions::create` — start a detached session with an optional name, directory, first window name and command; `remain_on_exit: true` keeps its pane, dead, once the command ends.
- `tmux::sessions::rename` / `tmux::sessions::kill` — rename or kill a session by id (`$5`) or exact name.
- `tmux::windows::create` — add a window to a session with an optional name, directory, command and `select`; `remain_on_exit: true` keeps its pane once the command ends.
- `tmux::windows::select` — make a window current in its session (every attached client follows).
- `tmux::windows::rename` / `tmux::windows::kill` — rename or kill a window by id (`@3`) or index.
- `tmux::windows::move` — move a window to another session (after its last window) or next to one of its windows (`before` / `after`, tmux 3.2 or later, which also reorder it inside its own session); it keeps its id and programs, and moving a session's last window closes that session (`source_session_closed`).
- `tmux::panes::select` — make a pane (`%5`) active; `zoom: true` zooms the window on it, `zoom: false` unzooms, no `zoom` keeps the window's zoom; it never toggles, and a one-pane window can't zoom (every client of that window follows).
- `tmux::panes::split` — split a pane `right` or `below`, in its current directory unless `cwd`, with the default shell unless `command`, active unless `select: false`; `remain_on_exit: true` keeps it once its program ends.
- `tmux::panes::kill` — kill a pane and its program; `window_closed` / `session_closed` say when that closed its window and session.
- `tmux::panes::respawn` — start a pane's program again (its own command unless `command`); a running one needs `kill: true`, a dead one (`remain_on_exit`) doesn't.
- `tmux::panes::capture` — a pane's visible screen with colour escapes, its size, cursor and alternate-screen flag.
- `tmux::subagents::transcript` — what one sub-agent of a pane's Claude Code or Codex has done (`pane`, `subagent` from `agent.subagents[].id`): its task, messages, reasoning, tool calls and results, oldest first.
- `tmux::ports::list` — listening TCP ports with process, iii role, worker, compose namespace and pane, plus each `iii compose` stack.
- `tmux::install` — install the tmux program on this computer in the background (package manager, else tmux's official build); answers at once, and starting during a run joins it.
- `tmux::usage::read` — Claude Code's accounts with each one's 5-hour window, week and per-model weeks (used %, reset time), Codex's plan, windows and credits, the global account, the rules and any suggestion; `refresh: true` reads again now (at most once a minute per account).
- `tmux::accounts::list` — the Claude Code accounts (name, colour, folder, email, plan, signed in, panes running on it, usage), the global one, the rules, the suggestion and the latest switch.
- `tmux::accounts::add` / `update` / `remove` — add an account (`name`, `color`, `shared` groups linked from `~/.claude`), rename, recolour or change what it shares, take it off the list (`delete_files` deletes its folder; never `default` or the global one).
- `tmux::accounts::login_start` / `login_code` / `login_cancel` — sign an account in: `login_start` runs `claude auth login` for it in a hidden terminal and answers the link (`url`) the person opens; `login_code` types the code the page showed them (`{state: "done", account}`, or `invalid_code`, `rejected`, `expired`, `gone`, `failed`); `browser: host` opens the worker computer's own browser, which signs in with no code. Sign-ins under way are in `accounts::list` as `logins`.
- `tmux::accounts::global` — make an account global; `move`: `idle_now_working_after` (default) or `new_only`. Answers the panes `moved`, `after_turn` and `stayed`.
- `tmux::panes::account` — `global` makes a pane follow the global account, an account id pins it; Claude Code moves there now when idle, after its turn otherwise (`after_turn`); the account it runs on cancels a waiting move.
- `tmux::accounts::move_idle` — move the idle Claude Code panes of account `from` to `to`, pinned there.
- `tmux::accounts::rules` / `tmux::accounts::dismiss` — what to do when the global account runs low (`off`, `suggest`, `switch`; threshold, order, switch back after the reset); "Not now" on a suggestion.
- `tmux::windows::create` / `tmux::sessions::create` with `claude: { account, skip_permissions }` instead of `command` — start Claude Code on `global` or an account; the answer's `runs` is the exact command.
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
