# tmux

tmux as an iii worker. It lists every tmux session, window and pane on the
host, creates, renames and kills sessions and windows, moves windows between
sessions, and tells you what is running where: which pane holds Claude Code
or Codex and whether that agent is working, waiting for you or idle, which
pane runs `iii compose` and on which port its engine listens, and every TCP
port your processes listen on, traced back to the pane that started them. Its
Console page embeds a terminal that is a real tmux client: what you type goes
straight into the session, and the screen updates as tmux draws it, the same
way a second terminal attached with `tmux attach` would.

## Install

```bash
iii call compose::add worker=tmux
```

The worker drives the `tmux` binary installed on the host (Linux or macOS) and
talks to the same tmux server you use from your terminal. Without tmux it still
starts and reports `available: false`, and `tmux::install` can put it there.

## Installing tmux

```bash
iii call tmux::install               # starts or joins the install, answers at once
iii call tmux::install::status       # poll until succeeded or failed
```

| Computer | What runs, in order |
|---|---|
| macOS | Homebrew when `brew` is there (never as root), otherwise MacPorts as root or with `sudo -n`; then the official build |
| Linux, as root or with `sudo -n` | The first of `apt-get` (after `apt-get update`), `dnf`, `pacman` (never `-Sy`, so a stale or empty database falls through), `zypper`, `apk`; then the official build |
| Linux, no root and no passwordless sudo | The official build |
| Linux on a processor with no official build (armv7) | Only a package manager, and without root or sudo the error says what to ask an administrator to run |

Nothing ever waits for a password or for an answer. The official build is
[`tmux/tmux-builds`](https://github.com/tmux/tmux-builds) 3.7c for Linux and
macOS on x86_64 and arm64, pinned by SHA-256 in this worker's source. It goes
to `~/.local/bin/tmux`: checked before it is unpacked, asked `-V` before it
lands there, moved atomically. A file already at that path is never replaced.

An attempt only counts when tmux then resolves, answers `-V` and starts a
server on a private socket of its own, and one that reports a failure counts
when tmux works anyway. A package manager that fails falls through to the
official build, and the failure's `log` says so. With tmux already there the
install changes nothing: it makes the same check and answers `succeeded` with
`method: null`. An install never talks to your tmux server.

The answer of both functions:

```json
{ "state": "running", "step": "downloading", "label": "Downloading tmux…",
  "hints": ["From the tmux project's official builds", "tmux 3.7c"],
  "progress": 0.42, "method": "prebuilt", "path": null, "version": null,
  "error": null, "log": [] }
```

`state` is `idle`, `running`, `succeeded` or `failed`; `step` is `checking`,
`updating`, `downloading`, `verifying`, `installing` or `finishing`; `method` is
`apt`, `dnf`, `pacman`, `zypper`, `apk`, `homebrew`, `macports` or `prebuilt`.
`label` and `hints` are the words to show while it runs, `error` is one
sentence, and `log` is empty until a failure, when it holds every command and
its output. A worker that restarted mid-install answers `idle`.

`tmux::status` says `installable: true` when tmux is missing and this can try:
Linux or macOS, with `tmux_binary` left as plain `tmux`, on a processor with an
official build or a computer with one of those package managers.

A daemon's `PATH` is often short, so plain `tmux` is also looked for after
`PATH` in `~/.local/bin`, `/opt/homebrew/bin`, `/usr/local/bin`,
`/home/linuxbrew/.linuxbrew/bin` and `/opt/local/bin`.

On a system with no terminfo entry for `xterm-256color` (Alpine without
`ncurses-terminfo-base`), the official build can run every command but can't
attach a terminal, so the install also writes that one entry to
`~/.terminfo/x/xterm-256color`.

`tmux --install-tmux` runs the same install with no engine and exits: the steps
and the log on stderr, the final status as the last line of stdout.

## Quickstart

Open the **tmux** page in the Console to browse sessions, open one in the
terminal, and see the **Ports** tab. From code or an agent:

```bash
# Every session → window → pane, with agent / compose / port status
iii call tmux::sessions::list

# A new session running a dev server, then a second window in it
iii call tmux::sessions::create name=api cwd=~/code/api command="pnpm dev"
iii call tmux::windows::create session=api name=logs

# Who listens on which port, and which compose stacks are up
iii call tmux::ports::list
```

A pane running an agent and a pane running `iii compose` look like this in
`tmux::sessions::list`:

```json
{ "id": "%3", "command": "claude",
  "agent": { "id": "claude", "label": "Claude Code", "state": "waiting", "since_ms": 1791936000000,
             "title": "Fix the login redirect" } }
{ "id": "%7", "command": "iii:c:default",
  "compose": { "namespace": "default", "pid": 2759592 },
  "ports": [ { "port": 49135, "role": "engine", "process": "iii:e:default" },
             { "port": 3111, "role": "worker", "worker": "http" },
             { "port": 3114, "role": "worker", "worker": "ade" } ] }
```

Agent state comes from the pane's last lines: a permission or choice prompt is
`waiting`, Claude Code's `esc to interrupt` line is `working`. Claude Code and
Codex also count as `working` while their sub-agents run after the main turn
ended, which their screen does not show; the worker reads what they record on
disk (Claude Code's `~/.claude/sessions/<pid>.json` and each sub-agent's meta
file and transcript, the `rollout-*.jsonl` files Codex holds open) and lists
those sub-agents in `subagents`:

```json
{ "id": "%9", "command": "codex",
  "agent": { "id": "codex", "label": "Codex", "state": "working", "since_ms": 1792698537000,
             "title": null,
             "subagents": [ { "id": "01a16528-d646", "description": "review/tests", "kind": null,
                              "depth": 1, "since_ms": 1792698537000 } ] } }
```

The Console page shows the count on the agent's badge and a row per sub-agent
under its window. A sub-agent has no pane of its own: selecting its row opens
its transcript, which `tmux::subagents::transcript` returns from the same
files. That is the task it was given, what it wrote and reasoned, and the
tools it called with what they returned, oldest first, while it runs and
after it finished:

```bash
iii call tmux::subagents::transcript pane=%9 subagent=01a16528-d646
```

```json
{ "pane_id": "%9", "agent": "codex", "subagent_id": "01a16528-d646", "working": true,
  "prompt": "Message Type: NEW_TASK\nTask name: /root/review/tests\n…",
  "items": [ { "kind": "tool_call", "at_ms": 1792698540000, "text": "cargo test",
               "tool": "shell", "error": false },
             { "kind": "tool_result", "at_ms": 1792698551000, "text": "test result: ok…",
               "tool": "shell", "error": false } ],
  "truncated": false }
```

Item kinds are `user` (a later message from the main agent), `text`,
`thinking` (when the agent records it in clear), `tool_call` and
`tool_result`. Each text is cut at 4 000 characters, and only the last 4 MiB
of a long transcript are read, plus its task. Codex keeps a sub-agent's task
encrypted, so there the prompt shows only its name and sender.

The agent's
`title` is the session title it put on its pane (Claude Code's conversation
topic) without the ✳ or spinner in front; it is null while the pane shows
tmux's default title, the host name. Selecting a window
(`tmux::windows::select`) works like tmux itself: every client attached to
that session, your own terminal included, switches to it.

`tmux::windows::move` moves a window to another session, or next to one of
its windows. The window keeps its id, panes and programs:

```bash
# After the last window of `work`
iii call tmux::windows::move session=api window=@3 target_session=work
# Just before (or after) window 0 of `work`; in the window's own session this reorders it
iii call tmux::windows::move session=work window=@3 target_session=work before=0
```

```json
{ "session_id": "$2", "window_id": "@3", "window_index": 0, "source_session_closed": false }
```

Without `before` or `after` the window goes after the session's last window,
even when an index between two windows is free (tmux's own `move-window -t
work:` would take that gap). `before` and `after` are tmux's `move-window -b`
and `-a`, so the windows from that spot on move up one index. They need tmux
3.2 and 2.1 or later; an older tmux gets one sentence saying so. The
destination's current window stays as it was, and a session's current window
reordered inside it stays current. When the window was the last one of its
session, tmux closes that session and `source_session_closed` is true; with
tmux's default `detach-on-destroy on`, clients attached there (a Console
terminal too) end, so switch a terminal to the target session first. The
window already in the target session without `before`/`after`, both at once,
or a window next to itself are refused in one sentence.

Panes work the same way. `tmux::panes::select` makes a pane active, for every
client looking at that window. `zoom` sets the window's zoom and never toggles
it the way prefix `z` does: `zoom: true` zooms the window on that pane (or keeps
it zoomed there), `zoom: false` unzooms it, and without `zoom` the window keeps
its zoom, moved to that pane. A window with one pane can't zoom, and the
answer's `zoomed` stays false. `tmux::panes::capture` returns a pane's visible
screen with its colour escapes, its size and cursor:

```bash
iii call tmux::panes::select pane=%3 zoom=true
iii call tmux::panes::capture pane=%3
```

`tmux::panes::split` puts a new pane to the `right` of a pane or `below` it. It
starts in that pane's current directory unless `cwd` says otherwise, runs the
default shell unless `command` does, and becomes the active pane unless
`select: false`. `tmux::panes::kill` kills a pane and its program; when that was
its window's last pane the window closes too (`window_closed`), and when that
window was its session's last, the session (`session_closed`).
`tmux::panes::respawn` starts a pane's program again in the same pane, the
command it was started with unless `command`. A pane whose program still runs
is refused unless `kill: true`; one kept on screen after its program ended
(tmux's `remain-on-exit`) is dead and restarts without it.

```bash
iii call tmux::panes::split pane=%3 direction=below command="npm test" select=false
iii call tmux::panes::respawn pane=%7 kill=true
iii call tmux::panes::kill pane=%7
```

```json
{ "session_id": "$1", "window_id": "@4", "pane_id": "%7" }
{ "pane_id": "%7" }
{ "killed": true, "window_closed": false, "session_closed": false }
```

## Configuration

```yaml
tmux_binary: tmux            # name on PATH or absolute path (never this worker's own binary)
socket_name: null            # tmux -L <name>; null = your default tmux server
socket_path: null            # tmux -S <path>; wins over socket_name
poll_interval_ms: 1000       # live feed cadence for sessions and agent state
ports_interval_ms: 3000      # process and listening-port re-scan cadence
busy_window_ms: 2000         # agents without working patterns: "working" while the pane changed this recently
command_timeout_ms: 5000     # one tmux command
max_terminals: 8             # concurrent Console terminals (each one is a tmux client)
terminal_idle_timeout_ms: 30000  # detach a Console terminal its page stopped heart-beating
agents:                      # first match wins; patterns are case-insensitive regexes over the pane's last lines
  - id: claude
    label: Claude Code
    process: [claude]
    working_patterns: ["esc to interrupt"]
    waiting_patterns: ["❯\\s*1\\.\\s*Yes", "Do you want to proceed\\?", "Enter to select"]
```

The built-in list also recognises `codex`, `opencode` and `gemini`; see
[`src/config.rs`](src/config.rs). Edit the configuration from the page's
settings action or with `configuration::set`.

## Observable functions

| Function | Args | Delivery | Event |
|---|---|---|---|
| `tmux::on-changed` | none; the overview has no field to filter on | iii's default delivery; once a new observer is accepted, the worker calls the function with the current overview for that observer alone (metadata `{ for_observer: <observer id> }`), delivered like every other call | `Overview { sessions, ports }`: the shapes `tmux::sessions::list` and `tmux::ports::list` return |

`tmux::on-changed` is private: only this worker calls it, whenever sessions,
windows, panes, agent states, compose stacks or listening ports change. The
worker polls tmux only while something observes it.

Overviews can arrive out of order: each one is its own call, and iii doesn't
order deliveries across calls, so two sent milliseconds apart (a mutation's
and the next poll's) may land the other way round, and nothing re-sends until
something changes again. Keep the overview with the highest
`sessions.generated_at_ms` (and `ports.generated_at_ms`) and drop an older one
that comes after it; the same rule settles a `list` answer racing a push.

```ts
iii.registerFunction('my-worker::on-tmux', async ({ output: { sessions, ports } }) => {
  // compare agent states with the previous overview
})
iii.observe({ function_id: 'tmux::on-changed', capture: ['output'] }, 'my-worker::on-tmux')
```

The Console page's terminal functions, `tmux::terminal::open`, `write`,
`resize`, `switch`, `replay`, `redraw`, `close` and `upload-image`, cannot be
observed (`NOT_OBSERVABLE`): `open` returns the terminal's access key, every
other call carries it, `write` carries what is typed into the terminal and
`upload-image` the image being pasted.

`tmux::terminal::upload-image` is how an image is pasted into a terminal. It
only writes a file: the image (`mime` `image/png`, `image/jpeg`, `image/gif` or
`image/webp`, matching the data's first bytes; `data` base64, at most 12 MiB
decoded) goes to a new file in the cache folder of the user this worker runs
as, and the answer is that file's absolute path. It types nothing into the
terminal; the page pastes the path itself, and Claude Code and Codex then
attach it as `[Image #1]` (Codex only when the paste is that path alone and its
model takes images; any other program just receives the path as text).

```json
{ "path": "/home/me/.cache/iii/tmux/paste/20261110-161648-0a1b2c3d.png",
  "bytes": 48213, "mime": "image/png" }
```

| Computer | Folder |
|---|---|
| Linux | `$XDG_CACHE_HOME/iii/tmux/paste` when that's an absolute path, otherwise `~/.cache/iii/tmux/paste` |
| macOS | `~/Library/Caches/iii/tmux/paste` |

The folder is readable by this user only (0700) and so is each file (0600),
named `<UTC yyyymmdd-hhmmss>-<8 hex>.<png|jpg|gif|webp>` with nothing from the
caller in it. Files of that name older than 7 days are removed when the worker
starts and on each upload, and nothing else in that folder is touched. They
are not removed right away because Codex reads the file only when the message
it is attached to is sent.
