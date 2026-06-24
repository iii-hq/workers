# hermes

Hermes agent as an iii worker: the Hermes agent on the iii bus as functions, streams, and a trigger. `hermes::run` runs one headless Hermes turn carrying the iii runtime context, so the agent discovers and drives the whole engine live. `hermes::send` delivers to any of Hermes's 27+ messaging platforms (Telegram, Discord, Slack, WhatsApp, Teams, …). Inbound platform messages and webhook events land on the worker's HTTP sink and republish so any iii worker can react. The result: iii gains omnichannel reach wired to its entire function registry, with an iii-aware agent in between.

## Install

```bash
iii worker add hermes
```

Container worker (`deploy: image`) — the image bundles the Hermes CLI (Python 3.11 + Node bridge + platform adapters) via the official installer. Hermes is provider-agnostic: configure any LLM provider non-interactively with `hermes auth add anthropic --api-key <key>` (or set `ANTHROPIC_API_KEY` + `HERMES_INFERENCE_MODEL=anthropic/...`). The interactive OAuth-via-portal flow is not needed.

## What it exposes

| Function | Purpose |
| --- | --- |
| `hermes::run` | Run one Hermes turn, wait, return the final result (carries the iii runtime context) |
| `hermes::send` | Deliver a message to a gateway platform — omnichannel out |
| `hermes::sessions::list` | Sessions this worker has run, plus raw `hermes sessions list` |
| `hermes::status` | Point-in-time session state, live flag |
| `hermes::stop` | Interrupt a live run |
| `run::start_and_wait` | Alias for `hermes::run` under the shared agent entrypoint |

Trigger sink (HTTP): `hermes::inbound` — the Hermes gateway delivers inbound platform/webhook events here; the worker republishes them for other workers to react to.

## Quickstart

```bash
# one turn (the agent has the iii runtime context by default)
iii trigger hermes::run --timeout-ms 600000 \
  --json '{"prompt":"List every worker connected to this engine and what each one does.","cwd":"/tmp"}'

# omnichannel out
iii trigger hermes::send --json '{"platform":"telegram","message":"deploy finished"}'

iii trigger hermes::sessions::list
iii trigger hermes::run --help
```

Pass the same `session_id` again to continue a conversation: the worker threads it to Hermes as `hermes -z --resume <session_id>`, so the first call creates that session and later calls continue it. (Hermes also keeps its own cross-session memory, so some facts persist beyond a single session regardless.)

## The agent on the bus

By default every turn prepends the iii runtime context: the engine-grounded discovery rules retargeted to the `iii` CLI, which Hermes reaches through its own shell / `execute_code` tool. The agent discovers capabilities from the live engine (`engine::functions::list`, `<fn> --help`, the registry flow) instead of memory. Disable per call with `"iii_context": false`, or globally in `config.yaml`.

## Omnichannel front door

The unique value over the other agent workers: Hermes is iii's gateway to 27+ chat platforms.

```
27 platforms ─▶ Hermes gateway ─▶ hermes::inbound (iii-http) ─▶ republished
                                                                    │ iii worker reacts,
                                                                    │ drives the bus
                                                                    ▼
                                                       hermes::send --to <platform> ─▶ out
```

Point a Hermes webhook route / delivery target at the worker's `inbound_api_path` (default `/hermes/inbound`, served via iii-http). An inbound message flows in, an iii worker handles it against the full function registry, and the reply goes back out through `hermes::send`.

## Configuration

```yaml
engine_url: ws://127.0.0.1:49134

defaults:
  model: ""                 # HERMES_INFERENCE_MODEL value; empty = Hermes default
  cwd: ""

events_stream: agent::events       # translated AgentEvent frames
raw_events_stream: hermes::events   # raw Hermes run output
iii_context: true                   # prepend the iii runtime context on a fresh session
hermes_executable: ""               # path to the hermes CLI; empty = resolve on PATH
inbound_api_path: /hermes/inbound   # iii-http path the gateway delivers inbound events to
```

## Scope

Exposes the agent loop, omnichannel send, and inbound events — not Hermes's duplicate tools (shell/code/web/file, which iii already has as `shell`, `coder`, `web`) and not MCP. iii capabilities reach the agent through the injected context + Hermes's own shell.

## Status

Foundation. The agent loop (`hermes::run` via `hermes -z`), `hermes::send`, sessions, and the iii context are wired against the documented Hermes CLI. Two parts finalize against a live Hermes instance with credentials:

- **Events granularity** — Hermes one-shot returns only final text, so `agent::events` carries `turn_end` + `agent_end` (the final message), not per-tool frames. A richer event stream depends on a Hermes mode that exposes one.
- **Inbound trigger contract** — `hermes::inbound` republishes raw deliveries today; the exact gateway payload is mapped onto a dedicated `hermes::message` trigger type once verified against a running gateway.
