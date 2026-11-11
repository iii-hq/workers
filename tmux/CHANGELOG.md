# Changelog

## Unreleased

### Fixed

- **Names are taken as written.** tmux 2.7 and later expand formats in session and window
  names, so a `#` in a name given to `tmux::sessions::create`, `tmux::windows::create` or a
  rename came out changed, and `#(…)` ran that command on the tmux server; `#` is now doubled
  where (and only on the tmux versions where) it would be expanded.
- **A session named like an id is never taken for another.** Names shaped like an id (`$2`,
  `@3`, `%4`) are refused, and a call naming `$2` is refused in one sentence when `$2` is one
  session's id and another's name (made outside this worker) instead of picking either.
- **Refusals are one sentence.** tmux's `can't find pane: %9` (`can't find pane %9` before
  3.0) and the like read `There is no pane %9.`, and a socket path stays out of the answer. A
  missing pane is caught on tmux 3.4 and later too, where `display-message` answers for a
  target it can't find instead of failing.
- **Zooming works on tmux before 3.1**, which has no `select-pane -Z`, and a zoom another
  client changed between the worker's read and its toggle is set again.
- **Start directories are taken as written.** tmux expands formats in a start directory, so a
  `cwd` with `#` in it opened `tmux::sessions::create` or `tmux::windows::create` somewhere
  else, and one with `#(…)` ran that command on the tmux server.
