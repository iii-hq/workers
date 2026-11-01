# grok

The [xAI Grok CLI](https://docs.x.ai) as an iii worker: the Grok agent exposed as functions on the iii bus, nothing else. The worker spawns the same `grok` binary the user runs in their terminal, with the same `XAI_API_KEY`, the same filesystem, and the same working directory. `grok::run` executes one headless turn (`grok --single <prompt> --output-format streaming-json`) and returns the result; the raw Grok events are delivered verbatim on `grok::on-raw-event`, and a translated AgentEvent view on `grok::on-agent-event`, so the acp worker and any sibling worker observe a Grok run exactly like a native harness turn.

## Install

```bash
iii call compose::add worker=grok
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
iii call compose::add worker=grok
iii   # starts the engine + worker
```

Then talk to it like any other function: from `iii call grok::run`, or from any SDK:

```ts
import { registerWorker } from 'iii-sdk';

const iii = registerWorker('ws://127.0.0.1:49134', { workerName: 'demo' });

const res = await iii.call({
  function_id: 'grok::run',
  payload: {
    prompt: 'Add a /health endpoint to server.ts and run the tests',
    cwd: '/path/to/repo',
  },
  timeout_ms: 600_000,
});
// { session_id, grok_thread_id, result, stop_reason, num_turns }
```

Or straight from the terminal with the `iii call` CLI:

```bash
# one full turn (raise the timeout; the default 30s is too short for agent turns)
iii call grok::run --timeout-ms 600000 \
  --json '{"prompt":"add a /health endpoint and run the tests","cwd":"/path/to/repo"}'

# quick reads use key=value syntax
iii call grok::sessions::list
iii call grok::status session_id=<session_id>

# background turn + control
iii call grok::start --json '{"prompt":"...","cwd":"/path/to/repo"}'
iii call grok::stop session_id=<session_id>

# ask the running engine for a function's description and parameter table
iii call grok::run --help
```

A turn from the CLI, and the published request schema served by `--help`:

![iii call grok::run returning the result over the bus](https://raw.githubusercontent.com/iii-hq/workers/main/grok/assets/cli-run.png)

![iii call grok::run --help printing the typed request schema as a parameter table](https://raw.githubusercontent.com/iii-hq/workers/main/grok/assets/cli-help.png)

Call `grok::run` again with the returned `session_id` to continue the same conversation: the worker maps iii session ids to Grok session ids in engine state and resumes automatically (sessions persist in `~/.grok/sessions`).

Two ids come back from every run. `session_id` is the iii session id: the key for `grok::status`, `grok::stop`, resume, and the event feeds' `session_id` arg. `grok_thread_id` is Grok's own session id (what the worker passes to `--resume` on the next turn) — returned for reference, not a lookup key.

## Functions

| Function | Purpose |
| --- | --- |
| `grok::run` | Run one turn, wait, return the final result |
| `grok::start` | Fire-and-forget turn; progress arrives on the event feeds |
| `grok::stop` | Interrupt a live run |
| `grok::status` | Session state, live flag, turn count |
| `grok::sessions::list` | All sessions this worker has run |
| `grok::on-agent-event` | Event feed: translated AgentEvent frames of one session (args `{ session_id }`, see [Event feeds](#event-feeds)) |
| `grok::on-raw-event` | Event feed: raw streaming-json events of one session (args `{ session_id }`) |
| `grok::on-config-change` | Internal: observes `configuration::on-updated` and reloads the configuration |

`grok::run` accepts either a bare `prompt` string or a `messages` array (`[{ role: 'user', content: [{ type: 'text', text }] }]`) — the same input contract as the claude-code worker and `run::start_and_wait`, so the acp worker drives Grok with `--brain-fn grok::run` — plus `model`, `cwd`, `always_approve`, and `iii_context`.

### Raw events

Every line Grok emits on its `--output-format streaming-json` stream is delivered verbatim on `grok::on-raw-event`. Consumers that want the exact Grok wire format observe `grok::on-raw-event`; consumers that want harness-shaped frames observe `grok::on-agent-event`. Same turn, two views (see Event feeds below).

The streaming-json stream (captured from Grok CLI 0.2.77) is delta-based: assistant text arrives as `{"type":"text","data":"<chunk>"}` lines, the turn closes with `{"type":"end","stopReason","sessionId","requestId"}`, and failures arrive as `{"type":"error","message"}`. The worker accumulates the text deltas and emits one `message_complete` frame on `grok::on-agent-event` at `end`.

> Note: the Grok CLI streaming-json schema is not formally published, so the typed model in [`src/grok/events_types.rs`](src/grok/events_types.rs) is lenient — unrecognized event types pass through verbatim on `grok::on-raw-event` and are skipped on the translated feed rather than failing the turn. Headless output carries no token usage and does not break out tool-call events today.

## Event feeds

The worker provides two functions that carry one session's frames. Observe one with `{ session_id }` as args to run a function of your own on that session's frames:

| Function | Args | Carries |
| --- | --- | --- |
| `grok::on-agent-event` | `{ session_id }` | translated AgentEvent frames (`message_complete`, `turn_end`, `agent_end`), what the acp worker renders |
| `grok::on-raw-event` | `{ session_id }` | every streaming-json line, verbatim |

```ts
iii.registerFunction('my-app::on-grok-frame', async (frame) => {
  // frame: { session_id, event_id, seq, epoch, source, event }
})

iii.observe(
  { function_id: 'grok::on-agent-event', args: { session_id: 'sess-1' }, deliver: 'output' },
  'my-app::on-grok-frame',
  { metadata: { conn: 'c1' } }, // optional: reaches your function as its metadata
)
```

`session_id` is required (non-empty, at most 512 characters) and is the only arg; the engine rejects any other with `INVALID_ARGS`. Each function accepts at most 256 observers, and one more is rejected with `OBSERVER_LIMIT`. Both functions are private: only this worker calls them, and any worker may observe them.

Each frame is one fire-and-forget call of the function, and the event it returns is what its session's observers observe (with `deliver: 'output'`, your function receives it as its input):

```json
{
  "session_id": "sess-1",
  "event_id": "sess-1-<epoch>-00000002",
  "seq": 2,
  "epoch": "<uuid>",
  "source": "grok",
  "event": { "type": "text", "data": "Hello" }
}
```

`seq` counts from 0 per feed and session within one `epoch` (a uuid minted per worker process, so a restart starts a new epoch); the two feeds count independently, and a frame of a session nobody observes still takes its `seq`. Deliveries may arrive out of order: order by `(epoch, seq)` and dedup by `event_id`. The feeds are ephemeral: nothing is stored or replayed, and an observer added mid-turn only sees later frames. History lives in the session record (`grok::status`, `grok::sessions::list`) and in the `grok::run` result. A failing consumer never fails the turn.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""             # empty = Grok CLI default (e.g. grok-build-0.1)
  cwd: ""               # default working directory for runs
  always_approve: true  # auto-approve tool/command execution on headless turns

grok_executable: ""              # path to the grok CLI; empty = PATH resolution
```

Auth is the Grok CLI's own: the worker inherits `XAI_API_KEY` from its environment. `always_approve` keeps headless turns from blocking on an interactive approval prompt; set it to `false` to let the CLI's approval policy gate tool execution.

## The agent on the bus

By default every turn carries the iii runtime context, prepended to the prompt on the first turn of a session: the same engine-grounded rules as the harness identity prompts, retargeted to the `iii` CLI the agent reaches through its shell. The agent discovers capabilities from the live engine instead of memory — `iii call engine::functions::list` to find function ids, `iii call <fn> --help` as the contract before every first call, the registry flow (`directory::registry::workers::list/info`, `worker::add`) when nothing registered fits — plus the calling rules and error-handling discipline that go with them. Local file edits stay on Grok's native tools; backend actions go through registered functions.

```bash
# the agent answers this by querying the live engine itself
iii call grok::run --timeout-ms 300000 \
  --json '{"prompt":"List every worker connected to this engine and what each one does.","cwd":"/tmp"}'
```

With the context on, Grok reaches for `iii call engine::workers::list` itself and reports the live iii worker functionality — every worker and what it does — instead of guessing from `ps` and config files:

![Grok answering a live-system question by querying the engine through the iii CLI](https://raw.githubusercontent.com/iii-hq/workers/main/grok/assets/iii-context.png)

The context is injected once at the start of a session; resumed turns rely on the existing session history. Turn it off per call with `"iii_context": false` or globally in `config.yaml`.

## Observability

Every `grok::run` is an ordinary traced invocation on the engine: the trace carries the full input payload and the output (result, stop reason) as span events, with per-function p50/p95/p99 in the console's trace explorer — no extra instrumentation in the worker. Headless Grok output carries no token usage today, so the worker does not report it.

## How it maps

| Grok | iii |
| --- | --- |
| one headless `grok --single` turn | `grok::run` invocation |
| every streaming-json line, verbatim | `grok::on-raw-event` call |
| accumulated `text` deltas at `end` | `message_complete` frame on `grok::on-agent-event` |
| turn `end` | `turn_end` + `agent_end` frames, function return value |
| `end.sessionId` → `--resume` next turn | engine state scope `grok_sessions`, keyed by iii session_id |
| extra capability | another iii worker on the bus (`shell`, `database`, `storage`, ...) |
