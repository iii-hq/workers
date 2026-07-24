# harness architecture

Reference documentation for the `harness` worker — the durable, resumable
agent turn loop specified in
[tech-specs/2026-06-agentic/harness.md](../../tech-specs/2026-06-agentic/harness.md).
These documents are written to be sufficient on their own: a reader (human or
LLM) should be able to integrate against the harness without opening the
source. The golden schemas under
[../tests/golden/schemas/](../tests/golden/schemas) are the wire truth; this
prose explains how to use them.

## Document map

| Document | Audience | Read it when |
|---|---|---|
| [integration.md](integration.md) | Authors of consumers / siblings | You are building something on top of the harness — a chat UI, a Telegram / WhatsApp / Slack bridge, a cron or webhook worker, an event-driven agent loop, a notification sibling. This file is the handoff contract. |

The worker [README](../README.md) is the operator how-to (install, config,
quickstart). The spec
[harness.md](../../tech-specs/2026-06-agentic/harness.md) is the design of
record — deeper on the loop mechanics (durability, idempotency, steering,
hooks) than the integration contract needs to be.

## The system in one paragraph

The harness is the thin worker that wires the other three into an agent loop:
take an incoming message, persist it, assemble a context, stream a completion,
run any function calls the model requests, and repeat until the turn stops —
all as durable, enqueued steps so a crash or restart resumes mid-turn. It owns
sequencing and nothing else. Consumers are deliberately thin: they **kick off**
a turn (`harness::send` / `harness::run`), **render** the conversation by
binding `session-manager`'s transcript events, and **react** to boundaries
(`harness::turn-completed`) and human-gated calls (approval-gate's
`approval::*` triggers). There is no `agent::events` firehose — the transcript
is the stream, and turn lifecycle rides discrete triggers.

## The system in one diagram

```mermaid
flowchart LR
  consumer["consumer<br/>(chat UI / bridge / cron)"]
  subgraph harness [harness]
    entry["send / run / spawn"]
    loopStep["turn (durable step)"]
    events["turn-started / turn-completed"]
    hooks["hook::* (sync)"]
  end
  session["session-manager"]
  ctx["context-manager"]
  router["llm-router"]
  gate["approval-gate"]
  fns["iii functions"]

  consumer -->|"trigger harness::send / run"| entry
  entry --> loopStep
  loopStep -->|"append / update-message"| session
  loopStep -->|"assemble"| ctx
  loopStep -->|"chat"| router
  loopStep -->|"dispatch (agent_trigger)"| fns
  loopStep --> events
  loopStep -.->|"pre-trigger hold"| hooks
  hooks -.-> gate
  session -.->|"message-* / status-changed"| consumer
  events -.->|"turn-completed"| consumer
  gate -.->|"pending-created / resolved"| consumer
```

## Vocabulary

| Term | Meaning |
|---|---|
| **Turn** | One run of the loop for a session: one or more generate steps until the model stops, with a coarse `TurnStatus` (`running` / `awaiting_functions` / `completed` / `cancelled` / `failed`). |
| **Step** | One durable, enqueued `harness::turn` iteration: assemble → generate → dispatch. Re-enqueued until the turn finalises. |
| **Steering / merge** | A `harness::send` for a session that already has a running turn folds the new message into it instead of starting a second turn (the response carries `merged: true`). |
| **Dispatch policy** | The fail-closed `options.functions.allow` / `deny` globs deciding which functions the model may call. Absent or empty `allow` → a plain chat loop. Ask-mode turns are additionally capped at the configured default policy (`default_functions`). |
| **Exposure mode** | How allowed functions reach the model: one generic `agent_trigger` schema (default) or one schema per allowed function (`native`). |
| **Output contract** | The turn's deliverable: free `text` (default) or `json` validated against a schema; the result rides `harness::turn-completed` and `harness::run`. |
| **Hook** | A synchronous extension point (`harness::hook::*`) a sibling binds to veto / hold / mutate in-path. Hook *logic* lives in the sibling, never the harness. |
| **Pending / parked** | An approval/hook hold checkpoints `pending`; the turn parks and `harness::function::resolve` resumes it later. Delegation never parks; approvals still can. |
| **Sub-agent** | A child turn in a child session, started fire-and-forget by `harness::spawn` from a parent turn. Its completion is observable only via its `harness::turn-completed` event and the state it writes — the result is never delivered back to the caller. |
| **Terminal turn** | `harness::turn-completed` carries `terminal: bool` — `false` while the session still owns an armed wake (a one-shot notify subscription), meaning a later turn in the same session carries the real outcome. Consumers finalize a logical exchange only on `terminal: true`. |