- **Kill session names its windows again.** The Console's confirm dialog lost its
  `details` list in the ADE redesign (#1528), which also broke the page's type-check and
  with it the worker's build. The Kill session confirmation now names the windows that
  end in its sentence, each with its agent, `iii compose` or command, past six as `N more`
  (`0:editor (vim), 1:dev (npm) and 2:logs (tail) end with every process in them…`).
- **A worker without a UTF-8 locale lists its sessions again.** With no `LANG` (a daemon
  started by systemd or launchd, a container) tmux printed the listing separator as `_`,
  so `tmux::sessions::list` came back empty and `tmux::sessions::create` failed. Every tmux
  command now passes `-u`.

### Changed

- **The Console page follows the ADE redesign (#1528).** Interface text is set in Inter
  and code, ports and the terminal in Geist Mono, as on the ide page; the sessions pane
  sits on the canvas surface (in the light theme it showed the page's grey hairline
  colour, and a narrow pane's drawer was see-through); the session tree takes the
  sidebar's rhythm (36px rounded rows, 14px labels). A sub-agent's own messages no
  longer indent their whole step, and the Ports tables centre each row on its tmux
  location.
- **iii-sdk 0.26.0.** The worker builds against the observe v2 SDK and invokes functions
  with `call`. Console terminal calls (`tmux::terminal::*`) take their caller from the
  invocation context the engine stamps (`caller.worker_id`); the payload's
  `_caller_worker_id` is read only on an engine without observe (0.24). A call from an
  observer activation is refused.
- **tmux is also found outside `PATH`.** Plain `tmux` is looked for in `~/.local/bin`,
  `/opt/homebrew/bin`, `/usr/local/bin`, `/home/linuxbrew/.linuxbrew/bin` and
  `/opt/local/bin` after `PATH`.

### Added

- **Claude Code and Codex usage.** `tmux::usage::read` and the live feed's new `usage`
  field give each Claude Code account's 5-hour window, week and per-model weeks with their
  reset times (from the endpoint Claude Code reads its own usage from, with the account's
  sign-in, read-only: no token is ever refreshed, logged or returned) and Codex's plan,
  windows and credits (from its newest session record). Readings stay at most 5 minutes old
  while someone watches; `refresh: true` reads again, at most once a minute per account.
  With nobody watching, it reads only when another signed-in account could take over.
- **Claude Code accounts.** `tmux::accounts::list`, `add`, `login`, `update` and `remove`
  keep several Claude Code accounts, each a config folder of its own under
  `~/.local/share/iii/claude-accounts` that shares settings, extensions, plugins and
  conversations with `~/.claude` through links, as chosen. `login` opens a `login` window
  running `claude auth login` for the account. `default` stays `~/.claude`.
- **A global account.** `tmux::accounts::global` puts a signed-in account in tmux's global
  environment (`CLAUDE_CONFIG_DIR`), so plain `claude` in a new window runs on it, and moves
  the panes that follow it: idle ones at once, working ones once their turn ends, or none
  (`new_only`). Once `accounts.json` exists the worker keeps that variable in step; without
  it the worker never touches it.
- **Claude Code on an account from create.** `tmux::windows::create` and
  `tmux::sessions::create` take `claude: { account, skip_permissions }` instead of
  `command`, build the command (`claude`, `env -u CLAUDE_CONFIG_DIR claude`, `env
  CLAUDE_CONFIG_DIR=<folder> claude`, plus `--dangerously-skip-permissions` when asked,
  never as root), mark the pane and answer the command as `runs`. Each Claude Code pane in
  `tmux::sessions::list` has a `claude` object: its account, whether it follows the global
  one, is pinned or was started by hand, a waiting move, permission prompts skipped, its
  session and whether it can move.
- **Moving a pane between accounts.** `tmux::panes::account` makes a pane follow the global
  account or pins it to one; Claude Code then stops (SIGTERM, up to 10 seconds) and starts
  again on that account with `--resume`, its conversation copied there when the accounts
  don't share conversations (never over a longer copy). It moves only when its status is
  `idle`, looked at again right before it stops, and only to a signed-in account; a working
  or waiting pane moves once its turn ends, and a move that fails says why (`move_error`).
  `tmux::accounts::move_idle` moves every idle pane of one account to another.
- **Rules for when the global account runs low.** `tmux::accounts::rules` sets a threshold
  (week 80 %, 90 % or the limit, and optionally a full 5-hour window) and whether the worker
  suggests another account (`suggestion` in the feed, `tmux::accounts::dismiss` for "Not
  now") or switches by itself, even with the app closed (`last_switch`), and switches back
  after the reset if asked.
- **A demo kit** (`demo/`): the worker in a container with its own tmux, a fake Claude
  Code and Codex, fake accounts and a mock usage endpoint, for the apps' screenshots and
  tests. Its scripts refuse to run outside that container.
- **Pasting a file into a Console terminal.** `tmux::terminal::upload` saves any file (at
  most 12 MiB) in a new folder of its own under the user's cache folder
  (`~/.cache/iii/tmux/uploads`, `~/Library/Caches/iii/tmux/uploads`), under its name made one
  safe token, and answers its path, which the page pastes; it types nothing. A PNG, JPEG, GIF
  or WebP gets the extension its content has. Only this user can read it; folders on the way
  are never followed through a symbolic link, and a file is linked under its name only once
  written whole. Uploads are kept 7 days and 256 MiB at most. Like the other terminal
  functions it needs the terminal's key and cannot be observed.
- **Keeping a pane whose program ended.** `remain_on_exit: true` on `tmux::sessions::create`,
  `tmux::windows::create` and `tmux::panes::split` keeps the new pane, dead, once its program
  ends (even at once), for `tmux::panes::respawn` to start again.
- **Splitting, killing and restarting panes.** `tmux::panes::split` puts a new pane to the
  right of a pane or below it, in that pane's current directory unless `cwd`, running
  `command` or the default shell. `tmux::panes::kill` kills a pane and says whether its window
  and session closed with it. `tmux::panes::respawn` starts a pane's program again; one still
  running needs `kill: true`.
- **Moving a window to another session.** `tmux::windows::move` moves a window, with its id,
  panes and programs, after another session's last window, or just before or after one of its
  windows (`before` / `after`, which also reorder a window inside its own session; tmux 3.2 or
  later, an older tmux gets one sentence). The destination's current window stays as it
  was. When the window was its session's last, tmux closes that session and the answer's
  `source_session_closed` says so.
- **Installing tmux from the worker.** `tmux::install` puts the tmux program on the
  computer in the background and `tmux::install::status` says how it is going: the
  system's package manager when this user may run it without a password, otherwise tmux's
  official build (3.7c, pinned by SHA-256) in `~/.local/bin`. `tmux::status` gains
  `installable`. `--install-tmux` runs the same install with no engine.
- **tmux sessions and windows as functions.** `tmux::sessions::{list,create,rename,kill}`
  and `tmux::windows::{create,select,rename,kill}` drive the host's tmux server; sessions
  and windows are addressed by id (`$5`, `@3`) or exact name / index, never by prefix.
- **What runs in each pane.** `tmux::sessions::list` reports the agent CLI in a pane
  (Claude Code, Codex, OpenCode, Gemini CLI, or your own matchers) with a working /
  waiting / idle state, the `iii compose` daemon a pane runs, and the ports the pane's
  processes listen on.
- **Listening ports.** `tmux::ports::list` lists every TCP listener of your processes
  with its iii role (engine, worker, compose), worker name, compose namespace and tmux
  pane, plus each running `iii compose` stack with its engine WebSocket port and its
  workers' ports. Linux reads `/proc`; macOS uses `lsof`.
- **Panes.** `tmux::panes::select` makes a pane active and zooms the window on it
  (`zoom: true`), unzooms it (`zoom: false`) or keeps the window's zoom as it moves;
  `tmux::panes::capture` returns a pane's visible screen with its colour escapes, size,
  cursor and alternate-screen flag.
- **Working agents in narrow panes.** Claude Code and Codex also count as working while
  their running clock shows (`✻ Harmonizing… (16m 46s`, `Working (93s`), since a narrow
  pane cuts the `esc to interrupt` tail off the status line (for example when a phone is
  attached to the session). Stored configurations keep the patterns they were saved with.
- **Sub-agents.** Claude Code and Codex stay `working` while sub-agents run after the
  main turn ended (their screen shows no `esc to interrupt` then), and each agent lists
  the sub-agents at work in `subagents` (`id`, `description`, `kind`, `depth`,
  `since_ms`). Claude Code's state comes from its session file
  (`~/.claude/sessions/<pid>.json`, or under `CLAUDE_CONFIG_DIR`) and its sub-agents'
  meta files and transcripts; Codex's from the rollouts its process holds open. A
  sub-agent leaves the list once it finishes or is stopped. The Console page shows the
  count on the agent's badge and a row per sub-agent under its window. A prompt on
  screen still makes the agent `waiting`.
- **Sub-agent transcripts.** A sub-agent has no pane of its own, so
  `tmux::subagents::transcript` (`pane`, `subagent`) returns what it has done so far:
  the task it was given, what it wrote and reasoned, the tools it called and what they
  returned, oldest first (the latest 200, or `limit` up to 1000). It reads Claude Code's
  sub-agent transcript or the thread's Codex rollout, while the sub-agent runs and after
  it finished, and each call reads only what was added since the last. Selecting a
  sub-agent in the Console page's tree opens its transcript in a Sub-agent tab, which
  follows new steps every 2 s.
- **Agent session titles.** An agent's `title` is the pane title it set (Claude Code's
  conversation topic) without the status glyph in front, or null when the pane still
  shows tmux's default title, the host name.
- **Live feed.** `tmux::on-changed`, a private function observed with `iii.observe`,
  sends the whole overview to each new observer and is called again whenever it changes.
- **Console page.** A sessions/windows tree with agent and compose badges, a Ports view,
  dialogs to create and rename, and a terminal that is a real tmux client: keystrokes go
  straight to tmux and the screen updates as tmux draws it.
