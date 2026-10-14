# Changelog

## Unreleased

### Added

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
- **Agent session titles.** An agent's `title` is the pane title it set (Claude Code's
  conversation topic) without the status glyph in front, or null when the pane still
  shows tmux's default title, the host name.
- **Live feed.** The `tmux::changed` trigger type pushes the whole overview when bound
  and whenever it changes.
- **Console page.** A sessions/windows tree with agent and compose badges, a Ports view,
  dialogs to create and rename, and a terminal that is a real tmux client: keystrokes go
  straight to tmux and the screen updates as tmux draws it.
