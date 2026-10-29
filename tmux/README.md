# tmux

tmux as an iii worker. It lists every tmux session, window and pane on the
host, creates, renames and kills sessions and windows, and tells you what is
running where: which pane holds Claude Code or Codex and whether that agent is
working, waiting for you or idle, which pane runs `iii compose` and on which
port its engine listens, and every TCP port your processes listen on, traced
back to the pane that started them. Its Console page embeds a terminal that
is a real tmux client: what you type goes straight into the session, and the
screen updates as tmux draws it, the same way a second terminal attached with
`tmux attach` would.

## Install

```bash
iii trigger compose::add worker=tmux
```

The worker drives the `tmux` binary installed on the host (Linux or macOS) and
talks to the same tmux server you use from your terminal. Without tmux it still
starts and reports `available: false`.

## Quickstart

Open the **tmux** page in the Console to browse sessions, open one in the
terminal, and see the **Ports** tab. From code or an agent:

```bash
# Every session → window → pane, with agent / compose / port status
iii trigger tmux::sessions::list

# A new session running a dev server, then a second window in it
iii trigger tmux::sessions::create name=api cwd=~/code/api command="pnpm dev"
iii trigger tmux::windows::create session=api name=logs

# Who listens on which port, and which compose stacks are up
iii trigger tmux::ports::list
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
iii trigger tmux::subagents::transcript pane=%9 subagent=01a16528-d646
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

Panes work the same way. `tmux::panes::select` makes a pane active and, with
`zoom: true`, zooms the window on it the way prefix `z` does, for every client
looking at that window; `zoom: false` unzooms. `tmux::panes::capture` returns a
pane's visible screen with its colour escapes, its size and cursor:

```bash
iii trigger tmux::panes::select pane=%3 zoom=true
iii trigger tmux::panes::capture pane=%3
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

## Custom trigger types

| Trigger type | Fires when | Payload to subscribers |
|---|---|---|
| `tmux::changed` | Once when bound, then whenever sessions, windows, panes, agent states, compose stacks or listening ports change | `{ sessions, ports }` — the same shapes `tmux::sessions::list` and `tmux::ports::list` return |

Bind it with `config: {}`. The worker only polls tmux while at least one
binding exists.
