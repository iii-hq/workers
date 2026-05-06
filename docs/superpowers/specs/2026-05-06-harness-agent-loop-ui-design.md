# Harness agent-loop UI

**Status:** design
**Date:** 2026-05-06
**Scope:** make the harness web UI a real agent surface — actual tool calling, live event streaming, in-line user approvals.

## Problem

The harness composes ~22 workers and ships a React UI, but the UI's agent loop is half-broken:

1. Tool calling is disabled. `harness/web/src/App.tsx:300` sends `tools: []` with a TODO blaming Anthropic vs OpenAI schema drift. The whole `TOOLS` array in `App.tsx:30` is dead code today.
2. No streaming. UI calls `run::start_and_wait`, blocks for up to 240s, then renders a final transcript. Mid-turn tool calls are invisible while they happen.
3. Tool calls and tool results aren't rendered. `SessionView.blockText` filters to `type === "text"` only — `tool_use` and `tool_result` content blocks are dropped on the floor.
4. `<ApprovalRow>` doesn't exist. ARCHITECTURE.md §Trust boundary lists it as the third trust layer, but `harness/web/src/components/` has no such file.

Together: the UI promises an agent surface but delivers chat-with-stub-tools. This spec restores the missing pieces.

## Investigation findings

- Streaming infra already exists. `turn-orchestrator/src/events.rs` publishes a rich `AgentEvent` enum (`AgentStart/End`, `TurnStart/End`, `MessageStart/Update/End`, `ToolExecutionStart/Update/End`) to the `agent::events/<session_id>` stream via `stream::set`. The UI just doesn't subscribe.
- The schema "drift" comment is misleading. The canonical wire type is `AgentTool.parameters` (`turn-orchestrator/crates/harness-types/src/tool.rs:11`). `provider-anthropic/src/lib.rs:178` already converts `parameters` → `input_schema` internally for the Anthropic API. The UI just uses the wrong field name.
- `agent::before_tool_call` is the existing topic that gates tool dispatch. `policy-denylist` already subscribes to it as a synchronous gate; an approval worker plugs into the same surface.
- `harness/iii.worker.yaml` is missing today (per `harness/ARCHITECTURE.md:77`); the integration test fails. This spec restores it.

## Decisions (locked in during brainstorm)

- **Transport for live events: SSE.** New `GET /bridge/events?session_id=…` endpoint on `iii-harness`, native browser `EventSource`. Server-push, no new client deps, native reconnect with `Last-Event-ID`.
- **Loop driver: `run::start` + SSE-only.** UI fires `run::start` (returns immediately with `session_id`), then drives all UI state from the event stream. The transcript is reconstructed from `MessageEnd`/`AgentEnd` frames. `run::start_and_wait` stays in the bus for CLI consumers; the web UI no longer uses it. Removes the 240s timeout problem entirely — approvals can take as long as they need.
- **Approval gate: a dedicated `approval-gate` worker** subscribed to `agent::before_tool_call`. Mirrors `policy-denylist`'s wiring. Per-session approval list passed in the `run::start` payload; 5-minute auto-deny timeout.

## Architecture

```
                   browser
                      │
  POST /bridge/trigger│        GET /bridge/events?session_id=…
   (one-shot calls)   │           (SSE stream of AgentEvent frames)
                      ▼                       ▲
        ┌─────────────────────────────────────┴──────────┐
        │                iii-harness                     │
        │  - bridge::trigger          (existing)         │
        │  - GET /bridge/events       (NEW: SSE pump)    │
        └─────────────────────────────┬──────────────────┘
                                      │ stream::tail (poll cursor)
                                      │ stream::set   (write decisions)
                                      ▼
                        iii bus: agent::events/<sid>
                        ▲                          ▲
                        │ emits                    │ emits ApprovalRequested
   turn-orchestrator ───┘                          │   + reads decisions
                        │                          │
                        ├─→ agent::before_tool_call (topic)
                        │      ├─ policy-denylist (existing)
                        │      └─ approval-gate     (NEW)
```

Three new pieces; everything else reuses existing infrastructure.

## Components

### 1. `approval-gate` worker

New crate at `workers/approval-gate/`. Subscribes to `agent::before_tool_call`. Two registered functions for the UI: `approval::resolve` and `approval::list_pending`.

**On a `before_tool_call` event:**

