# claude-code

Claude Code as an iii worker: the Claude Code API exposed as functions and streams on the iii bus, nothing else. The worker spawns the same `claude` binary the user runs in their terminal, with the same login, the same filesystem, and the same tools (file edits, shell, web). `claude::run` executes one headless turn and returns the result; the raw Claude Code messages mirror verbatim onto the `claude::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console, the acp worker, and any sibling worker observe a Claude Code run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the same entrypoint the console and the acp worker drive, so both run Claude Code with no changes.

The same worker also runs Claude Code as a **terminal on the console**: `claude::terminal::*` installs the CLI on the terminal host, equips a workspace with the iii skills, and opens Claude in a `shell::pty` session on its own page — always Claude, never a shell. Both halves report onto one events stream, so a headless turn and a typed turn look the same in the console. Whether they also share one login depends on where each half runs: from a local checkout both are host processes and read the same `~/.claude`, while a worker installed from the registry runs the headless half in a microVM that cannot (see [Logging in](#logging-in)). Its sibling [`pi-cli`](https://github.com/iii-hq/workers/tree/main/pi-cli) is the terminal half for the pi agent.

## Install

```bash
iii worker add claude-code
```

Requires the `claude` CLI on the host (the Agent SDK shells out to it) and either `ANTHROPIC_API_KEY` in the worker environment or an existing `claude` login.

The terminal half adds three companions:

| Worker | Why |
|---|---|
| [`shell`](https://github.com/iii-hq/workers/tree/main/shell) ≥ 0.12 | Owns the terminal. `shell::pty::open` runs the CLI, `shell::exec` installs it and equips the workspace, `coder::read-file` reads the files back. The workspace must be inside shell's jail (`fs.host_roots`). |
| [`console`](https://github.com/iii-hq/workers/tree/main/console) | Renders the injected **claude** page. |
| `configuration` (built in) | Holds the settings below. |

## Skills

Install the `claude-code` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill claude-code
```

## Quickstart

From zero to a Claude Code turn over the bus:

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
iii worker add claude-code
iii   # starts the engine + worker
```

Then talk to it like any other function: from the console chat, from `iii trigger claude::run`, or from any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.trigger({
  function_id: 'claude::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
    permission_mode: 'acceptEdits',
  },
  timeout_ms: 600_000,
});
// { session_id, claude_session_id, result, stop_reason, usage, total_cost_usd }
```

Or straight from the terminal with the `iii trigger` CLI:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii trigger claude::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads use key=value syntax
iii trigger claude::sessions::list
iii trigger claude::status session_id=<session_id>

# background turn + control
iii trigger claude::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii trigger claude::stop session_id=<session_id>

# ask the running engine for a function's description
iii trigger claude::run --help
```

A turn from the CLI and the session record it leaves behind:

![iii trigger claude::run returning the result with usage and cost](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/cli-run.png)

![iii trigger claude::status showing the stored session record](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/cli-status.png)

![iii trigger claude::run --help printing the published request schema as a parameter table](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/cli-help.png)

Call `claude::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Claude Code session ids in engine state and resumes automatically.

Two ids come back from every run. `session_id` is the iii session id: the key for `claude::status`, `claude::stop`, resume, and the stream group. `claude_session_id` is Claude Code's internal session id (what the worker passes to the CLI's resume under the hood) — returned for reference, not a lookup key.

Long turns: use `claude::start` to return immediately, then watch `agent::events` (group_id = your session_id) for `message_complete`, `function_execution_start/end`, and `turn_end` frames. `claude::stop` interrupts a live run, `claude::status` reads a point-in-time view, `claude::sessions::list` enumerates past sessions.

## Functions

