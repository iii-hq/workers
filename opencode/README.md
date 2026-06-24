# opencode

OpenCode as an iii worker: the OpenCode API exposed as functions and streams on the iii bus, nothing else. The worker spawns the same `opencode` CLI the user runs in their terminal, with the same login, filesystem, and tools. `opencode::run` executes one headless turn (`opencode run --format json`) and returns the result with token usage and cost; the raw JSON events mirror verbatim onto the `opencode::events` stream, and a translated AgentEvent view lands on `agent::events`, so the iii console, the acp worker, and any sibling worker observe an OpenCode run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the same entrypoint the console and acp worker drive.

## Install

```bash
iii worker add opencode
```

Requires the `opencode` CLI on the host (`npm i -g opencode-ai` or the official installer) and an API key for the LLM provider you use — `ANTHROPIC_API_KEY` (or any provider key) in the worker environment, or an existing `opencode auth` login.

## Skills

```bash
npx skills add iii-hq/workers --skill opencode
```

## Quickstart

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
iii worker add opencode
iii   # starts the engine + worker
```

Then drive it from the console chat, `iii trigger`, or any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.trigger({
  function_id: 'opencode::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
    model: 'anthropic/claude-sonnet-4-5',
  },
  timeout_ms: 600_000,
});
// { session_id, opencode_session_id, result, stop_reason, usage, total_cost_usd }
```

Or from the terminal:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii trigger opencode::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads
iii trigger opencode::sessions::list
iii trigger opencode::status session_id=<session_id>

# background turn + control
iii trigger opencode::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii trigger opencode::stop session_id=<session_id>

iii trigger opencode::run --help
```

Call `opencode::run` again with the returned `session_id` to continue: the worker captures OpenCode's session id (`ses_...`) from the event stream, maps it to your iii `session_id` in engine state, and resumes with `--session`.

Two ids come back. `session_id` is the iii session id (the key for status/stop/resume and the stream group). `opencode_session_id` is OpenCode's own `ses_...` id, returned for reference.

Long turns: use `opencode::start` to return immediately, then watch `agent::events` (group_id = session_id) for `message_complete`, `function_execution_start/end`, and `turn_end`. `opencode::stop` interrupts a live run.

## Functions

| Function | Purpose |
| --- | --- |
| `opencode::run` | Run one turn, wait, return the final result + usage + cost |
| `opencode::start` | Fire-and-forget turn; progress arrives on `agent::events` |
| `opencode::stop` | Interrupt a live run |
| `opencode::status` | Session state, live flag, usage, cost |
| `opencode::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Alias for `opencode::run` under the shared agent entrypoint |

`opencode::run` accepts a bare `prompt` or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), plus `model` (`provider/model`), `cwd`, `agent`, and `iii_context` overrides.

## Events, usage, and cost

Every line OpenCode emits under `--format json` is mirrored verbatim onto `opencode::events`, group_id = session_id:

| OpenCode event | `agent::events` |
| --- | --- |
| `text` | `message_complete` |
| `tool_use` (completed) | `function_execution_start` + `function_execution_end` |
| `step_finish` | token usage + cost, surfaced on `agent_end` and the return value |
| stream end | `turn_end` + `agent_end` |

OpenCode reports per-step `tokens` (input / output / reasoning / cache read+write) and `cost` inline in the event stream, so `opencode::run` returns `usage` + `total_cost_usd` with no extra bookkeeping.

## The agent on the bus

By default every turn's prompt carries the iii runtime context: the engine-grounded discovery rules retargeted to the `iii` CLI, which OpenCode reaches through its own shell tool. The agent discovers capabilities from the live engine (`engine::functions::list`, `iii trigger <fn> --help`, the registry flow) instead of memory. Local file edits stay on OpenCode's native tools; backend actions go through registered functions. The context is prepended on a fresh session and skipped on resume; turn it off with `"iii_context": false` per call or in `config.yaml`.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                # empty = OpenCode default; else "provider/model"
  cwd: ""                  # default working directory for runs
  agent: ""                # OpenCode agent; empty = default

events_stream: agent::events       # translated AgentEvent frames
raw_events_stream: opencode::events  # verbatim OpenCode JSON events
iii_context: true                    # prepend the iii runtime context on a fresh session
opencode_executable: ""              # path to the opencode CLI; empty = resolve on PATH
```

`config.yaml` is the seed: on first boot the worker registers it with the built-in `configuration` worker as the initial value, then reads the live value back and hot-reloads on `configuration:updated`. `engine_url` stays on the local seed (bootstrap).

## How it maps

| OpenCode | iii |
| --- | --- |
| `opencode run --format json` turn | `opencode::run` invocation |
| each JSON event, verbatim | `opencode::events` stream frame |
| `text` part | `message_complete` frame on `agent::events` |
| `tool_use` part | `function_execution_start` / `function_execution_end` frames |
| `step_finish` tokens + cost | `usage` + `total_cost_usd` on the result + `agent_end` |
| session resume (`--session ses_...`) | engine state scope `opencode_sessions`, keyed by iii session_id |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |
