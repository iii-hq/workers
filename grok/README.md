# grok

The [xAI Grok CLI](https://docs.x.ai) as an iii worker: the Grok agent exposed as functions and streams on the iii bus, nothing else. The worker spawns the same `grok` binary the user runs in their terminal, with the same `XAI_API_KEY`, the same filesystem, and the same working directory. `grok::run` executes one headless turn (`grok --single <prompt> --output-format streaming-json`) and returns the result; the raw Grok events mirror verbatim onto the `grok::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console and any sibling worker observe a Grok run exactly like a native harness turn.

## Install

```bash
iii worker add grok
```

Requires the Grok CLI on the host and `XAI_API_KEY` in the worker environment. See [docs.x.ai](https://docs.x.ai) for CLI installation and authentication.

## Skills

Install the `grok` agent skill for Claude Code, Cursor, and 30+ other agents:

```bash
npx skills add iii-hq/workers --skill grok
```

## Quickstart

From zero to a Grok turn over the bus:

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
iii worker add grok
iii   # starts the engine + worker
```

Then talk to it like any other function: from `iii trigger grok::run`, or from any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.trigger({
  function_id: 'grok::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
  },
  timeout_ms: 600_000,
});
// { session_id, grok_thread_id, result, stop_reason, num_turns }
```

Or straight from the terminal with the `iii trigger` CLI:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii trigger grok::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads use key=value syntax
iii trigger grok::sessions::list
iii trigger grok::status session_id=<session_id>

# background turn + control
iii trigger grok::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii trigger grok::stop session_id=<session_id>

# ask the running engine for a function's description and parameter table
iii trigger grok::run --help
```

Call `grok::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Grok session ids in engine state and resumes automatically (sessions persist in `~/.grok/sessions`).

Two ids come back from every run. `session_id` is the iii session id: the key for `grok::status`, `grok::stop`, resume, and the stream group. `grok_thread_id` is Grok's own session id (what the worker passes to `--resume` on the next turn) — returned for reference, not a lookup key.

## Functions

| Function | Purpose |
| --- | --- |
| `grok::run` | Run one turn, wait, return the final result |
| `grok::start` | Fire-and-forget turn; progress arrives on the streams |
| `grok::stop` | Interrupt a live run |
| `grok::status` | Session state, live flag, turn count |
| `grok::sessions::list` | All sessions this worker has run |

`grok::run` accepts either a bare `prompt` string or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`) — the same input contract as the claude-code worker and `run::start_and_wait`, so the acp worker drives Grok with `--brain-fn grok::run` — plus `model`, `cwd`, `always_approve`, and `iii_context`.

### Raw events

Every line Grok emits on its `--output-format streaming-json` stream is mirrored verbatim onto the `grok::events` stream, group_id = session_id. Consumers that want the exact Grok wire format read `grok::events`; consumers that want harness-shaped frames read `agent::events`. Same turn, two views.

The streaming-json stream (captured from Grok CLI 0.2.77) is delta-based: assistant text arrives as `{"type":"text","data":"<chunk>"}` lines, the turn closes with `{"type":"end","stopReason","sessionId","requestId"}`, and failures arrive as `{"type":"error","message"}`. The worker accumulates the text deltas and emits one `message_complete` frame on `agent::events` at `end`.

> Note: the Grok CLI streaming-json schema is not formally published, so the typed model in [`src/grok/events_types.rs`](src/grok/events_types.rs) is lenient — unrecognized event types pass through verbatim on `grok::events` and are skipped on the translated stream rather than failing the turn. Headless output carries no token usage and does not break out tool-call events today.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""             # empty = Grok CLI default (e.g. grok-build-0.1)
  cwd: ""               # default working directory for runs
  always_approve: true  # auto-approve tool/command execution on headless turns

events_stream: agent::events     # translated AgentEvent frames
raw_events_stream: grok::events  # verbatim Grok streaming-json events
grok_executable: ""              # path to the grok CLI; empty = PATH resolution
base_url: ""                     # override the xAI API base URL; empty = default
```

Auth is the Grok CLI's own: the worker inherits `XAI_API_KEY` from its environment. `always_approve` keeps headless turns from blocking on an interactive approval prompt; set it to `false` to let the CLI's approval policy gate tool execution.

## The agent on the bus

By default every turn carries the iii runtime context, prepended to the prompt on the first turn of a session: the same engine-grounded rules as the harness identity prompts, retargeted to the `iii` CLI the agent reaches through its shell. The agent discovers capabilities from the live engine instead of memory — `iii trigger engine::functions::list` to find function ids, `iii trigger <fn> --help` as the contract before every first call, the registry flow (`directory::registry::workers::list/info`, `worker::add`) when nothing registered fits — plus the calling rules and error-handling discipline that go with them. Local file edits stay on Grok's native tools; backend actions go through registered functions.

```bash
# the agent answers this by querying the live engine itself
iii trigger grok::run --timeout-ms 300000 \
  --json '{"prompt":"List every worker connected to this engine and what each one does.","cwd":"/tmp"}'
```

The context is injected once at the start of a session; resumed turns rely on the existing session history. Turn it off per call with `"iii_context": false` or globally in `config.yaml`.

## Observability

Every `grok::run` is an ordinary traced invocation on the engine: the trace carries the full input payload and the output (result, stop reason) as span events, with per-function p50/p95/p99 in the console's trace explorer — no extra instrumentation in the worker. Headless Grok output carries no token usage today, so the worker does not report it.

## How it maps

| Grok | iii |
| --- | --- |
| one headless `grok --single` turn | `grok::run` invocation |
| every streaming-json line, verbatim | `grok::events` stream frame |
| accumulated `text` deltas at `end` | `message_complete` frame on `agent::events` |
| turn `end` | `turn_end` + `agent_end` frames, function return value |
| `end.sessionId` → `--resume` next turn | engine state scope `grok_sessions`, keyed by iii session_id |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |
