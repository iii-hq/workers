# pi

Pi coding agent as an iii worker: the Pi API exposed as functions and streams on the iii bus, nothing else. The worker runs the same in-process agent loop Pi runs in the terminal, with the same tools (read, bash, edit, write) against any host directory. `pi::run` executes one headless turn and returns the result; the raw Pi events mirror verbatim onto the `pi::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console, the acp worker, and any sibling worker observe a Pi run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the same entrypoint the console and the acp worker drive, so both run Pi with no changes.

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

Call `pi::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Pi session files in engine state and resumes automatically.

Two ids come back from every run. `session_id` is the iii session id: the key for `pi::status`, `pi::stop`, `pi::steer`, resume, and the stream group. `pi_session_id` is Pi's internal session id — returned for reference, not a lookup key.

Long turns: use `pi::start` to return immediately, then watch `agent::events` (group_id = your session_id) for `message_complete`, `function_execution_start/end`, and `turn_end` frames. `pi::stop` interrupts a live run, `pi::status` reads a point-in-time view, `pi::sessions::list` enumerates past sessions.

## Functions

| Function | Purpose |
| --- | --- |
| `pi::run` | Run one turn, wait, return the final result |
| `pi::start` | Fire-and-forget turn; progress arrives on `agent::events` |
| `pi::steer` | Inject a steering instruction into a live run |
| `pi::follow_up` | Queue a follow-up message for a live run |
| `pi::stop` | Interrupt a live run |
| `pi::status` | Session state, live flag, usage, cost |
| `pi::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Alias for `pi::run` under the entrypoint the console and acp worker drive |

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