1. Read `approval_required: string[]` from the topic payload. `turn-orchestrator/src/states/tools.rs` already publishes the tool call onto `agent::before_tool_call`; this spec adds the per-turn `approval_required` list (passed in `run::start`'s payload and persisted in the run request) to that topic frame so subscribers see it. If `tool_call.name` is not in the list → return `allow` immediately (no-op for that call).
2. Else: write `state::set scope=approvals key=<session>/<tool_call_id>` with `{tool_call_id, name, args, status: "pending", expires_at}`.
3. Emit `ApprovalRequested` onto `agent::events/<session_id>` so the UI sees it via SSE.
4. Loop on `state::get` with 250 ms backoff until `status` flips to `"allow"` or `"deny"`, or `expires_at` passes. On timeout → set `status="deny", reason="timeout"` and emit `ApprovalResolved`.
5. Return the block decision to the topic.

**Function: `approval::resolve`** — `{tool_call_id, decision: "allow" | "deny", reason?}`. Uses `state::update` with a conditional op (`set_if status == "pending"`); second writer for the same id gets `{ok: false, error: "already_resolved"}`. If the engine's `state::update` lacks conditional set, the implementation phase falls back to read-modify-write under a per-key sequence number — race tolerance verified in `approval-gate/tests/integration.rs`.

**Function: `approval::list_pending`** — `{session_id}` returns the currently-blocked calls. UI uses it on tab refresh to bootstrap before opening SSE.

**Failure mode:** any state-read error → log and return `deny` (fail closed). Idempotent on `tool_call_id` so an orchestrator-side retry of the topic can't double-prompt.

### 2. SSE bridge endpoint

New HTTP trigger in `iii-harness/src/lib.rs`: `GET /bridge/events?session_id=X`.

- Spawns a task that loops `iii.trigger("stream::tail", {stream_name: "agent::events", group_id: session_id, since_id: cursor, max_items: 50, block_ms: 5000})`.
- For each item writes `id: <item_id>\ndata: <json>\n\n` to the response body.
- On client disconnect (axum detects), drops the task.
- Heartbeat every 15 s with `: keepalive\n\n` to defeat reverse-proxy idle timeouts.

`Last-Event-ID` header on browser reconnect is read as the `since_id` cursor. No replay code lives in the JS client.

Same trust posture as `bridge::trigger` — single-tenant local install, no auth on the endpoint.

### 3. UI changes

**`App.tsx`:**

- `TOOLS[*].input_schema` → `parameters` (the only "schema unification" fix needed).
- `send()` calls `bridge("run::start", payload)` instead of `run::start_and_wait`. Returns immediately with `{session_id}`.
- New `useAgentStream(sessionId)` hook opens `EventSource` against `/bridge/events`, runs each frame through a pure reducer that returns `{messages, pendingApprovals, status}`.
- Stop sending `tools: []` — actually pass `TOOLS`.
- New per-session payload field: `approval_required: string[]`. Default: `["shell::filesystem::write", "shell::filesystem::mkdir"]`.

**New components in `harness/web/src/components/`:**

- `ToolUseBlock` — collapsible card for a `tool_use` content block. Header `tool · <name>`, body = pretty-printed JSON args. Collapsed by default.
- `ToolResultBlock` — `tool_result` rendering. Status pill (ok/error), truncated output with "show more". `is_error: true` → red border.
- `ApprovalRow` — pinned above the composer when `pendingApprovals.length > 0`. Shows tool name + args, two buttons: **allow** / **deny** (call `bridge("approval::resolve", …)`). Auto-disappears on `ApprovalResolved` frame.

**`SessionView`:** stop filtering to `text`-only. Render every block by type: `text` → `<p>`, `tool_use` → `ToolUseBlock`, `tool_result` → `ToolResultBlock`.

### 4. `harness-types` additions

Two new variants on `AgentEvent`:

```rust
ApprovalRequested {
    tool_call_id: String,
    tool_name: String,
    args: serde_json::Value,
    expires_at: u64, // unix ms
},
ApprovalResolved {
    tool_call_id: String,
    decision: String,        // "allow" | "deny"
    reason: Option<String>,
},
```

Additive — existing consumers ignore unknown tags via serde. Round-trip tests follow the pattern in `agent_event.rs`.

### 5. `iii.worker.yaml`

Restore the missing manifest at `harness/iii.worker.yaml`. Add `approval-gate` to the dependencies and to `EXPECTED_WORKERS` in `harness/src/lib.rs`. The existing `harness/tests/integration.rs` enforces the two stay aligned.

## Data flow — one full turn with approval

```
1. user types "create /tmp/notes.md with hello", clicks send
2. UI: bridge("run::start", {
       session_id, provider, model, messages, system_prompt,
       tools: TOOLS,
       approval_required: ["shell::filesystem::write"]
   }) → returns immediately with {session_id}

3. UI opens EventSource("/bridge/events?session_id=…")

4. turn-orchestrator runs the loop, emits in order:
       AgentStart
       MessageStart {role: "user", …}
       MessageEnd   {role: "user", …}
       TurnStart
       MessageUpdate × N        // streaming tokens of assistant reply
       MessageEnd   {role: "assistant", content: [text, tool_use{write…}]}
   tool dispatch fires `agent::before_tool_call`:
       - policy-denylist: not on denylist → allow
       - approval-gate:   name in approval_required → write pending state,
                          emit ApprovalRequested, block

5. UI sees ApprovalRequested → reducer puts it in pendingApprovals
   → ApprovalRow renders above composer

6. user clicks "allow"
   UI: bridge("approval::resolve", {tool_call_id, decision: "allow"})

7. approval-gate sees state flip → emits ApprovalResolved → returns allow
   to the topic → orchestrator resumes:
       ToolExecutionStart {tool_call_id, tool_name, args}
       ToolExecutionEnd   {tool_call_id, result, is_error: false}
       MessageStart {role: "tool", …}
       MessageEnd   {role: "tool", …}
       TurnStart … MessageEnd {role: "assistant", "done!"}
       TurnEnd
       AgentEnd

8. UI reducer collapses everything into the transcript; SessionView renders
   text + ToolUseBlock + (resolved/hidden) ApprovalRow + ToolResultBlock
   + final assistant text.
```

**Reconnect path** (SSE drops mid-step 4):

- Browser auto-reconnects with `Last-Event-ID: s0506-…-00000012`.
- Endpoint resumes `stream::tail since=…-00000012`.
- UI replays missed frames into the same reducer — idempotent because each `MessageEnd` overwrites by message id, and `ApprovalResolved` clears the pending entry by `tool_call_id`.

**Persistence:**

- Transcript: still written by `turn-orchestrator` to `state scope=agent key=session/<id>/messages` on every `MessageEnd` (unchanged).
- Approval state: `state scope=approvals key=<session>/<tool_call_id>`, with an `expires_at` checked by the gate's poll loop. Survives a UI refresh — on reload the UI calls `approval::list_pending` to bootstrap, then opens SSE.

**Concurrency caveat:** two browser tabs on the same session both render the same `ApprovalRow`. Whichever clicks first wins (state flip is an atomic CAS), the other tab sees `ApprovalResolved` and clears.

## Error handling

| Failure | Where caught | Behavior |
|---|---|---|
| Schema validation fails on `tool_use` | turn-orchestrator (existing) | Emits `ToolExecutionEnd {is_error: true}`. UI renders red `ToolResultBlock`. |
| Tool returns `is_error: true` | turn-orchestrator (existing) | Same — UI renders red. Model decides whether to retry. |
| `approval-gate` cannot read state | gate worker | Log and auto-deny (fail closed). Emit `ApprovalResolved {decision: "deny", reason: "state_unavailable"}`. |
| Approval times out (5 min) | gate worker | Auto-deny with `reason: "timeout"`. Tool returns blocked to the model; model explains and stops. |
| User clicks deny | UI → gate | Gate returns block. Orchestrator emits `ToolExecutionEnd {is_error: true, result: {content: [{type:"text", text:"denied by user"}]}}`. Loop continues; model sees the denial. |
| `approval::resolve` called with unknown `tool_call_id` | gate worker | `{ok: false, error: "not_found"}`. UI surfaces a toast. |
| SSE drops mid-turn | browser `EventSource` | Native auto-reconnect with `Last-Event-ID` → endpoint resumes from cursor. Reducer is idempotent. |
| `iii-harness` crashes mid-turn | turn-orchestrator (existing) | Already durable: `subscriber.rs` resumes from persisted `TurnState`. UI reconnects SSE on harness restart. |
| Tab refresh during pending approval | UI bootstrap | `approval::list_pending` → hydrate `pendingApprovals` → open SSE. |
| `run::start` returns error (auth missing, etc.) | UI `send()` | Surface in `app-error` (existing pattern). No SSE opened. |
| Unknown `AgentEvent` variant | UI reducer | Default branch: log to console, ignore. |
| Two tabs both `approval::resolve` the same call | gate worker | First write wins via state CAS. Second gets `{ok: false, error: "already_resolved"}`. |
| Empty `approval_required` list | gate worker | No-op for that turn — every call falls through to allow. Effectively opt-in. |

**Trust boundary** (matches `harness/ARCHITECTURE.md` §Trust boundary):

1. SDK wrapper allowlist on path args (existing).
2. `policy-denylist` blocks by name (existing).
3. `approval-gate` blocks pending UI confirmation (NEW — third layer ARCHITECTURE.md promised).

## Testing

**Unit (no engine):**

- `harness-types`: round-trip serde for `ApprovalRequested` and `ApprovalResolved`.
- `approval-gate` decision loop: in-memory state mock, table-driven cases — pending→allow, pending→deny, pending→timeout, unknown id, fail-closed on state error.
- UI event reducer: pure `(state, AgentEvent) → state`. Cases: out-of-order frames, duplicate `MessageEnd`, `ApprovalResolved` before its `Requested` (replay), unknown variant.

**Integration (one engine, in-process workers):**

- `approval-gate/tests/integration.rs`: spawn engine, register gate, fire `agent::before_tool_call` with `approval_required` set, assert it blocks; post `approval::resolve allow`, assert unblock under 500 ms.
- `harness/tests/sse_bridge.rs`: spawn harness, write three events to `agent::events/test`, open SSE, assert three frames arrive with monotonic `id:` lines. Write a fourth, disconnect, reconnect with `Last-Event-ID` of the third — assert only the fourth replays.
- `harness/tests/integration.rs` (existing): extend `EXPECTED_WORKERS` assertion to include `approval-gate`. Restore `iii.worker.yaml`.

**End-to-end (Playwright):**

- `harness/web/tests/e2e/approval.spec.ts`: start full demo via `scripts/demo.sh all`, drive UI to send "create /tmp/x.md", assert `ApprovalRow` appears within 3 s of the `tool_use` rendering, click allow, assert `ToolResultBlock` renders ok and the file exists. Second run: click deny, assert tool_result shows "denied by user" and the file does NOT exist.
- Reconnect smoke: kill the SSE socket mid-turn, assert UI reconnects within 5 s and the transcript matches.

**Coverage gates** (per CLAUDE.md): 80% on `approval-gate` Rust crate, 80% on the UI reducer (jest). E2E covers golden, deny, timeout (mocked clock), and reconnect paths.

**Out of scope:** streaming-token rendering polish (`MessageUpdate` is stored and overwritten on `MessageEnd`); multi-tab race beyond the unit test on `state::update` CAS.

## File-level inventory

**New:**

- `workers/approval-gate/Cargo.toml`
- `workers/approval-gate/iii.worker.yaml`
- `workers/approval-gate/src/lib.rs`
- `workers/approval-gate/src/main.rs`
- `workers/approval-gate/tests/integration.rs`
- `harness/web/src/useAgentStream.ts`
- `harness/web/src/reducer.ts` (+ `reducer.test.ts`)
- `harness/web/src/components/ToolUseBlock.tsx`
- `harness/web/src/components/ToolResultBlock.tsx`
- `harness/web/src/components/ApprovalRow.tsx`
- `harness/web/tests/e2e/approval.spec.ts`
- `harness/tests/sse_bridge.rs`
- `harness/iii.worker.yaml` (restore)

**Modified:**

- `turn-orchestrator/crates/harness-types/src/agent_event.rs` (two new variants + tests)
- `turn-orchestrator/src/run_start.rs` (carry `approval_required` into the persisted run request)
- `turn-orchestrator/src/states/tools.rs` (include `approval_required` in the `agent::before_tool_call` payload)
- `harness/src/lib.rs` (add `approval-gate` to `EXPECTED_WORKERS`; register `GET /bridge/events`)
- `harness/Cargo.toml` (axum SSE feature already pulled in by iii-sdk; verify)
- `harness/web/src/App.tsx` (run::start, useAgentStream, approval_required, parameters rename, pass tools)
- `harness/web/src/components/SessionView.tsx` (render all block types)
- `harness/tests/integration.rs` (extend manifest assertion)

**Deleted:** none.

## Open questions

None — all decisions locked in during brainstorm.
