# pi

Pi coding agent as an iii worker: the Pi API exposed as functions and streams on the iii bus, nothing else. The worker runs the same in-process agent loop Pi runs in the terminal, with the same tools (read, bash, edit, write) against any host directory. `pi::run` executes one headless turn and returns the result; the raw Pi events mirror verbatim onto the `pi::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console, the acp worker, and any sibling worker observe a Pi run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the same entrypoint the console and the acp worker drive, so both run Pi with no changes.

The same worker also runs pi as a **terminal on the console**: `pi::terminal::*` installs the CLI on the terminal host, equips a workspace with the iii skills, and opens pi in a `shell::pty` session on its own page — always pi, never a shell. Both halves report onto one events stream, so a headless turn and a typed turn look the same in the console. Whether they also share one login depends on where each half runs (see [Logging in](#logging-in)). Its sibling [`claude-code`](https://github.com/iii-hq/workers/tree/main/claude-code) is the same shape for Claude Code.

## Install

```bash
iii worker add pi
```

Pi runs the loop in-process (no CLI subprocess), so the worker environment needs model credentials — `ANTHROPIC_API_KEY` (or the provider key Pi is configured for) or an existing Pi login.

## Skills

Install the `pi` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill pi
```

## Quickstart

From zero to a Pi turn over the bus:

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
iii worker add pi
iii   # starts the engine + worker
```

Then talk to it like any other function: from the console chat, from `iii trigger pi::run`, or from any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.trigger({
  function_id: 'pi::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
  },
  timeout_ms: 600_000,
});
// { session_id, pi_session_id, result, stop_reason, usage, total_cost_usd }
```

Or straight from the terminal with the `iii trigger` CLI:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii trigger pi::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads use key=value syntax
iii trigger pi::sessions::list
iii trigger pi::status session_id=<session_id>

# background turn + control
iii trigger pi::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii trigger pi::stop session_id=<session_id>

# ask the running engine for a function's description
iii trigger pi::run --help
```

A turn from the CLI returns the result with token usage and cost:

![iii trigger pi::run returning pong with usage and cost](https://raw.githubusercontent.com/iii-hq/workers/main/pi/assets/cli-run.png)

`iii trigger pi::run --help` prints the published request schema as a parameter table:

![iii trigger pi::run --help printing the request schema as a parameter table](https://raw.githubusercontent.com/iii-hq/workers/main/pi/assets/cli-help.png)

Call `pi::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Pi session files in engine state and resumes automatically.

