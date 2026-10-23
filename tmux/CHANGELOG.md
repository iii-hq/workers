# Changelog

## Unreleased

### Changed

- **iii-sdk 0.25.0.** The worker builds against the 0.25 SDK. Console terminal calls
  (`tmux::terminal::*`) take their caller from the invocation context the engine stamps
  (`caller.worker_id`); the payload's `_caller_worker_id` is read only on an engine
  without observe (0.24). A call from an observer activation is refused.
  `tmux::changed` is still a trigger type, registered the 0.24 way, so existing
  consumers see no change on either engine.

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
- **Live feed.** The `tmux::changed` trigger type pushes the whole overview when bound
  and whenever it changes.
- **Console page.** A sessions/windows tree with agent and compose badges, a Ports view,
  dialogs to create and rename, and a terminal that is a real tmux client: keystrokes go
  straight to tmux and the screen updates as tmux draws it.