| Function | Purpose |
| --- | --- |
| `claude::run` | Run one turn, wait, return the final result |
| `claude::start` | Fire-and-forget turn; progress arrives on `agent::events` |
| `claude::stop` | Interrupt a live run |
| `claude::status` | Session state, live flag, usage, cost |
| `claude::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Alias for `claude::run` under the entrypoint the console and acp worker drive |
| `claude::terminal::describe` | What a terminal session runs: program, argv, cwd, env — the page passes it straight to `shell::pty::open` — plus `activity_bridge` and `detail`, which say whether the hooks can reach the bus. Internal. |
| `claude::terminal::activity` | One Claude Code hook event in, AgentEvent frames out. Internal, and `trace_hidden` — the signal is the stream, not the delivery. |
| `claude::auth::status` | Which plan a terminal session spends (see [Billing](#billing-which-plan-a-session-spends)). Agent-denied. |
| `claude::ui-content` | Console page assets. Internal. |

`claude::run` accepts either a bare `prompt` string or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), plus `model`, `cwd`, `system_prompt`, `append_system_prompt`, `permission_mode`, `allowed_tools`, `disallowed_tools`, and `max_turns` overrides.

### Raw API pass-through

The named fields above cover the common path; everything else the Agent SDK accepts goes through the `options` field untouched (camelCase, exactly as in the SDK):

```jsonc
{
  "prompt": "...",
  "options": {
    "forkSession": true,
    "includePartialMessages": true,
    "fallbackModel": "claude-sonnet-4-6",
    "addDirs": ["/another/repo"]
  }
}
```

And the full output side is available raw: every message Claude Code emits (`system/init`, `assistant`, `user`, `result`, and `stream_event` token deltas when `includePartialMessages` is set) is mirrored verbatim onto the `claude::events` stream, group_id = session_id. Consumers that want the exact Claude Code wire format read `claude::events`; consumers that want harness-shaped frames read `agent::events`. Same turn, two views.

## The agent on the bus

By default every turn's system prompt carries the iii runtime context: the same engine-grounded rules as the harness identity prompts, retargeted to the `iii` CLI the agent reaches through its shell. The agent discovers capabilities from the live engine instead of memory — `iii trigger engine::functions::list` to find function ids, `iii trigger <fn> --help` as the contract before every first call, the registry flow (`directory::registry::workers::list/info`, `worker::add`) when nothing registered fits — plus the calling rules and error-handling discipline that go with them. The matching `Bash(iii *)` allow rule is added automatically so those calls run headless. Local file edits stay on Claude Code's native tools; backend actions go through registered functions.

```bash
# the agent answers this by querying the live engine itself
iii trigger claude::run --timeout-ms 300000 \
  --json '{"prompt":"List every worker connected to this engine and what each one does.","cwd":"/tmp"}'
```

Turn it off per call with `"iii_context": false` or globally in `config.yaml`; a caller-supplied `system_prompt` always wins verbatim and gets nothing appended.

## Plan mode and permission modes

`permission_mode` maps straight onto Claude Code's native modes, per turn:

| Mode | Behavior |
| --- | --- |
| `default` | Claude Code's standard permission prompts (headless: unapproved calls fail) |
| `acceptEdits` | File edits auto-approved; the worker default |
| `plan` | Native plan mode: read-only exploration, produces a plan, refuses edits |
| `bypassPermissions` | Skip all permission checks |

Plan mode headless behaves like plan mode in the terminal: the turn ends when Claude finishes the plan, and the plan text is the `result` — nothing executes. Because the worker resumes sessions, plan-then-execute is two calls against the same `session_id`:

```bash
# 1. plan (read-only)
iii trigger claude::run --timeout-ms 600000 \
  --json '{"prompt":"Plan how to add rate limiting to the REST API. Do not implement.","cwd":"/path/to/repo","permission_mode":"plan"}'

# 2. execute the plan with full context, same conversation
iii trigger claude::run --timeout-ms 600000 \
  --json '{"session_id":"<from-step-1>","prompt":"Implement the plan.","permission_mode":"acceptEdits","cwd":"/path/to/repo"}'
```

The approval step is whatever sits between the two calls — a human reading the plan, another worker, or a trigger.

## The terminal on the console

A **claude** page appears in the console nav; opening it starts a session. The
first run installs the CLI and the skills, so it takes a minute. Answer the
login prompt in the terminal, then ask Claude to build something on the engine
— the workspace notes and the installed iii skills teach it how to register
functions and triggers. Every prompt and tool call it runs is streamed onto
`agent::events`, exactly like a `claude::run` turn.

```bash
iii trigger claude::terminal::describe   # what a session runs, and where
iii trigger shell::pty::sessions         # what is actually running
iii trigger stream::list stream_name=agent::events
```

The page keeps a per-tab lease, so a reload or a pane move reattaches to the
same live Claude rather than starting a second one.

A session outlives the tab that opened it: the reattach lease is stored per
browser, so closing the console and coming back finds the same agent instead
of starting a second one beside it. When the replay would be partial — the
worker's ring buffer is finite and an agent repaints constantly — the page
skips the broken history and asks the agent to paint its current screen (a
one-row resize, which is a SIGWINCH) rather than writing wreckage into the
pane.

The status bar carries a font-size stepper (8–40 px, 14 by default; Ctrl or ⌘
+ scroll does the same). The size is one value for every terminal in the
console — shell's panes included — so it is set once, not per page. The
terminal itself stays dark whatever the console theme is: an agent CLI paints
its own interface in colors chosen for a dark terminal and never learns the
page went light.

### What the workspace gets

On every boot, when `terminal.setup_workspace` is on:

- The `iii-hq/iii` skills, once per workspace (`npx skills add iii-hq/iii
  --all`), with a minimal `package.json` first — the skills CLI installs at
  the nearest manifest, and without one they land above the workspace where
  Claude does not look.
- `CLAUDE.md`, inside an `<!-- iii:begin -->` block the worker rewrites.
  Anything outside the markers is the operator's and survives.
- `.claude/settings.json` hooks for `SessionStart`, `SessionEnd`,
  `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, and `Stop`. Each posts its
  payload to `claude::terminal::activity` with the `iii` CLI — the bus, so it
  works whether or not the terminal host is this worker's host. Only these
  keys are rewritten; the rest of the file is left alone.

## Billing: which plan a session spends

The status bar under the terminal says it, and `claude::auth::status` answers
it on the bus:

| Badge | Means |
|---|---|
| `team subscription · you@example.com · org` | A Claude subscription login on the terminal host pays. |
| `API key billing · ANTHROPIC_API_KEY` | An API key pays, per token. |
| `not signed in` | No credentials yet — run `/login` in the terminal. |

The rule has a trap in it, which is why the badge exists: **an API key beats a
subscription login.** With `ANTHROPIC_API_KEY` set, `claude auth status` still
reports `authMethod: claude.ai` (the account IS signed in) while the CLI bills
the key — it says so itself ("ANTHROPIC_API_KEY or another auth source is set
and takes precedence over your claude.ai login") and a bogus key 401s the turn.
So the badge reads `apiKeySource`, not `authMethod`, and the tooltip spells out
that the login is signed in but not billed.

The key is read from the environment of the `shell` worker that runs the
session, so that is where to set or unset it.

**A subscription login needs `USER` in the session environment.** Claude Code
keeps subscription credentials in the OS keychain and finds them by the
current user, and a worker's environment is not the operator's shell — compose
clears it and re-seeds an allowlist from the daemon's own environment, so a
daemon started without `USER` hands every child a blank one. The CLI then
reports `loggedIn: false` beside a keychain that plainly holds the login. This
worker asks the terminal host who it is (`id -un`) and puts `USER`/`LOGNAME`
into every session and into the billing probe, so the answer does not depend
on how the daemon was started.

`claude::auth::status` stays agent-denied in `iii-permissions.yaml`: it carries
the operator's account and organization. The console page reaches it as a
user-initiated call, which is not the agent path.

## Logging in

The simplest flow is the terminal itself: open the page and run `/login` (or
`claude auth login`; `--console` for API billing instead of the subscription).
The credentials land in the home directory of whatever runs the session — the
`shell` worker's — and every later session of this terminal reuses them.

**Who else sees that login depends on how a worker is deployed, not on which
machine it is.** Compose starts a container one of two ways
(`iii-compose/src/lifecycle.rs`):

| Container | Start | Home directory |
|---|---|---|
| `path://…` (local checkout), or a registry worker whose payload is a **binary** (Rust `deploy: binary`, e.g. `shell`) | host process | the host's |
| a registry worker whose payload is a **bundle** (Node `deploy: bundle`, e.g. this worker) | microVM, own rootfs | the guest's |

Only the container's own config directory is shared into a VM (virtiofs), and
compose v1 has no volume field, so a VM-booted worker cannot read the host's
`~/.claude`. Consequences:

- The terminal half is unaffected either way: the CLI runs inside the `shell`
  worker, which is a host process, so the login is the host's and this
  worker's own rootfs never matters.
- The headless half spawns `claude` in **this** worker's container. Run from a
  local checkout it is a host process and shares that same login; installed
  from the registry it is in a microVM and does **not**. Give it credentials
  through the channels a VM has: `environment` / `env_file` on its container
  (`ANTHROPIC_API_KEY`, or a long-lived subscription token from `claude
  setup-token`), or the `auth-credentials` vault over the bus — which is why
  that vault exists, and why `provider-claude-code` reads it instead of the
  file.

### If the worker that owns the terminal is virtualized

The terminal half holds because `shell` is a binary payload and therefore a
host process today. If binary workers get VM-booted too, the terminal keeps
working — the CLI, the workspace, and the login simply move into that guest,
and this worker still reaches them over the bus — but three things change:

- The login belongs to that guest's rootfs. Nothing on the host shares it, and
  it survives restarts only as long as the guest's state does.
- The workspace is inside the guest, so the agent no longer sees the
  operator's checkout unless the guest is given it.
- The activity hooks call the `iii` CLI, which may not exist in that guest.
  The worker probes for it and reports the answer as `activity_bridge` on
  `claude::terminal::describe` (empty = the hooks are installed but mute, and
  `detail` says so). That is the first thing to check if a terminal works
  while `agent::events` stays empty.

For a terminal host with no one at the keyboard, the same two options apply:
`claude setup-token` once (needs a browser once, returns a long-lived
subscription token) or `ANTHROPIC_API_KEY`, in the environment the `shell`
worker starts with. Either way the badge says which one won.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                    # empty = Claude Code default
  permission_mode: acceptEdits # default | acceptEdits | plan | bypassPermissions
  max_turns: 50
  cwd: ""                      # default working directory for runs

approval_gate: false           # route tool permissions through policy::check_permissions
events_stream: agent::events   # translated AgentEvent frames
raw_events_stream: claude::events  # verbatim Claude Code messages
claude_executable: ""          # path to the claude CLI; empty = SDK default resolution

terminal:                      # the console terminal page — a DIFFERENT host: shell's
  executable: ""               # path to `claude` there; empty = resolve on PATH there
  args: []                     # extra argv for every terminal session
  workspace_dir: ""            # empty = `claude-code` under shell's primary root
  auto_install: true           # install Claude Code from https://claude.ai/install.sh when missing
  setup_workspace: true        # keep the skills, the engine notes, and the activity hooks in place
```

Settings live in the `configuration` worker under the **Claude Code** entry —
edit them on the console's Workers tab; they hot-reload, and a change to the
`terminal` block re-runs the workspace preparation for the next session.

With `approval_gate: true` and the harness worker installed, every Claude Code tool call is checked against `policy::check_permissions` before it executes, fail-closed when the gate is unreachable, so the same YAML permission rules and console approval flow that govern native harness turns govern Claude Code.

## Observability

Every `claude::run` is an ordinary traced invocation on the engine: the trace carries the full input payload (prompt, cwd, caller worker id) and the output (result, stop reason, token usage, cost) as span events, with per-function p50/p95/p99 in the console's trace explorer — no extra instrumentation in the worker.

![claude::run invocations in the iii console trace explorer, with input and output payloads](https://raw.githubusercontent.com/iii-hq/workers/main/claude-code/assets/console-traces.png)

### Turns group like a harness turn

Both halves stamp the identity keys an agent harness stamps, so the trace views
group and label this worker's turns with no knowledge of Claude Code
(`console/docs/timeline-span-tags.md`):

| Key | Value here |
| --- | --- |
| `iii.session.id` | the iii session — what "group by session" groups on |
| `iii.message.id` | one turn: one headless `claude::run`, or one terminal prompt |
| `iii.tag.kind` | `claude.run` for a headless turn, `claude.terminal.turn` for a typed one |
| `iii.tag.message` | a preview of the prompt |
| `iii.tag.display_name` | `Claude terminal · <prompt>` on the terminal half |

The transport is W3C baggage, so every call a turn makes — and every span those
calls produce in other workers — carries the same keys. A terminal turn reaches
this worker as several separate calls (one per hook), so its identity lives in
those keys rather than in one covering span; a headless turn gets both.

## How it maps

| Claude Code | iii |
| --- | --- |
| SDK `query()` turn | `claude::run` invocation |
| every SDK message, verbatim | `claude::events` stream frame |
| assistant message | `message_complete` frame on `agent::events` |
| tool_use / tool_result | `function_execution_start` / `function_execution_end` frames |
| final result | `turn_end` + `agent_end` frames, function return value |
| session resume | engine state scope `claude_sessions`, keyed by iii session_id |
| permission prompt | `canUseTool` -> `policy::check_permissions` (optional) |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |
| interactive CLI session | `shell::pty` session on the injected **claude** console page |
| terminal hook event | `claude::terminal::activity` -> the same `agent::events` frames |