![iii trigger pi::sessions::list showing the stored session records](https://raw.githubusercontent.com/iii-hq/workers/main/pi/assets/cli-sessions.png)

Two ids come back from every run. `session_id` is the iii session id: the key for `pi::status`, `pi::stop`, `pi::steer`, resume, and the stream group. `pi_session_id` is Pi's internal session id — returned for reference, not a lookup key.

Long turns: use `pi::start` to return immediately, then watch `agent::events` (group_id = your session_id) for `message_complete`, `function_execution_start/end`, and `turn_end` frames. `pi::stop` interrupts a live run, `pi::status` reads a point-in-time view, `pi::sessions::list` enumerates past sessions.

## Functions

| Function | Purpose |
| --- | --- |
| `pi::run` | Run one turn, wait, return the final result |
| `pi::start` | Fire-and-forget turn; progress arrives on `agent::events` |
| `pi::task` | Delegate one task as a SUB-AGENT: fire it from a trigger, get the session id back at once, pass `parent_session_id` to nest it under the session that delegated it, and read the outcome from `agent_tasks/<session id>` in state — which is what an orchestrator binds a `state` trigger to and is woken by |
| `pi::steer` | Inject a steering instruction into a live run |
| `pi::follow_up` | Queue a follow-up message for a live run |
| `pi::stop` | Interrupt a live run |
| `pi::status` | Session state, live flag, usage, cost |
| `pi::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Alias for `pi::run` under the entrypoint the console and acp worker drive |
| `pi::terminal::describe` | What a terminal session runs: program, argv, cwd, env — the page passes it straight to `shell::pty::open` — plus `activity_bridge` and `detail`. Internal. |
| `pi::terminal::activity` | One pi extension event in, AgentEvent frames out. Internal, and `trace_hidden` — the signal is the stream, not the delivery. |
| `pi::auth::status` | Which plan a terminal session spends (see Billing). Agent-denied. |
| `pi::ui-content` | Console page assets. Internal. |

`pi::run` accepts either a bare `prompt` string or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), plus `model`, `cwd`, `thinking_level`, `tools`, and `iii_context` overrides.

### Raw events

Every event Pi emits (`agent_start/end`, `turn_start/end`, `message_start/update/end`, `tool_execution_start/update/end`, and the session events `queue_update`, `compaction_start/end`) is mirrored verbatim onto the `pi::events` stream, group_id = session_id. Consumers that want the exact Pi event format read `pi::events`; consumers that want harness-shaped frames read `agent::events`. Same turn, two views.

## Steering a live run

A turn started with `pi::start` keeps its session reachable while it streams. Two functions push instructions into it:

```bash
# start a long run
iii trigger pi::start --json '{"prompt":"refactor the auth module","cwd":"/path/to/repo","session_id":"s1"}'

# redirect it mid-flight — applied after the current tool calls finish
iii trigger pi::steer --json '{"session_id":"s1","prompt":"stop, keep the public API unchanged"}'

# queue work for after it would otherwise stop
iii trigger pi::follow_up --json '{"session_id":"s1","prompt":"then add tests for the new paths"}'
```

`pi::steer` maps onto Pi's steering queue (interrupt-style), `pi::follow_up` onto its follow-up queue (wait-style). Both no-op with `{steered:false}` / `{queued:false}` when no run is live for the session.

## The agent on the bus

By default every turn's prompt carries the iii runtime context: the same engine-grounded rules as the harness identity prompts, retargeted to the `iii` CLI the agent reaches through its shell. The agent discovers capabilities from the live engine instead of memory — `iii trigger engine::functions::list` to find function ids, `iii trigger <fn> --help` as the contract before every first call, the registry flow (`directory::registry::workers::list/info`, `worker::add`) when nothing registered fits — plus the calling rules and error-handling discipline that go with them. Local file edits stay on Pi's native tools; backend actions go through registered functions.

```bash
# the agent answers this by querying the live engine itself
iii trigger pi::run --timeout-ms 300000 \
  --json '{"prompt":"List every worker connected to this engine and what each one does.","cwd":"/tmp"}'
```

Pi answers by querying the live engine itself, grouping every connected worker by role:

![iii trigger pi::run enumerating every worker connected to the engine](https://raw.githubusercontent.com/iii-hq/workers/main/pi/assets/cli-discovery.png)

The context is prepended on a fresh session and skipped on resume (it is already in the conversation history). Turn it off entirely with `"iii_context": false` per call or globally in `config.yaml`.

## Thinking and tools

`thinking_level` maps straight onto Pi's reasoning levels, per turn:

| Level | Behavior |
| --- | --- |
| `off` | No extended reasoning |
| `minimal` / `low` | Short reasoning budget |
| `medium` | The worker default |
| `high` / `xhigh` | Deep reasoning (xhigh on supported model families) |

`tools` is an allowlist: leave it empty for Pi's defaults (`read`, `bash`, `edit`, `write`), or pass a subset to narrow what the turn can do — e.g. `{"tools":["read","bash"]}` for a read-and-run turn that cannot edit or write files.

## The terminal on the console

A **pi** page appears in the console nav; opening it starts a session. The first
run installs the CLI and the skills, so it takes a minute. Log in with `/login`
if the host has no provider key, then ask pi to build something on the engine —
the workspace notes and the installed iii skills teach it how to register
functions and triggers.

```bash
iii trigger stream::list stream_name=agent::events
iii trigger pi::terminal::describe   # what a session runs, and where
```

The page keeps a per-browser lease, so a reload, a pane move, or closing the
console and coming back reattaches to the same live pi rather than starting a
second one beside it. `shell::pty::sessions` lists what is actually running.
When a replay would be partial — the worker's ring buffer is finite and an agent
repaints constantly — the page skips the broken history and asks the agent to
paint its current screen (a one-row resize, which is a SIGWINCH) rather than
writing wreckage into the pane.

The status bar carries a font-size stepper (8–40 px, 14 by default; Ctrl or ⌘ +
scroll does the same). The size is one value for every terminal in the console —
shell's panes included. The terminal itself stays dark whatever the console theme
is: an agent CLI paints its own interface in colors chosen for a dark terminal
and never learns the page went light.

`-a` is in the default terminal args for a reason: pi loads project-local
extensions only in a trusted directory, and asks about trust otherwise. Without
it, every session opens with a prompt and never loads the extension that reports
what pi did. Drop the flag if you would rather answer that question yourself.

## What the workspace gets

On every boot, when `setup_workspace` is on:

- `AGENTS.md`, inside an `<!-- iii:begin -->` block the worker rewrites.
  Anything outside the markers is the operator's and survives. The iii half of
  that block — how to work against a live engine, and what skills are installed
  — is FETCHED from the [`iii-directory`](https://github.com/iii-hq/workers/tree/main/iii-directory)
  worker (`directory::system-prompts::get name=iii-runtime` plus
  `directory::skills::index`), which is also what a headless turn is given. One
  copy, one owner: this worker ships neither the prompt nor a second set of
  skills on disk. When the directory answers nothing, the block says so and
  points the agent at `engine::functions::list`.
- `.pi/extensions/iii-activity.ts`, which posts pi's lifecycle events
  (`session_start`, `before_agent_start`, `tool_execution_start`/`end`,
  `agent_end`, `session_shutdown`) to `pi::terminal::activity` with the `iii` CLI —
  the bus, so it works whether or not the terminal host is this worker's host.
  It is discovered from the workspace, so a session an operator starts by hand
  in the same directory reports its turns too.

## Billing: which plan a session spends

The status bar under the terminal says it, and `pi::auth::status` answers
it on the bus. pi keeps credentials per provider, so the badge reports the
provider named by `terminal.auth_provider`:

| Badge | Means |
|---|---|
| `anthropic (subscription)` | One provider is signed in, with an OAuth login behind it. |
| `openai (API key)` | One provider is signed in, paying per token. |
| `2 providers · openai, anthropic` | Several are usable; the tooltip names each one's kind, and which one a turn spends follows the model it runs. |
| `no provider signed in` | Nothing usable yet — run `/login` in the terminal, or set a provider's API key. |

pi is not one account, so the badge is not one provider. The names come from
pi's own auth store (`~/.pi/agent/auth.json`, read for NAMES only), and each
one's state comes from `pi auth check` — never a credential. `terminal.auth_provider` in
the settings is now only the fallback asked about when that store cannot be
read.

It runs `pi auth check --provider <p> --json` on the terminal host and never
passes `--credentials`: the page needs the KIND of credential, never the
credential. `pi auth check` exits 1 for a provider that is not ready and still
prints the JSON that says why, so the exit code is not read as the answer.

`pi::auth::status` stays agent-denied in `iii-permissions.yaml`. The
console page reaches it as a user-initiated call, which is not the agent path.

## Logging in

The simplest flow is the terminal itself: open the page and run `/login`. The
credentials land in the home directory of whatever runs the session — the
`shell` worker's — and every later session of this terminal reuses them.

**Who else sees that login depends on how this worker is deployed.** Compose
runs a `path://` container and a registry **binary** payload as host processes,
and a registry **bundle** payload (Node `deploy: bundle`, which this worker is)
in a microVM with its own rootfs; only the container's config directory is
shared in, and compose v1 has no volume field. So the terminal half is
unaffected either way — the CLI runs inside the `shell` worker, a host process —
while the headless half shares that login only when it is a host process too.
Run from the registry it is in a microVM and does not: give it a provider key
through `environment` / `env_file` on its container.

### If the worker that owns the terminal is virtualized

Everything above holds because `shell` is a binary payload and therefore a host
process today. If binary workers get VM-booted too, the terminal keeps working
— pi, the workspace, and the login move into that guest — but the login belongs
to that guest's rootfs, the workspace is no longer the operator's checkout, and
the activity extension calls the `iii` CLI, which may not exist there. The
worker probes for it and reports the answer as `activity_bridge` on
`pi::terminal::describe` (empty = the extension is installed but mute, with
`detail` saying so) — the first thing to check if a terminal works while
`agent::events` stays empty.

For a terminal host with no one at the keyboard, put the provider's key in the
environment the `shell` worker starts with. Either way the badge says which one
won.

## Turns group like a harness turn

Every extension event carries the identity keys an agent harness stamps, so the
console's trace views group and label a pi session with no knowledge of pi
(`console/docs/timeline-span-tags.md`):

| Key | Value here |
| --- | --- |
| `iii.session.id` | the terminal session — what "group by session" groups on |
| `iii.message.id` | one pi run: the prompt and every tool call it makes |
| `iii.tag.kind` | `pi.terminal.turn` |
| `iii.tag.message` | a preview of the prompt |
| `iii.tag.display_name` | `pi terminal · <prompt>` |

The transport is W3C baggage, so the calls a run makes carry the same keys. One
run reaches this worker as several separate calls (one per extension event), so
its identity lives in those keys rather than in one covering span.
## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                # empty = Pi settings default; else "provider/modelId"
  thinking_level: medium   # off | minimal | low | medium | high | xhigh
  cwd: ""                  # default working directory for runs
  tools: []                # empty = Pi defaults (read, bash, edit, write)
  agent_dir: ""            # Pi global config dir; empty = ~/.pi/agent

events_stream: agent::events   # translated AgentEvent frames
raw_events_stream: pi::events  # verbatim Pi events
iii_context: true              # prepend the iii runtime context on fresh sessions

terminal:                  # the console page: what a typed session runs
  executable: ""           # path to `pi` on the terminal host; empty = resolve on PATH
  args: ["-a", "--use-theme", "dark"]   # `-a` trusts the workspace; the theme matches the page
  workspace_dir: ""        # empty = `pi` under shell's primary root
  auto_install: true       # install pi from https://pi.dev/install.sh when missing
  setup_workspace: true    # keep skills, engine notes, and the activity extension in place
  auth_provider: anthropic # fallback provider for the billing badge
```

`config.yaml` is the seed: on first boot the worker registers it with the built-in `configuration` worker as the initial value, then reads the live value back and hot-reloads on every `configuration:updated`. `engine_url` is excluded from the managed schema — it is bootstrap, so it stays on the local seed / `--url`.

## Observability

Every `pi::run` is an ordinary traced invocation on the engine: the trace carries the full input payload (prompt, cwd, caller worker id) and the output (result, stop reason, token usage, cost) as span events, with per-function p50/p95/p99 in the console's trace explorer — no extra instrumentation in the worker.

## How it maps

| Pi | iii |
| --- | --- |
| `AgentSession.prompt()` turn | `pi::run` invocation |
| every AgentSession event, verbatim | `pi::events` stream frame |
| assistant `message_end` | `message_complete` frame on `agent::events` |
| `tool_execution_start` / `tool_execution_end` | `function_execution_start` / `function_execution_end` frames |
| final result | `turn_end` + `agent_end` frames, function return value |
| `steer()` / `followUp()` | `pi::steer` / `pi::follow_up` |
| session resume | engine state scope `pi_sessions`, keyed by iii session_id |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |
