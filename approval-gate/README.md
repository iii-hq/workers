# approval-gate

Rules-driven safety gate for LLM-initiated function calls. Subscribes to `agent::before_function_call`; for every call it consults a first-match ruleset and replies on the hook envelope with one of three verdicts: **Allow** → pass through, **Deny** → structured `ApprovalRuleDenied`, **Ask** → write a Pending record and pause. Operators resolve pending rows via `approval::resolve`; resolved rows are stitched into the next assistant turn via `approval::consume`. A built-in watchdog (`approval::tick_timeouts`) fires on a configurable interval so expired Pending rows and stale InFlight rows are reclaimed and the orchestrator gets woken via `run::resume`.

`approval_required` from the run-request payload is tolerated for mixed deployments but ignored — policy decisions come entirely from `rules:`.

## Install

```bash
iii worker add approval-gate
```

`iii worker add` fetches the binary, writes `config.yaml` defaults into `~/.iii/config.yaml`, and the engine starts the worker on the next `iii start`.

## Quickstart

Approval traffic is normally published by `provider-router` / `turn-orchestrator`. To confirm the gate is connected, target the registered handler directly (dev only — production traffic comes from the orchestrated hook envelope):

```rust
use iii_sdk::{register_worker, InitOptions, TriggerRequest};
use serde_json::json;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let iii = register_worker("ws://127.0.0.1:49134", InitOptions::default());

    let envelope = json!({
        "event_id": "evt-check",
        "reply_stream": "rs-check",
        "payload": {
            "session_id": "sess-dev",
            "function_call": {
                "id": "tc-dev",
                "function_id": "shell::fs::write",
                "arguments": {},
            }
        }
    });

    let result = iii
        .trigger(TriggerRequest {
            function_id: "policy::approval_gate".into(),
            payload: envelope,
            action: None,
            timeout_ms: Some(240_000),
        })
        .await?;

    println!("{result:#?}");
    Ok(())
}
```

Release the latch with `approval::resolve`:

```json
{ "session_id": "sess-dev", "function_call_id": "tc-dev", "decision": "allow" }
```

## Configuration

Committed defaults (`iii.worker.yaml`):

```yaml
topic: agent::before_function_call   # subscribe trigger topic
approval_state_scope: approvals      # state:: scope for approval records
default_timeout_ms: 300000           # Pending-row TTL → auto deny after 5 min
tick_interval_ms: 15000              # watchdog cadence; 0 disables the loop
```

No `rules:` key means policy is disabled and non-approval calls allow through. An explicit empty list (`rules: []`) means policy is enabled and no-match falls back to Ask. Rule order is first match wins, so place specific allow/deny entries before a catch-all ask.

Example local harness preset:

```yaml
rules:
  - { permission: "fs::read", pattern: "*", action: allow }
  - { permission: "shell::exec", pattern: "git status*", action: allow }
  - { permission: "approval::*", pattern: "*", action: allow }
  - { permission: "harness::call", pattern: "*", action: deny, reason: "harness dispatch cannot bypass approval policy" }
  - { permission: "shell::exec", pattern: "*", action: ask }
  - { permission: "*", pattern: "*", action: ask }
```

Engines may nest the block under `config:` exactly like [`policy-denylist`](../policy-denylist/README.md). Overrides:

| Env | Meaning |
|---|---|
| `APPROVAL_GATE_TOPIC` | Overrides `topic` |
| `APPROVAL_GATE_STATE_SCOPE` | Overrides `approval_state_scope` |
| `APPROVAL_GATE_TIMEOUT_MS` | Overrides `default_timeout_ms` |

Other defaults and serde aliases live in [`src/config.rs`](src/config.rs).

## Registered surfaces

| Function | Role |
|---|---|
| `policy::approval_gate` | Hook subscriber (`durable:subscriber` on `topic`). Runs the layered rules engine on every incoming call. |
| `approval::resolve` | Operator decision (`allow` / `deny`) for one pending `(session_id, function_call_id)`. Returns `{ok, cascaded?}`. On `allow + always: true`, pushes a session-scoped Allow rule and cascade-resolves the rest of the session's pending rows matching the same exact-argv pattern. |
| `approval::consume` | Atomic drain: returns Done rows for a session and deletes them in the same call. Pending and InFlight rows stay in state. Pending rows past `expires_at` are lazy-flipped before return. Required payload: `{session_id, limit?}`. Response: `{ok, entries, omitted}`. |
| `approval::list_pending` | UI-facing read: returns the current Pending rows for a session. Applies lazy-timeout flip on read. |
| `approval::sweep_session` | Force-cancel every non-terminal row for session stop/abort cleanup. |
| `approval::lookup_record` | Single-row lookup by `(session_id, function_call_id)`; returns null when absent. |
| `approval::tick_timeouts` | Watchdog tick: flips expired Pending + stale InFlight rows, returns `sessions_woken`, and the registered closure fires `run::resume` for each. Fires automatically every `tick_interval_ms` and is also callable on demand. |

## Runtime expectations

`state::*`, `stream::set`, `run::resume`, and the configured hook publisher must exist on the bus. Without `provider-router` emitting `agent::before_function_call`, nothing reaches the subscriber even though registrations succeed.

## Atomicity caveats

- **Same-process concurrent `approval::resolve`** for the same call_id is serialized by a per-key async mutex in `resolve.rs`. Cross-process races (two workers subscribing to the same scope) are not closed in-crate.
- **Same-process concurrent `approval::consume`** for the same session is serialized by a per-session async mutex in `delivery.rs`. Cross-process races require an engine-level atomic delete-if-present primitive (tracked in iii-database backlog); the in-process mutex is belt-and-suspenders for the single-worker shape.
