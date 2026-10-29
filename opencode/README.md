# opencode

OpenCode as an iii worker: the OpenCode API exposed as functions on the iii bus, nothing else. The worker spawns the same `opencode` CLI the user runs in their terminal, with the same login, filesystem, and tools. `opencode::run` executes one headless turn (`opencode run --format json`) and returns the result with token usage and cost; the raw JSON events are delivered verbatim on the `opencode::on-raw-event` function, and a translated AgentEvent view on `opencode::on-agent-event`, so the acp worker and any sibling worker that observes them follows an OpenCode run exactly like a native harness turn. The worker also registers `run::start_and_wait`, the same entrypoint the console and acp worker drive.

## Install

```bash
iii call compose::add worker=opencode
```

Requires the `opencode` CLI on the host (`npm i -g opencode-ai` or the official installer) and an API key for the LLM provider you use — `ANTHROPIC_API_KEY` (or any provider key) in the worker environment, or an existing `opencode auth` login.

## Skills

```bash
npx skills add iii-hq/workers --skill opencode
```

## Quickstart

```bash
curl -fsSL https://install.iii.dev/iii/main/install.sh | sh
iii call compose::add worker=opencode
iii   # starts the engine + worker
```

Then drive it from the console chat, `iii call`, or any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.call({
  function_id: 'opencode::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
    model: 'anthropic/claude-sonnet-4-5',
  },
  timeoutMs: 600_000,
});
// { session_id, opencode_session_id, result, stop_reason, usage, total_cost_usd }
```

Or from the terminal:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii call opencode::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads
iii call opencode::sessions::list
iii call opencode::status session_id=<session_id>

# background turn + control
iii call opencode::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii call opencode::stop session_id=<session_id>

iii call opencode::run --help
```

A turn from the CLI returns the result with token usage and cost, and `opencode::run --help` prints the published request schema as a parameter table:

