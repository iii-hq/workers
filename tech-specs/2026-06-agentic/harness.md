# harness

Worker prefix: `harness::*`

## Definition

`harness` is the thin worker that wires the other three into an agent loop. It owns sequencing and
nothing else: take an incoming message, persist it, assemble a context, stream a completion, persist
the result, run any tool calls, and repeat until the turn stops.

It is deliberately minimal. The rule of thumb: **if a concern grows real logic, it becomes its own
worker** rather than living in the harness. Approval gating, spend budgets, compaction *scheduling*,
hook fan-out, and multi-agent handoff are all out of scope here — they are siblings the harness can
call or that can subscribe around it (see [Out of scope](#out-of-scope-future-sibling-workers)).

The harness is the only worker in this spec that depends on the other three, and even those are soft:
without `context-manager` it sends raw history; without tool functions it is a plain chat loop. It
needs `session-manager` (to persist/stream) and `llm-router` (to generate).

## What it wires

```mermaid
sequenceDiagram
  participant C as consumer (chat/tg)
  participant H as harness
  participant S as session-manager
  participant X as context-manager
  participant R as llm-router
  participant F as iii function (tool)

  C->>H: harness::send {message, model}
  H->>S: session::create or ensure
  H->>S: session::append (user message)
  H-->>C: {session_id, turn_id}
  Note over H: enqueue harness::turn (durable)
  H->>S: session::set_status working
  H->>S: session::messages
  H->>X: context::assemble
  H->>R: router::chat (over channel)
  R-->>H: AssistantMessageEvent frames
  H->>S: session::append (assistant) then session::update_message (stream deltas)
  alt assistant requested tool calls
    H->>F: iii.trigger(function_id, args)
    F-->>H: result
    H->>S: session::append (function_result)
    Note over H: re-enqueue harness::turn
  else no tool calls
    H->>S: session::set_status done
  end
```

## The loop

`harness::send` is the entry point; it ensures the session, persists the user message, and enqueues
the first `harness::turn` step, then returns immediately. The loop runs as durable enqueued steps so a
crash or restart resumes mid-turn. One `harness::turn` step does:

1. Mark working: `session::set_status working` (first step of a turn).
2. Load active path: `session::messages`.
3. Assemble context: `context::assemble` (skipped if `context-manager` absent -> raw messages + base
   system prompt). Tools come from the engine registry (see [Tools](#tools-the-white-box)).
4. Generate: open a channel, call `router::chat`; `session::append` an assistant message, then
   `session::update_message` as deltas arrive (each fires `session::message_updated`). Deltas may be
   batched to throttle update frequency; the final update writes the complete `AssistantMessage`.
5. If the message has `function_call` content: dispatch each via `harness::tool::dispatch`, append
   each `function_result`, then re-enqueue `harness::turn` to let the model react.
6. Else, steering check: re-read `session::messages` for any user message appended after this turn
   started (see [Triggers](#triggers)); if present, continue with another generate step; otherwise
   `session::set_status done` and stop.

A `max_turns` guard caps runaway loops. Cancellation is cooperative: `harness::stop` sets an abort
flag the next step checks, then sets `session::set_status done`.

The harness maps the turn lifecycle onto the session's coarse status: `working` while a turn is
running or awaiting tools, `done` once it stops. The internal `TurnStatus` (below) is finer-grained
and stays inside the harness; consumers watch the session status instead.

## Tools (the white box)

"Tools" are not a harness feature — they are the **iii substrate**. Any registered iii function is a
candidate tool. The harness:

- Discovers them via `engine::functions::list` and builds the `AgentFunction[]` tool schema
  (filtered by an allow-list/namespace policy supplied in `options`).
- Dispatches a model-chosen call with `iii.trigger({ function_id, payload })` via
  `harness::tool::dispatch`, capturing the result as a `function_result` message.

A tool can do anything an iii function can, including calling back to the consumer (the diagram's
`funcs -> chat` edge) — that is the tool's own behaviour, not the harness's.

## Functions

- `harness::send` — Entry point: persist the incoming message and kick off a turn; returns fast.
- `harness::turn` — Internal durable loop step (enqueued); not called directly by consumers.
- `harness::tool::dispatch` — Internal: invoke one iii function as a tool and capture its result.
- `harness::stop` — Request cancellation of an in-flight turn.
- `harness::status` — Read the current turn status for a session.

## Triggers

### Trigger types emitted

None. The harness drives itself through the durable queue (an invocation mode, not a trigger) and
relies on `session-manager` for consumer-facing reactivity.

### Triggers bound

- **Optional** `harness::on_steering` — bind to [`session::message_added`](session-manager.md#trigger-types-emitted)
  so a user message that arrives mid-turn is folded into the running turn rather than dropped. If a
  turn is already running, the loop's steering check (step 6) picks the new message up on its own; if
  none is running, the handler kicks a fresh turn:

```typescript
iii.registerFunction("harness::on_steering", async (evt) => {
  if (evt.message.role !== "user") return;
  const status = await iii.trigger({
    function_id: "harness::status",
    payload: { session_id: evt.session_id },
  });
  if (!status || status.status === "stopped" || status.status === "failed") {
    await iii.trigger({
      function_id: "harness::send",
      payload: { session_id: evt.session_id, message: evt.message, model: "<model>" },
    });
  }
  // else: a turn is running; its steering check re-reads session::messages and folds this in.
});

iii.registerTrigger({
  type: "session::message_added",
  function_id: "harness::on_steering",
  config: { roles: ["user"] },
});
```

This is opt-in; the default harness has no bound triggers.

---

## API Reference

Shared types (`AgentMessage`, `ContentBlock`, `AssistantMessage`, `AgentFunction`) are defined in
[README.md § Cross-cutting contracts](README.md#cross-cutting-contracts).

```typescript
type TurnStatus = "running" | "awaiting_tools" | "stopped" | "failed";
```

### `harness::send`

Accept an incoming message, ensure the session, append the user message, and enqueue the first turn
step. Returns before the turn runs.

- Invocation: **sync** (kicks an async/enqueued loop)

Request:

```typescript
type SendRequest = {
  session_id?: string;            // omit to create a new session
  message: AgentMessage | string; // string is sugar for a user text message
  model: string;
  provider?: string;
  options?: {
    system_prompt?: string;
    max_turns?: number;           // default 16
    thinking_level?: string;
    tools?: {
      allow?: string[];           // function_id globs to expose (e.g. "shell::*")
      deny?: string[];
    };
    metadata?: Record<string, unknown>; // tracing passthrough (session_id/message_id propagate)
  };
};
```

Response:

```typescript
type SendResponse = {
  session_id: string;
  turn_id: string;     // identifies this loop invocation
  accepted: true;
};
```

Example:

```jsonc
// request
{ "message": "Summarise the repo README", "model": "claude-sonnet-4", "provider": "anthropic",
  "options": { "tools": { "allow": ["shell::*", "fs::*"] } } }
// response
{ "session_id": "s_7a1", "turn_id": "t_001", "accepted": true }
```

### `harness::turn`

Internal durable loop step. Documented for completeness; consumers do not call it. Enqueued onto a
FIFO `harness-turn` queue; each run advances one step of [the loop](#the-loop).

- Invocation: **enqueue** (`TriggerAction.Enqueue({ queue: "harness-turn" })`)

```typescript
type TurnStepPayload = {
  session_id: string;
  turn_id: string;
  step: number;          // monotonic; guards against stale/duplicate dequeues
};
type TurnStepResult = {
  session_id: string;
  status: TurnStatus;
  next_step?: number;    // present while the loop continues
};
```

Failure handling: an unexpected throw marks the turn `failed` and appends a `custom`
(`custom_type: "error"`) entry so the UI sees the reason. A step may opt into queue retry/backoff for
transient provider errors instead of failing the turn.

### `harness::tool::dispatch`

Invoke a single iii function as a tool and return a normalised result. Internal to the loop but also
callable directly to execute a one-off tool with the same result shape.

- Invocation: **sync**

```typescript
type ToolDispatchRequest = {
  session_id: string;
  call: {
    id: string;            // function_call id, echoed into the result
    function_id: string;   // the iii function to invoke
    arguments: unknown;
  };
};
type ToolDispatchResponse = {
  function_call_id: string;
  function_id: string;
  content: ContentBlock[]; // tool output, normalised to content blocks
  is_error: boolean;
  details?: unknown;
  duration_ms: number;
};
```

The harness performs no approval check here in v1; gating is delegated to an optional approval sibling
that can wrap or precede dispatch (see [Out of scope](#out-of-scope-future-sibling-workers)).

### `harness::stop`

Request cancellation. Sets an abort flag the next `harness::turn` step observes; an in-flight provider
stream is closed best-effort.

- Invocation: **sync**

```typescript
type StopRequest = { session_id: string; turn_id?: string }; // turn_id omitted = current turn
type StopResponse = { stopping: boolean };
```

### `harness::status`

- Invocation: **sync**

```typescript
type StatusRequest = { session_id: string };
type StatusResponse = {
  session_id: string;
  turn_id: string | null;
  status: TurnStatus;
  step: number;
  turn_count: number;
  max_turns: number;
  pending_tool_calls: string[];   // function_call ids awaiting results
} | null;                          // null for unknown sessions
```

---

## State

| Scope | Key | Value | Purpose |
|---|---|---|---|
| `harness_turn` | `<session_id>` | turn record `{ turn_id, status, step, turn_count, abort? }` | Loop progress; survives restart. |

Transcript truth lives in [session-manager](session-manager.md); the harness keeps only loop
bookkeeping.

## Dependencies

- `session-manager` (`session::*`) — persist messages, stream content via `session::update_message`,
  and set status. Required.
- `llm-router` (`router::chat`) — generation. Required.
- `context-manager` (`context::assemble`) — context budgeting. Soft; degrades to raw history.
- `iii-queue` — the durable `harness-turn` loop.
- iii engine `engine::functions::list` + `iii.trigger` — tool discovery and dispatch.

## Out of scope (future sibling workers)

Kept out to preserve thinness; each is a clean add-on that wraps the loop or subscribes to its events:

- **approval-gate** — intercept `harness::tool::dispatch` (or a pre-dispatch hook) to allow / deny /
  hold tool calls against a policy; resume via a state trigger.
- **llm-budget** — track spend from `router` usage and cap per workspace/agent.
- **context-scheduler** — decide *when* to compact (the optional reactive trigger in
  [context-manager](context-manager.md#triggers)); the harness only compacts inline on overflow.
- **hook-fanout** — publish-and-collect lifecycle hooks around turns/tools.
- **multi-agent / orchestrator** — agent-to-agent handoff via queues; the harness runs one agent
  loop.

If any of these needs to sit *inside* the critical path, prefer a pre/post hook function id the
harness calls (config-driven) over embedding the logic.

## Boundaries

- Does **not** store the transcript, build context, or talk to providers itself — it calls the other
  three.
- Does **not** gate approvals, meter cost, or schedule compaction in v1.
- Does **not** define tools — tools are any iii function; the harness only discovers and dispatches
  them.