![opencode::run returning pong with usage and cost, after engine::functions::list shows the registered surface](https://raw.githubusercontent.com/iii-hq/workers/main/opencode/assets/cli-run.png)

![iii call opencode::run --help printing the request schema as a parameter table](https://raw.githubusercontent.com/iii-hq/workers/main/opencode/assets/cli-help.png)

**Resume uses the iii `session_id`, not `opencode_session_id`.** Two ids come back from every run: `session_id` (a UUID) is the iii key for resume / status / stop and the `session_id` event feeds are bound with; `opencode_session_id` (`ses_...`) is OpenCode's own id, returned for reference only. To continue a conversation, pass the same **iii `session_id`** again. The worker looks up the stored OpenCode session for that key and resumes with `--session`:

![two opencode::run turns sharing the iii session_id; the second resumes (num_turns 2) and recalls a token from the first](https://raw.githubusercontent.com/iii-hq/workers/main/opencode/assets/cli-resume.png)

Long turns: use `opencode::start` to return immediately, then observe `opencode::on-agent-event` with args `{ session_id }` (see [Event feeds](#event-feeds)) for `message_complete`, `function_execution_start/end`, and `turn_end`. `opencode::stop` interrupts a live run.

## Functions

| Function | Purpose |
| --- | --- |
| `opencode::run` | Run one turn, wait, return the final result + usage + cost |
| `opencode::start` | Fire-and-forget turn; progress arrives on `opencode::on-agent-event` |
| `opencode::stop` | Interrupt a live run |
| `opencode::status` | Session state, live flag, usage, cost |
| `opencode::sessions::list` | All sessions this worker has run |
| `run::start_and_wait` | Alias for `opencode::run` under the shared agent entrypoint |

`opencode::run` accepts a bare `prompt` or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`), plus `model` (`provider/model`), `cwd`, `agent`, and `iii_context` overrides.

## Events, usage, and cost

Every line OpenCode emits under `--format json` is delivered verbatim on `opencode::on-raw-event`, and translated onto `opencode::on-agent-event`:

| OpenCode event | `opencode::on-agent-event` |
| --- | --- |
| `text` | `message_complete` |
| `tool_use` (completed) | `function_execution_start` + `function_execution_end` |
| `step_finish` | token usage + cost, surfaced on `agent_end` and the return value |
| stream end | `turn_end` + `agent_end` |

OpenCode reports per-step `tokens` (input / output / reasoning / cache read+write) and `cost` inline in the event stream, so `opencode::run` returns `usage` + `total_cost_usd` with no extra bookkeeping.

## Event feeds

The worker hosts two event feeds, private functions it calls once per frame:
only this worker calls them, and anyone may observe them.

| Function | Args | Delivery | `event` in the frame |
| --- | --- | --- | --- |
| `opencode::on-agent-event` | `{ session_id }` | iii, to the observers of that session | one translated AgentEvent frame (`message_complete`, `function_execution_start/end`, `turn_end`, `agent_end`) |
| `opencode::on-raw-event` | `{ session_id }` | iii, to the observers of that session | one raw OpenCode JSON event, verbatim (`step_start`, `text`, `tool_use`, `step_finish`) |

`session_id` is the iii session id: required, non-empty and at most 512
characters; the engine rejects any other arg with `INVALID_ARGS`. Each feed
accepts at most 256 observers; one more is rejected with `OBSERVER_LIMIT`. A
frame of a session nobody observes is not sent.

```ts
iii.observe(
  { function_id: 'opencode::on-agent-event', args: { session_id }, deliver: 'output' },
  'my::on-opencode-event',
  { metadata: { team: 'a' } }, // optional: handed to your function as the metadata sidecar
);
```

With `deliver: 'output'` your function receives one call per frame
(fire-and-forget):

```jsonc
{
  "session_id": "<iii session_id>",
  "event_id": "<session_id>-<epoch>-00000007", // identity, for dedup
  "seq": 7,              // contiguous per (feed, session_id, epoch), from 0
  "epoch": "<uuid>",     // per worker process; a new epoch restarts seq at 0
  "source": "opencode",
  "event": { "type": "message_complete", "...": "..." }
}
```

The two feeds number their frames independently, and `seq` advances for every frame, observed or not. Deliveries are not guaranteed to arrive in order, so order by `(epoch, seq)` and drop repeated `event_id`s. Frames are ephemeral: nothing is stored or replayed, and an observer added after a frame was emitted never sees it. The durable record of a turn is the value `opencode::run` returns and the session record behind `opencode::status` / `opencode::sessions::list`. The feeds do not use iii-stream (see the guide "Migrate from iii-stream and pubsub"). The acp worker observes `opencode::on-agent-event` when it drives this worker.

## The agent on the bus

By default every turn's prompt carries the iii runtime context: the engine-grounded discovery rules retargeted to the `iii` CLI, which OpenCode reaches through its own shell tool. The agent discovers capabilities from the live engine (`engine::functions::list`, `iii call <fn> --help`, the registry flow) instead of memory. Local file edits stay on OpenCode's native tools; backend actions go through registered functions. The context is prepended on a fresh session and skipped on resume; turn it off with `"iii_context": false` per call or in `config.yaml`.

OpenCode answers by querying the live engine itself, grouping every connected worker by runtime:

![opencode::run enumerating the workers connected to the engine by running iii call engine::workers::list](https://raw.githubusercontent.com/iii-hq/workers/main/opencode/assets/cli-discovery.png)

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                # empty = OpenCode default; else "provider/model"
  cwd: ""                  # default working directory for runs
  agent: ""                # OpenCode agent; empty = default

iii_context: true                    # prepend the iii runtime context on a fresh session
opencode_executable: ""              # path to the opencode CLI; empty = resolve on PATH
```

`config.yaml` is the seed: on first boot the worker registers it with the built-in `configuration` worker as the initial value, then reads the live value back and hot-reloads on `configuration:updated`. `engine_url` stays on the local seed (bootstrap). The event feeds are fixed functions, not configuration; the former `events_stream` / `raw_events_stream` keys are accepted and ignored so stored configs keep loading.

## How it maps

| OpenCode | iii |
| --- | --- |
| `opencode run --format json` turn | `opencode::run` invocation |
| each JSON event, verbatim | `opencode::on-raw-event` frame |
| `text` part | `message_complete` frame on `opencode::on-agent-event` |
| `tool_use` part | `function_execution_start` / `function_execution_end` frames |
| `step_finish` tokens + cost | `usage` + `total_cost_usd` on the result + `agent_end` |
| session resume (`--session ses_...`) | engine state scope `opencode_sessions`, keyed by iii session_id |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |
