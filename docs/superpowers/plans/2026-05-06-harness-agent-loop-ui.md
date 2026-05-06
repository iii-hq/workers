# Harness agent-loop UI — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the harness web UI a real agent surface — actual tool calling, live event streaming via SSE, and inline user approvals for sensitive tool calls.

**Architecture:** A new `approval-gate` worker subscribes to `agent::before_tool_call` and blocks pending UI confirmation. `iii-harness` adds a `GET /bridge/events` SSE endpoint that tails the existing `agent::events/<session_id>` stream. The web UI switches from blocking `run::start_and_wait` to fire-and-forget `run::start` plus a native `EventSource`-driven event reducer; new components render `tool_use`, `tool_result`, and `ApprovalRow` blocks.

**Tech Stack:** Rust 2021 (workers, harness binary), iii-sdk 0.11.3, axum (HTTP triggers), React 18 + Vite 5 + TypeScript 5 (UI), Vitest (UI unit tests, added by this plan), Playwright (E2E, added by this plan).

**Spec:** [`docs/superpowers/specs/2026-05-06-harness-agent-loop-ui-design.md`](../specs/2026-05-06-harness-agent-loop-ui-design.md)

---

## File structure

**New crates / files:**

- `workers/approval-gate/Cargo.toml`
- `workers/approval-gate/iii.worker.yaml`
- `workers/approval-gate/src/lib.rs` — subscriber, decision loop, `approval::resolve` and `approval::list_pending` handlers
- `workers/approval-gate/src/main.rs` — binary entry point
- `workers/approval-gate/tests/integration.rs` — engine-backed test
- `workers/approval-gate/README.md`
- `harness/iii.worker.yaml` — restored manifest (currently missing per `harness/ARCHITECTURE.md:77`)
- `harness/tests/sse_bridge.rs` — SSE smoke + reconnect test
- `harness/web/src/reducer.ts` — pure `(state, AgentEvent) → state`
- `harness/web/src/reducer.test.ts` — vitest unit cases
- `harness/web/src/useAgentStream.ts` — `EventSource` hook
- `harness/web/src/components/ToolUseBlock.tsx`
- `harness/web/src/components/ToolResultBlock.tsx`
- `harness/web/src/components/ApprovalRow.tsx`
- `harness/web/vitest.config.ts`
- `harness/web/playwright.config.ts`
- `harness/web/tests/e2e/approval.spec.ts`

**Modified:**

- `turn-orchestrator/crates/harness-types/src/agent_event.rs` (+ two variants, + tests)
- `turn-orchestrator/src/run_start.rs` (carry `approval_required` into the run request)
- `turn-orchestrator/src/states/tools.rs` (include `approval_required` in the `agent::before_tool_call` payload)
- `harness/Cargo.toml` (add `axum`, `tokio-stream`, `futures` for SSE; add `tempfile` to dev-deps)
- `harness/src/lib.rs` (`approval-gate` in `EXPECTED_WORKERS`; register the SSE HTTP trigger)
- `harness/tests/integration.rs` (extend manifest assertion to include the new worker)
- `harness/web/src/types.ts` (`AgentEvent` discriminated union + reducer state)
- `harness/web/src/App.tsx` (`run::start`, `useAgentStream`, `parameters` rename, pass tools, `approval_required`)
- `harness/web/src/components/SessionView.tsx` (render every block type)
- `harness/web/package.json` (add `vitest`, `@playwright/test`)

**Deleted:** none.

---

## Phase 1 — wire format additions

### Task 1: Add `ApprovalRequested` and `ApprovalResolved` to `AgentEvent`

**Files:**

- Modify: `turn-orchestrator/crates/harness-types/src/agent_event.rs`

- [ ] **Step 1: Write the failing tests**

Add to the bottom of `agent_event.rs`'s `mod tests`:

```rust
    #[test]
    fn approval_requested_round_trips() {
        let evt = AgentEvent::ApprovalRequested {
            tool_call_id: "tc-9".into(),
            tool_name: "shell::filesystem::write".into(),
            args: serde_json::json!({ "path": "/tmp/x" }),
            expires_at: 1_700_000_000_000,
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_requested");
        assert_eq!(json["tool_call_id"], "tc-9");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);
    }

    #[test]
    fn approval_resolved_round_trips_with_optional_reason() {
        let evt = AgentEvent::ApprovalResolved {
            tool_call_id: "tc-9".into(),
            decision: "deny".into(),
            reason: Some("timeout".into()),
        };
        let json = serde_json::to_value(&evt).unwrap();
        assert_eq!(json["type"], "approval_resolved");
        assert_eq!(json["decision"], "deny");
        let back: AgentEvent = serde_json::from_value(json).unwrap();
        assert_eq!(back, evt);

        let none_reason = AgentEvent::ApprovalResolved {
            tool_call_id: "tc-9".into(),
            decision: "allow".into(),
            reason: None,
        };
        let json = serde_json::to_value(&none_reason).unwrap();
        assert!(json.get("reason").map_or(true, |v| v.is_null()));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p harness-types approval`
Expected: FAIL — `no variant or associated item named ApprovalRequested found`.

- [ ] **Step 3: Add the new variants**

In `turn-orchestrator/crates/harness-types/src/agent_event.rs`, add to the `AgentEvent` enum, after the `ToolExecutionEnd` variant and before the closing `}`:

```rust
    /// A tool call is paused by an approval subscriber, awaiting user decision.
    ApprovalRequested {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        /// Unix milliseconds. After this point the gate auto-denies.
        expires_at: u64,
    },
    /// Approval gate has resolved a previously-requested approval.
    ApprovalResolved {
        tool_call_id: String,
        /// "allow" or "deny".
        decision: String,
        /// Free-form reason — populated for "deny" (e.g. "timeout", "user").
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p harness-types`
Expected: PASS — all tests including the two new ones.

- [ ] **Step 5: Commit**

```bash
git add turn-orchestrator/crates/harness-types/src/agent_event.rs
git commit -m "feat(harness-types): add ApprovalRequested/ApprovalResolved AgentEvent variants"
```

---

## Phase 2 — orchestrator plumbing for `approval_required`

### Task 2: Carry `approval_required` into the persisted run request

**Files:**

- Modify: `turn-orchestrator/src/run_start.rs`

- [ ] **Step 1: Write the failing test**

Add to `mod tests` at the bottom of `run_start.rs`:

```rust
    #[test]
    fn build_run_request_propagates_approval_required() {
        let request = build_run_request(&json!({
            "approval_required": ["shell::filesystem::write"],
        }));
        assert_eq!(
            request["approval_required"],
            json!(["shell::filesystem::write"]),
        );
    }

    #[test]
    fn build_run_request_defaults_approval_required_to_empty() {
        let request = build_run_request(&json!({}));
        assert_eq!(request["approval_required"], json!([]));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iii-turn-orchestrator approval_required`
Expected: FAIL — assertion `request["approval_required"] == json!([])` fails because the field is `Null`.

- [ ] **Step 3: Add the field to `build_run_request`**

In `turn-orchestrator/src/run_start.rs:64-75`, add a line inside the `json!` block (immediately after the `tools` line):

```rust
        "tools": payload.get("tools").cloned().unwrap_or_else(|| json!([])),
        "approval_required": payload
            .get("approval_required")
            .cloned()
            .unwrap_or_else(|| json!([])),
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iii-turn-orchestrator`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add turn-orchestrator/src/run_start.rs
git commit -m "feat(turn-orchestrator): persist approval_required on run::start"
```

### Task 3: Include `approval_required` in `agent::before_tool_call` payload

**Files:**

- Modify: `turn-orchestrator/src/states/tools.rs`
- Modify: `turn-orchestrator/src/persistence.rs` (read run_request helper, if not already exported)

- [ ] **Step 1: Verify whether persistence exposes a run-request loader**

Run: `grep -n "load_run_request\|run_request_key" turn-orchestrator/src/persistence.rs turn-orchestrator/src/state.rs`
Expected: shows a `run_request_key` in `state.rs:97`. If `persistence.rs` has no `load_run_request`, the next step adds one; if it does, skip the `pub async fn load_run_request` addition.

- [ ] **Step 2: Write the failing test**

Add to the bottom of `turn-orchestrator/src/states/tools.rs`'s `mod tests`:

```rust
    #[test]
    fn before_tool_call_payload_carries_approval_required() {
        let tc = ToolCall {
            id: "tc-1".into(),
            name: "shell::filesystem::write".into(),
            arguments: json!({"path": "/tmp/x"}),
        };
        let approval_required = vec!["shell::filesystem::write".to_string()];
        let inner = build_before_tool_call_payload(&tc, &approval_required);
        assert_eq!(inner["tool_call"]["id"], "tc-1");
        assert_eq!(
            inner["approval_required"],
            json!(["shell::filesystem::write"]),
        );
    }

    #[test]
    fn before_tool_call_payload_omits_approval_required_when_empty() {
        let tc = ToolCall {
            id: "tc-1".into(),
            name: "shell::filesystem::ls".into(),
            arguments: json!({}),
        };
        let inner = build_before_tool_call_payload(&tc, &[]);
        assert_eq!(inner["approval_required"], json!([]));
    }
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test -p iii-turn-orchestrator before_tool_call_payload`
Expected: FAIL — `build_before_tool_call_payload` not defined.

- [ ] **Step 4: Add the helper and use it in `handle_prepare`**

In `turn-orchestrator/src/states/tools.rs`, add after the existing `build_finalize_lifecycle` helper:

```rust
/// Pure helper: build the inner payload for the `agent::before_tool_call`
/// topic. Subscribers (policy-denylist, approval-gate) read this shape.
pub(crate) fn build_before_tool_call_payload(
    tc: &ToolCall,
    approval_required: &[String],
) -> Value {
    json!({
        "tool_call": tc,
        "approval_required": approval_required,
    })
}
```

Then change the `publish_collect` call in `handle_prepare` (currently around line 24-31) from:

```rust
        let merged = publish_collect(
            iii,
            TOPIC_BEFORE,
            json!({ "tool_call": tc }),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
        )
        .await;
```

to:

```rust
        let merged = publish_collect(
            iii,
            TOPIC_BEFORE,
            build_before_tool_call_payload(&tc, &approval_required),
            "first_block_wins",
            HOOK_TIMEOUT_MS,
        )
        .await;
```

And, at the top of `handle_prepare`, load `approval_required` from the persisted run request once before the loop:

```rust
pub async fn handle_prepare(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    record.tool_results.clear();
    let calls = record.pending_tool_calls.clone();

    let run_request = persistence::load_run_request(iii, &record.session_id).await;
    let approval_required: Vec<String> = run_request
        .get("approval_required")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();

    let mut prepared: Vec<(ToolCall, Option<ToolResult>)> = Vec::with_capacity(calls.len());
    // ... existing loop
```

If `persistence::load_run_request` doesn't exist, add it to `turn-orchestrator/src/persistence.rs`:

```rust
pub async fn load_run_request(iii: &III, session_id: &str) -> serde_json::Value {
    let key = crate::state::run_request_key(session_id);
    iii.trigger(iii_sdk::TriggerRequest {
        function_id: "state::get".into(),
        payload: serde_json::json!({ "scope": "agent", "key": key }),
        action: None,
        timeout_ms: None,
    })
    .await
    .ok()
    .unwrap_or_else(|| serde_json::json!({}))
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test -p iii-turn-orchestrator`
Expected: PASS — all existing tests plus the two new helpers.

- [ ] **Step 6: Commit**

```bash
git add turn-orchestrator/src/states/tools.rs turn-orchestrator/src/persistence.rs
git commit -m "feat(turn-orchestrator): publish approval_required on agent::before_tool_call"
```

---

## Phase 3 — `approval-gate` worker

### Task 4: Scaffold the crate (Cargo.toml, main.rs, README, manifest)

**Files:**

- Create: `workers/approval-gate/Cargo.toml`
- Create: `workers/approval-gate/iii.worker.yaml`
- Create: `workers/approval-gate/src/main.rs`
- Create: `workers/approval-gate/src/lib.rs`
- Create: `workers/approval-gate/README.md`

- [ ] **Step 1: Create `Cargo.toml`**

```toml
[package]
name = "iii-approval-gate"
version = "0.1.0"
edition = "2021"
license = "Apache-2.0"
repository = "https://github.com/iii-hq/workers"
authors = ["iii contributors"]
publish = false

[lib]
name = "approval_gate"
path = "src/lib.rs"

[[bin]]
name = "iii-approval-gate"
path = "src/main.rs"

[dependencies]
iii-sdk = "=0.11.3"
serde_json = "1"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
async-trait = "0.1"
log = "0.4"
env_logger = "0.11"

[lints.rust]
unsafe_code = "forbid"
```

- [ ] **Step 2: Create `iii.worker.yaml`**

```yaml
iii: v1
name: approval-gate
language: rust
deploy: binary
manifest: Cargo.toml
bin: iii-approval-gate
description: Hook subscriber on agent::before_tool_call that pauses tool calls listed in approval_required until the UI resolves them via approval::resolve.
config:
  topic: agent::before_tool_call
  approval_state_scope: approvals
  default_timeout_ms: 300000
```

- [ ] **Step 3: Create `src/main.rs`**

```rust
use std::env;

use approval_gate::{register, Config};
use iii_sdk::III;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();
    let url = env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = III::connect(&url).await?;
    let _refs = register(&iii, Config::from_env())?;
    log::info!("approval-gate registered; awaiting events");
    tokio::signal::ctrl_c().await?;
    Ok(())
}
```

- [ ] **Step 4: Create stub `src/lib.rs`**

```rust
//! Approval gate. Subscribes to `agent::before_tool_call` and blocks calls
//! whose `tool_call.name` appears in the run's `approval_required` list,
//! waiting for the UI to call `approval::resolve` (or for a timeout).

use iii_sdk::{FunctionRef, III};

pub const FN_RESOLVE: &str = "approval::resolve";
pub const FN_LIST_PENDING: &str = "approval::list_pending";
pub const STATE_SCOPE: &str = "approvals";
pub const DEFAULT_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone)]
pub struct Config {
    pub topic: String,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            topic: "agent::before_tool_call".into(),
            timeout_ms: DEFAULT_TIMEOUT_MS,
        }
    }
}

impl Config {
    pub fn from_env() -> Self {
        let mut cfg = Self::default();
        if let Ok(t) = std::env::var("APPROVAL_GATE_TIMEOUT_MS") {
            if let Ok(n) = t.parse() {
                cfg.timeout_ms = n;
            }
        }
        cfg
    }
}

pub struct Refs {
    pub resolve: FunctionRef,
    pub list_pending: FunctionRef,
    pub subscriber_fn: FunctionRef,
    pub subscriber_trigger: iii_sdk::Trigger,
}

pub fn register(_iii: &III, _config: Config) -> anyhow::Result<Refs> {
    anyhow::bail!("not yet implemented")
}
```

- [ ] **Step 5: Create `README.md`**

```markdown
# approval-gate

Subscriber on `agent::before_tool_call`. Pauses tool calls whose name appears
in the run's `approval_required` list, emits `ApprovalRequested` onto
`agent::events/<session_id>`, and waits for the UI to call `approval::resolve`
(or for the configured timeout, default 5 minutes).

## Functions
- `approval::resolve { tool_call_id, decision, reason? }` — flip a pending entry to `allow` or `deny`.
- `approval::list_pending { session_id }` — return currently-blocked calls (used by the UI on tab refresh).

## Config (env)
- `APPROVAL_GATE_TIMEOUT_MS` — auto-deny timeout in ms (default `300000`).
```

- [ ] **Step 6: Verify the crate compiles**

Run: `cargo build -p iii-approval-gate`
Expected: PASS — compiles, with `register` returning the not-yet-implemented error at runtime.

- [ ] **Step 7: Commit**

```bash
git add workers/approval-gate
git commit -m "feat(approval-gate): scaffold crate, manifest, README"
```

### Task 5: Pure helpers — keys, topic decoding, decision merging

**Files:**

- Modify: `workers/approval-gate/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add at the bottom of `workers/approval-gate/src/lib.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pending_key_includes_session_and_tool_call_id() {
        assert_eq!(pending_key("s1", "tc-1"), "s1/tc-1");
    }

    #[test]
    fn extract_call_reads_session_id_and_tool_call_from_envelope() {
        let envelope = json!({
            "event_id": "evt-1",
            "reply_stream": "rs-1",
            "payload": {
                "tool_call": { "id": "tc-1", "name": "write", "arguments": {"path": "/tmp/x"} },
                "approval_required": ["write"],
                "session_id": "s1",
            }
        });
        let call = extract_call(&envelope).expect("decoded");
        assert_eq!(call.session_id, "s1");
        assert_eq!(call.tool_call_id, "tc-1");
        assert_eq!(call.tool_name, "write");
        assert_eq!(call.event_id, "evt-1");
        assert_eq!(call.reply_stream, "rs-1");
        assert!(call.approval_required.iter().any(|s| s == "write"));
    }

    #[test]
    fn requires_approval_only_for_listed_tools() {
        let call = IncomingCall {
            session_id: "s1".into(),
            tool_call_id: "tc-1".into(),
            tool_name: "ls".into(),
            args: json!({}),
            approval_required: vec!["write".into()],
            event_id: "e".into(),
            reply_stream: "r".into(),
        };
        assert!(!call.requires_approval());

        let call2 = IncomingCall {
            tool_name: "write".into(),
            ..call
        };
        assert!(call2.requires_approval());
    }

    #[test]
    fn build_pending_record_sets_status_and_expiry() {
        let now = 1_000_000;
        let rec = build_pending_record("tc-1", "write", &json!({"x": 1}), now, 60_000);
        assert_eq!(rec["status"], "pending");
        assert_eq!(rec["tool_call_id"], "tc-1");
        assert_eq!(rec["expires_at"], 1_060_000);
    }

    #[test]
    fn block_reply_for_decision_allow_does_not_block() {
        let reply = block_reply_for(&Decision::Allow);
        assert_eq!(reply["block"], false);
    }

    #[test]
    fn block_reply_for_deny_includes_reason() {
        let reply = block_reply_for(&Decision::Deny { reason: "timeout".into() });
        assert_eq!(reply["block"], true);
        assert_eq!(reply["reason"], "approval-gate: timeout");
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iii-approval-gate`
Expected: FAIL — none of the helpers exist yet.

- [ ] **Step 3: Implement the helpers**

Replace the contents of `workers/approval-gate/src/lib.rs` (keep the existing `Config`, `Refs`, constants, and the not-yet-implemented `register` for now) and add:

```rust
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct IncomingCall {
    pub session_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub args: Value,
    pub approval_required: Vec<String>,
    pub event_id: String,
    pub reply_stream: String,
}

impl IncomingCall {
    pub fn requires_approval(&self) -> bool {
        self.approval_required.iter().any(|n| n == &self.tool_name)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    Deny { reason: String },
}

pub fn pending_key(session_id: &str, tool_call_id: &str) -> String {
    format!("{session_id}/{tool_call_id}")
}

pub fn extract_call(envelope: &Value) -> Option<IncomingCall> {
    let event_id = envelope
        .get("event_id")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let reply_stream = envelope
        .get("reply_stream")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let inner = envelope.get("payload").unwrap_or(envelope);
    let tc = inner.get("tool_call")?;
    Some(IncomingCall {
        session_id: inner
            .get("session_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        tool_call_id: tc.get("id").and_then(Value::as_str)?.to_string(),
        tool_name: tc.get("name").and_then(Value::as_str)?.to_string(),
        args: tc.get("arguments").cloned().unwrap_or_else(|| json!({})),
        approval_required: inner
            .get("approval_required")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default(),
        event_id,
        reply_stream,
    })
}

pub fn build_pending_record(
    tool_call_id: &str,
    tool_name: &str,
    args: &Value,
    now_ms: u64,
    timeout_ms: u64,
) -> Value {
    json!({
        "tool_call_id": tool_call_id,
        "tool_name": tool_name,
        "args": args,
        "status": "pending",
        "expires_at": now_ms + timeout_ms,
    })
}

pub fn block_reply_for(decision: &Decision) -> Value {
    match decision {
        Decision::Allow => json!({ "block": false }),
        Decision::Deny { reason } => json!({
            "block": true,
            "reason": format!("approval-gate: {reason}"),
        }),
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iii-approval-gate`
Expected: PASS — all six helper tests.

- [ ] **Step 5: Commit**

```bash
git add workers/approval-gate/src/lib.rs
git commit -m "feat(approval-gate): pure helpers for envelope, pending state, decisions"
```

### Task 6: Implement the bus surface — `register` + state I/O trait

**Files:**

- Modify: `workers/approval-gate/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
    use std::sync::Mutex;

    struct InMemoryStateBus {
        store: Mutex<std::collections::HashMap<String, Value>>,
    }

    impl InMemoryStateBus {
        fn new() -> Self {
            Self { store: Mutex::new(std::collections::HashMap::new()) }
        }
    }

    #[async_trait::async_trait]
    impl StateBus for InMemoryStateBus {
        async fn set(&self, scope: &str, key: &str, value: Value) {
            self.store
                .lock()
                .unwrap()
                .insert(format!("{scope}/{key}"), value);
        }
        async fn get(&self, scope: &str, key: &str) -> Option<Value> {
            self.store.lock().unwrap().get(&format!("{scope}/{key}")).cloned()
        }
        async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
            let map = self.store.lock().unwrap();
            map.iter()
                .filter(|(k, _)| k.starts_with(&format!("{scope}/{prefix}")))
                .map(|(_, v)| v.clone())
                .collect()
        }
    }

    #[tokio::test]
    async fn resolve_flips_status_when_pending() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await;

        let out = handle_resolve(
            &bus,
            json!({
                "tool_call_id": "tc-1",
                "session_id": "s1",
                "decision": "allow",
            }),
        )
        .await;

        assert_eq!(out["ok"], true);
        let stored = bus
            .get(STATE_SCOPE, &pending_key("s1", "tc-1"))
            .await
            .unwrap();
        assert_eq!(stored["status"], "allow");
    }

    #[tokio::test]
    async fn resolve_rejects_already_resolved_entry() {
        let bus = InMemoryStateBus::new();
        let mut rec = build_pending_record("tc-1", "write", &json!({}), 0, 60_000);
        rec["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-1"), rec).await;

        let out = handle_resolve(
            &bus,
            json!({"tool_call_id": "tc-1", "session_id": "s1", "decision": "deny"}),
        )
        .await;
        assert_eq!(out["ok"], false);
        assert_eq!(out["error"], "already_resolved");
    }

    #[tokio::test]
    async fn list_pending_returns_only_pending_for_session() {
        let bus = InMemoryStateBus::new();
        bus.set(
            STATE_SCOPE,
            &pending_key("s1", "tc-1"),
            build_pending_record("tc-1", "write", &json!({}), 0, 60_000),
        )
        .await;
        let mut resolved = build_pending_record("tc-2", "write", &json!({}), 0, 60_000);
        resolved["status"] = json!("allow");
        bus.set(STATE_SCOPE, &pending_key("s1", "tc-2"), resolved).await;
        bus.set(
            STATE_SCOPE,
            &pending_key("other", "tc-3"),
            build_pending_record("tc-3", "write", &json!({}), 0, 60_000),
        )
        .await;

        let out = handle_list_pending(&bus, json!({ "session_id": "s1" })).await;
        let items = out["pending"].as_array().unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["tool_call_id"], "tc-1");
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iii-approval-gate`
Expected: FAIL — `StateBus`, `handle_resolve`, `handle_list_pending` not defined.

- [ ] **Step 3: Define the trait and handlers**

Append to `workers/approval-gate/src/lib.rs`:

```rust
#[async_trait::async_trait]
pub trait StateBus: Send + Sync {
    async fn set(&self, scope: &str, key: &str, value: Value);
    async fn get(&self, scope: &str, key: &str) -> Option<Value>;
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value>;
}

pub async fn handle_resolve(bus: &dyn StateBus, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let tool_call_id = payload
        .get("tool_call_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    let decision = payload
        .get("decision")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() || tool_call_id.is_empty() {
        return json!({ "ok": false, "error": "missing_id" });
    }
    if decision != "allow" && decision != "deny" {
        return json!({ "ok": false, "error": "bad_decision" });
    }
    let key = pending_key(session_id, tool_call_id);
    let Some(mut existing) = bus.get(STATE_SCOPE, &key).await else {
        return json!({ "ok": false, "error": "not_found" });
    };
    if existing.get("status").and_then(Value::as_str) != Some("pending") {
        return json!({ "ok": false, "error": "already_resolved" });
    }
    existing["status"] = json!(decision);
    if let Some(reason) = payload.get("reason").cloned() {
        existing["reason"] = reason;
    }
    bus.set(STATE_SCOPE, &key, existing).await;
    json!({ "ok": true })
}

pub async fn handle_list_pending(bus: &dyn StateBus, payload: Value) -> Value {
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .unwrap_or("");
    if session_id.is_empty() {
        return json!({ "pending": [] });
    }
    let prefix = format!("{session_id}/");
    let all = bus.list_prefix(STATE_SCOPE, &prefix).await;
    let pending: Vec<Value> = all
        .into_iter()
        .filter(|v| v.get("status").and_then(Value::as_str) == Some("pending"))
        .collect();
    json!({ "pending": pending })
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iii-approval-gate`
Expected: PASS — three new tokio tests plus all helpers.

- [ ] **Step 5: Commit**

```bash
git add workers/approval-gate/src/lib.rs
git commit -m "feat(approval-gate): handle_resolve, handle_list_pending, StateBus trait"
```

### Task 7: Wait-and-emit decision loop, real `register`

**Files:**

- Modify: `workers/approval-gate/src/lib.rs`

- [ ] **Step 1: Write the failing tests**

Add to `mod tests`:

```rust
    use std::sync::Arc;
    use std::time::Duration;

    fn now_ms() -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64
    }

    #[tokio::test]
    async fn await_decision_returns_allow_when_status_flips() {
        let bus = Arc::new(InMemoryStateBus::new());
        let key = pending_key("s1", "tc-1");
        bus.set(
            STATE_SCOPE,
            &key,
            build_pending_record("tc-1", "write", &json!({}), now_ms(), 5_000),
        )
        .await;

        let bus2 = bus.clone();
        let writer = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            let mut rec = bus2.get(STATE_SCOPE, &key).await.unwrap();
            rec["status"] = json!("allow");
            bus2.set(STATE_SCOPE, &key, rec).await;
        });

        let decision = await_decision(&*bus, "s1", "tc-1", now_ms() + 5_000).await;
        writer.await.unwrap();
        assert_eq!(decision, Decision::Allow);
    }

    #[tokio::test]
    async fn await_decision_returns_deny_timeout_when_expired() {
        let bus = InMemoryStateBus::new();
        let key = pending_key("s1", "tc-1");
        bus.set(
            STATE_SCOPE,
            &key,
            build_pending_record("tc-1", "write", &json!({}), 0, 0),
        )
        .await;
        let decision = await_decision(&bus, "s1", "tc-1", now_ms() - 10).await;
        match decision {
            Decision::Deny { reason } => assert_eq!(reason, "timeout"),
            other => panic!("expected Deny(timeout), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn await_decision_fail_closed_on_missing_record() {
        let bus = InMemoryStateBus::new();
        let decision = await_decision(&bus, "s1", "tc-1", now_ms() + 1_000).await;
        match decision {
            Decision::Deny { reason } => assert_eq!(reason, "state_unavailable"),
            other => panic!("expected Deny(state_unavailable), got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p iii-approval-gate await_decision`
Expected: FAIL — `await_decision` not defined.

- [ ] **Step 3: Implement the decision loop**

Append to `workers/approval-gate/src/lib.rs`:

```rust
const POLL_INTERVAL_MS: u64 = 250;

pub async fn await_decision(
    bus: &dyn StateBus,
    session_id: &str,
    tool_call_id: &str,
    expires_at: u64,
) -> Decision {
    let key = pending_key(session_id, tool_call_id);
    loop {
        let Some(rec) = bus.get(STATE_SCOPE, &key).await else {
            return Decision::Deny {
                reason: "state_unavailable".into(),
            };
        };
        match rec.get("status").and_then(Value::as_str) {
            Some("allow") => return Decision::Allow,
            Some("deny") => {
                let reason = rec
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or("user")
                    .to_string();
                return Decision::Deny { reason };
            }
            _ => {}
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(expires_at);
        if now >= expires_at {
            return Decision::Deny {
                reason: "timeout".into(),
            };
        }
        tokio::time::sleep(std::time::Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p iii-approval-gate`
Expected: PASS — all tests including the three new `await_decision` cases.

- [ ] **Step 5: Wire `register` into the iii bus**

Replace the stub `register` in `workers/approval-gate/src/lib.rs` with:

```rust
use iii_sdk::{IIIError, RegisterFunctionMessage, RegisterTriggerInput, Trigger, TriggerRequest};
use std::sync::Arc;

pub struct IiiStateBus(pub iii_sdk::III);

#[async_trait::async_trait]
impl StateBus for IiiStateBus {
    async fn set(&self, scope: &str, key: &str, value: Value) {
        let _ = self
            .0
            .trigger(TriggerRequest {
                function_id: "state::set".into(),
                payload: json!({ "scope": scope, "key": key, "value": value }),
                action: None,
                timeout_ms: None,
            })
            .await;
    }
    async fn get(&self, scope: &str, key: &str) -> Option<Value> {
        self.0
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": scope, "key": key }),
                action: None,
                timeout_ms: None,
            })
            .await
            .ok()
            .filter(|v| !v.is_null())
    }
    async fn list_prefix(&self, scope: &str, prefix: &str) -> Vec<Value> {
        let resp = self
            .0
            .trigger(TriggerRequest {
                function_id: "state::list".into(),
                payload: json!({ "scope": scope, "prefix": prefix }),
                action: None,
                timeout_ms: None,
            })
            .await
            .unwrap_or_else(|_| json!({ "items": [] }));
        resp.get("items")
            .and_then(|v| v.as_array().cloned())
            .unwrap_or_default()
            .into_iter()
            .map(|entry| entry.get("value").cloned().unwrap_or(entry))
            .collect()
    }
}

async fn write_event(iii: &iii_sdk::III, session_id: &str, event: &Value) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
                "item_id": format!("approval-{}", uuid_like()),
                "data": event,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

fn uuid_like() -> String {
    // Lightweight unique-ish id without pulling uuid in: ns timestamp + counter.
    use std::sync::atomic::{AtomicU64, Ordering};
    static C: AtomicU64 = AtomicU64::new(0);
    let n = C.fetch_add(1, Ordering::Relaxed);
    let t = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{t:x}-{n:x}")
}

async fn write_hook_reply(iii: &iii_sdk::III, stream_name: &str, event_id: &str, reply: &Value) {
    if stream_name.is_empty() || event_id.is_empty() {
        return;
    }
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": stream_name,
                "group_id": event_id,
                "item_id": uuid_like(),
                "data": reply,
            }),
            action: None,
            timeout_ms: None,
        })
        .await;
}

pub fn register(iii: &iii_sdk::III, config: Config) -> anyhow::Result<Refs> {
    let bus: Arc<dyn StateBus> = Arc::new(IiiStateBus(iii.clone()));

    // approval::resolve
    let bus_for_resolve = bus.clone();
    let resolve = iii.register_function((
        RegisterFunctionMessage::with_id(FN_RESOLVE.into())
            .with_description("Flip a pending approval entry to allow or deny.".into()),
        move |payload: Value| {
            let bus = bus_for_resolve.clone();
            async move { Ok::<_, IIIError>(handle_resolve(bus.as_ref(), payload).await) }
        },
    ));

    // approval::list_pending
    let bus_for_list = bus.clone();
    let list_pending = iii.register_function((
        RegisterFunctionMessage::with_id(FN_LIST_PENDING.into())
            .with_description("Return pending approvals for a session.".into()),
        move |payload: Value| {
            let bus = bus_for_list.clone();
            async move { Ok::<_, IIIError>(handle_list_pending(bus.as_ref(), payload).await) }
        },
    ));

    // Subscriber on agent::before_tool_call
    let timeout_ms = config.timeout_ms;
    let topic = config.topic.clone();
    let iii_for_sub = iii.clone();
    let bus_for_sub = bus.clone();
    let subscriber_fn = iii.register_function((
        RegisterFunctionMessage::with_id("policy::approval_gate".into())
            .with_description("Pause tool calls listed in approval_required.".into()),
        move |envelope: Value| {
            let iii = iii_for_sub.clone();
            let bus = bus_for_sub.clone();
            async move {
                let Some(call) = extract_call(&envelope) else {
                    return Ok::<_, IIIError>(json!({ "block": false }));
                };
                if !call.requires_approval() {
                    let reply = json!({ "block": false });
                    write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                    return Ok(reply);
                }
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                let expires_at = now + timeout_ms;
                let record = build_pending_record(
                    &call.tool_call_id,
                    &call.tool_name,
                    &call.args,
                    now,
                    timeout_ms,
                );
                bus.set(
                    STATE_SCOPE,
                    &pending_key(&call.session_id, &call.tool_call_id),
                    record,
                )
                .await;
                write_event(
                    &iii,
                    &call.session_id,
                    &json!({
                        "type": "approval_requested",
                        "tool_call_id": call.tool_call_id,
                        "tool_name": call.tool_name,
                        "args": call.args,
                        "expires_at": expires_at,
                    }),
                )
                .await;
                let decision =
                    await_decision(bus.as_ref(), &call.session_id, &call.tool_call_id, expires_at)
                        .await;
                let reason = match &decision {
                    Decision::Allow => None,
                    Decision::Deny { reason } => Some(reason.clone()),
                };
                let decision_str = match decision {
                    Decision::Allow => "allow",
                    Decision::Deny { .. } => "deny",
                };
                write_event(
                    &iii,
                    &call.session_id,
                    &json!({
                        "type": "approval_resolved",
                        "tool_call_id": call.tool_call_id,
                        "decision": decision_str,
                        "reason": reason,
                    }),
                )
                .await;
                let reply = block_reply_for(&match decision_str {
                    "allow" => Decision::Allow,
                    _ => Decision::Deny {
                        reason: reason.unwrap_or_else(|| "user".into()),
                    },
                });
                write_hook_reply(&iii, &call.reply_stream, &call.event_id, &reply).await;
                Ok(reply)
            }
        },
    ));

    let subscriber_trigger = iii
        .register_trigger(RegisterTriggerInput {
            trigger_type: "subscribe".into(),
            function_id: "policy::approval_gate".into(),
            config: json!({ "topic": topic }),
            metadata: None,
        })
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    Ok(Refs {
        resolve,
        list_pending,
        subscriber_fn,
        subscriber_trigger,
    })
}
```

- [ ] **Step 6: Verify the crate compiles**

Run: `cargo build -p iii-approval-gate`
Expected: PASS.

- [ ] **Step 7: Run all tests**

Run: `cargo test -p iii-approval-gate`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add workers/approval-gate/src/lib.rs
git commit -m "feat(approval-gate): subscribe agent::before_tool_call, await UI decision, emit events"
```

### Task 8: Engine-backed integration test

**Files:**

- Create: `workers/approval-gate/tests/integration.rs`

- [ ] **Step 1: Reuse the existing integration-test pattern**

Run: `cat workers/policy-denylist/tests/integration.rs` to see the harness this repo uses for engine-backed tests.

- [ ] **Step 2: Write the integration test**

Create `workers/approval-gate/tests/integration.rs`:

```rust
//! Engine-backed test for approval-gate. Spins up an in-process engine,
//! registers the gate, fires a before_tool_call envelope, posts
//! approval::resolve, and asserts unblock under 1 s.

use approval_gate::{register, Config, FN_RESOLVE, STATE_SCOPE};
use iii_sdk::{TriggerRequest, III};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn round_trip_allow_unblocks_under_one_second() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = match III::connect(&url).await {
        Ok(c) => c,
        Err(_) => {
            eprintln!("skipping: no engine at {url}");
            return;
        }
    };
    let _refs = register(
        &iii,
        Config {
            topic: "agent::before_tool_call::test".into(),
            timeout_ms: 5_000,
        },
    )
    .expect("register");

    let session_id = "approval-it-1";
    let tool_call_id = "tc-it-1";

    // Simulate the topic publish via direct call to the subscriber function.
    let envelope = json!({
        "event_id": "evt-it-1",
        "reply_stream": "rs-it-1",
        "payload": {
            "session_id": session_id,
            "tool_call": { "id": tool_call_id, "name": "shell::filesystem::write", "arguments": {} },
            "approval_required": ["shell::filesystem::write"],
        }
    });

    let subscriber_call = tokio::spawn({
        let iii = iii.clone();
        async move {
            iii.trigger(TriggerRequest {
                function_id: "policy::approval_gate".into(),
                payload: envelope,
                action: None,
                timeout_ms: Some(10_000),
            })
            .await
        }
    });

    // Wait for the pending entry to appear.
    let key = format!("{session_id}/{tool_call_id}");
    let mut tries = 0;
    loop {
        let v = iii
            .trigger(TriggerRequest {
                function_id: "state::get".into(),
                payload: json!({ "scope": STATE_SCOPE, "key": key }),
                action: None,
                timeout_ms: None,
            })
            .await
            .unwrap_or(json!(null));
        if v.get("status").and_then(|s| s.as_str()) == Some("pending") {
            break;
        }
        tries += 1;
        assert!(tries < 40, "pending entry never appeared");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let started = std::time::Instant::now();
    let resolve = iii
        .trigger(TriggerRequest {
            function_id: FN_RESOLVE.into(),
            payload: json!({
                "session_id": session_id,
                "tool_call_id": tool_call_id,
                "decision": "allow",
            }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("resolve");
    assert_eq!(resolve["ok"], true);

    let reply = subscriber_call
        .await
        .expect("join")
        .expect("subscriber returned");
    let elapsed = started.elapsed();
    assert!(elapsed < Duration::from_millis(1_000), "took {elapsed:?}");
    assert_eq!(reply["block"], false);
}
```

- [ ] **Step 3: Run the integration test**

Run: `cargo test -p iii-approval-gate --test integration -- --nocapture`
Expected: either PASS, or skipped with `skipping: no engine at …` if no engine is running. Boot one with `harness/scripts/demo.sh engine` if you want to see it pass locally.

- [ ] **Step 4: Commit**

```bash
git add workers/approval-gate/tests/integration.rs
git commit -m "test(approval-gate): engine-backed allow round-trip under 1s"
```

---

## Phase 4 — harness binary changes

### Task 9: Restore `harness/iii.worker.yaml`

**Files:**

- Create: `harness/iii.worker.yaml`

- [ ] **Step 1: Verify the test that drives this**

Run: `cat harness/tests/integration.rs`
Expected: shows `expected_workers_matches_yaml_dependency_count` reading `iii.worker.yaml` and asserting one dependency line per `EXPECTED_WORKERS` entry.

- [ ] **Step 2: Confirm current test fails**

Run: `cargo test -p iii-harness --test integration`
Expected: FAIL — `include_str!` on a missing file, or the count assertion blowing up.

- [ ] **Step 3: Create the manifest**

Create `harness/iii.worker.yaml`:

```yaml
iii: v1
name: harness
language: rust
deploy: binary
manifest: Cargo.toml
bin: iii-harness
description: Harness meta-worker. Registers harness::status and bridge::trigger; depends on the modular workers that back the iii chat surface.
dependencies:
  turn-orchestrator: latest
  provider-router: latest
  context-compaction: latest
  session-tree: latest
  session-corpus: latest
  document-extract: latest
  models-catalog: latest
  auth-credentials: latest
  auth-rbac: latest
  audit-log: latest
  policy-denylist: latest
  dlp-scrubber: latest
  guardrails: latest
  llm-budget: latest
  session-inbox: latest
  hook-fanout: latest
  shell-bash: latest
  shell-filesystem: latest
  subagent: latest
  provider-cli: latest
  provider-anthropic: latest
  provider-openai: latest
```

- [ ] **Step 4: Run the integration test**

Run: `cargo test -p iii-harness --test integration`
Expected: PASS — counts match (22 entries on each side).

- [ ] **Step 5: Commit**

```bash
git add harness/iii.worker.yaml
git commit -m "chore(harness): restore iii.worker.yaml manifest"
```

### Task 10: Add `approval-gate` to `EXPECTED_WORKERS` and the manifest

**Files:**

- Modify: `harness/src/lib.rs`
- Modify: `harness/iii.worker.yaml`

- [ ] **Step 1: Add to `EXPECTED_WORKERS`**

In `harness/src/lib.rs:18-41`, add `"approval-gate",` to the array (alphabetical or after `audit-log` is fine — there is no enforced order; the test only counts entries).

```rust
pub const EXPECTED_WORKERS: &[&str] = &[
    "turn-orchestrator",
    "provider-router",
    "context-compaction",
    "session-tree",
    "session-corpus",
    "document-extract",
    "models-catalog",
    "auth-credentials",
    "auth-rbac",
    "audit-log",
    "approval-gate",
    "policy-denylist",
    "dlp-scrubber",
    "guardrails",
    "llm-budget",
    "session-inbox",
    "hook-fanout",
    "shell-bash",
    "shell-filesystem",
    "subagent",
    "provider-cli",
    "provider-anthropic",
    "provider-openai",
];
```

- [ ] **Step 2: Add the dependency line to the manifest**

Add to `harness/iii.worker.yaml`'s `dependencies:` block:

```yaml
  approval-gate: latest
```

- [ ] **Step 3: Run the integration test**

Run: `cargo test -p iii-harness --test integration`
Expected: PASS — both sides now report 23.

- [ ] **Step 4: Commit**

```bash
git add harness/src/lib.rs harness/iii.worker.yaml
git commit -m "feat(harness): register approval-gate as expected worker"
```

### Task 11: Discovery — pin down the iii-sdk stream-tail API

**Files:**

- Read-only: `Cargo.lock`, the iii-sdk source

- [ ] **Step 1: Locate the iii-sdk crate source**

Run: `cargo doc --open -p iii-sdk` in the harness crate, OR `find ~/.cargo/registry/src -type d -name 'iii-sdk-*' -maxdepth 5`. Inspect for any function or trigger named like `stream::tail`, `stream::read`, `stream::range`, or any method on `III` such as `subscribe_stream`, `consume_stream`, `tail_stream`.

- [ ] **Step 2: Capture the answer in a doc comment**

Update the top of `harness/src/lib.rs` (or a new `harness/src/sse.rs` if you split — see next task) with a short comment recording: the function id used to read events, its payload shape, whether it returns one batch or blocks, and the cursor field name. Example placeholder format:

```rust
//! Event tail uses `stream::<NAME>` with payload `{stream_name, group_id, since_id?, max_items, block_ms}` returning `{items: [{item_id, data}], next_cursor}`.
```

If the SDK exposes only `stream::set` and no read, fall back to: register a `subscribe` trigger on `agent::events` (mirroring `policy-denylist`'s subscriber), buffer received frames in a per-session `tokio::sync::broadcast` channel, and pump those into SSE. Document the chosen path before continuing to Task 12.

- [ ] **Step 3: Commit the doc comment**

```bash
git add harness/src/lib.rs
git commit -m "docs(harness): record stream-tail discovery for SSE pump"
```

### Task 12: SSE endpoint — `GET /bridge/events`

**Files:**

- Modify: `harness/Cargo.toml`
- Create: `harness/src/sse.rs`
- Modify: `harness/src/lib.rs`

- [ ] **Step 1: Add dependencies**

In `harness/Cargo.toml`'s `[dependencies]`:

```toml
tokio-stream = "0.1"
futures-util = "0.3"
async-trait = "0.1"
```

(Do not add `axum` directly; HTTP triggers are surfaced through iii-sdk and return `{status_code, headers, body}`. The SSE pump returns a streamed body shaped as a string; if the SDK requires a chunked transfer encoding, the discovery step in Task 11 surfaces the right return shape.)

- [ ] **Step 2: Write the failing unit test for the framing helper**

Create `harness/src/sse.rs`:

```rust
//! Server-Sent Events helpers for the `/bridge/events` endpoint.

use serde_json::Value;

/// Format a single SSE frame: `id: <id>\ndata: <json>\n\n`.
pub fn format_frame(id: &str, data: &Value) -> String {
    format!("id: {id}\ndata: {}\n\n", data)
}

/// Heartbeat comment line. Sent every ~15s to defeat proxy idle timeouts.
pub fn heartbeat() -> &'static str {
    ": keepalive\n\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_carries_id_and_serialized_data() {
        let f = format_frame("evt-1", &json!({"type": "agent_start"}));
        assert!(f.starts_with("id: evt-1\n"));
        assert!(f.contains("data: {\"type\":\"agent_start\"}"));
        assert!(f.ends_with("\n\n"));
    }

    #[test]
    fn heartbeat_is_a_comment_line() {
        assert!(heartbeat().starts_with(":"));
        assert!(heartbeat().ends_with("\n\n"));
    }
}
```

In `harness/src/lib.rs`, add `pub mod sse;` near the top.

- [ ] **Step 3: Run the unit tests**

Run: `cargo test -p iii-harness sse`
Expected: PASS — two new tests.

- [ ] **Step 4: Add the SSE handler and HTTP trigger registration**

Append to `harness/src/lib.rs` (inside `register_with_iii`, after the `bridge::trigger` HTTP trigger registration):

```rust
    let iii_for_events = iii.clone();
    let events_fn = iii.register_function((
        RegisterFunctionMessage::with_id("bridge::events".into()).with_description(
            "Tail agent::events/<session_id> as Server-Sent Events. Used by harness/web/."
                .into(),
        ),
        move |input: Value| {
            let iii = iii_for_events.clone();
            async move {
                let body = input.get("body").cloned().unwrap_or(input.clone());
                let query = input
                    .get("query_params")
                    .cloned()
                    .or_else(|| body.get("query_params").cloned())
                    .unwrap_or_else(|| json!({}));
                let session_id = query
                    .get("session_id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| IIIError::Handler("missing session_id".into()))?
                    .to_string();
                let last_event_id = input
                    .get("headers")
                    .and_then(|h| h.get("Last-Event-ID").or_else(|| h.get("last-event-id")))
                    .and_then(Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        query
                            .get("since")
                            .and_then(Value::as_str)
                            .map(str::to_string)
                    });

                // Pump events. The exact iii-sdk read API is captured by the Task 11
                // discovery comment at the top of this file. The body below assumes a
                // batch `stream::tail` returning `{items: [{item_id, data}], next_cursor}`;
                // if the discovery mapped to a different shape, adjust accordingly.
                let body = pump_events(&iii, &session_id, last_event_id).await;
                Ok::<_, IIIError>(json!({
                    "status_code": 200,
                    "headers": {
                        "content-type": "text/event-stream",
                        "cache-control": "no-cache",
                        "connection": "keep-alive",
                    },
                    "body": body,
                }))
            }
        },
    ));

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "http".into(),
        function_id: "bridge::events".into(),
        config: json!({
            "api_path": "bridge/events",
            "http_method": "GET",
            "timeout_ms": 0u64, // 0 = no timeout; SSE is long-lived
            "stream_response": true,
        }),
        metadata: None,
    })
    .map_err(|e| anyhow::anyhow!(e.to_string()))?;
```

Add the helper at the bottom of `harness/src/lib.rs` (above `mod tests`):

```rust
async fn pump_events(iii: &III, session_id: &str, since: Option<String>) -> String {
    use crate::sse::{format_frame, heartbeat};
    let mut out = String::new();
    let mut cursor = since;
    let started = std::time::Instant::now();
    loop {
        let payload = json!({
            "stream_name": "agent::events",
            "group_id": session_id,
            "since_id": cursor,
            "max_items": 50,
            "block_ms": 5_000,
        });
        let resp = iii
            .trigger(TriggerRequest {
                function_id: "stream::tail".into(),
                payload,
                action: None,
                timeout_ms: Some(15_000),
            })
            .await;
        if let Ok(v) = resp {
            if let Some(items) = v.get("items").and_then(|x| x.as_array()) {
                for item in items {
                    let id = item
                        .get("item_id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    let data = item.get("data").cloned().unwrap_or(item.clone());
                    out.push_str(&format_frame(&id, &data));
                    cursor = Some(id);
                }
            }
        }
        if started.elapsed() > std::time::Duration::from_secs(3_600) {
            break;
        }
        out.push_str(heartbeat());
    }
    out
}
```

Update the returned `HarnessFunctionRefs` and `unregister_all` to include `events_fn`:

```rust
pub struct HarnessFunctionRefs {
    pub status: FunctionRef,
    pub bridge: FunctionRef,
    pub events: FunctionRef,
}

impl HarnessFunctionRefs {
    pub fn unregister_all(self) {
        self.status.unregister();
        self.bridge.unregister();
        self.events.unregister();
    }
}
```

And the final `Ok(...)` becomes `Ok(HarnessFunctionRefs { status, bridge, events: events_fn })`.

> Note: `pump_events` returns a fully-buffered string; iii-sdk's HTTP trigger may not stream chunked bodies. If the discovery from Task 11 shows the SDK supports a stream-response handler shape (e.g. an async iterator return), refactor `pump_events` to yield per-frame instead of building a `String`. Mark this as a follow-up if the SDK doesn't expose it.

- [ ] **Step 5: Verify the harness compiles and unit tests pass**

Run: `cargo test -p iii-harness`
Expected: PASS — `expected_workers_*` and `sse::*` tests.

- [ ] **Step 6: Commit**

```bash
git add harness/Cargo.toml harness/src/lib.rs harness/src/sse.rs
git commit -m "feat(harness): GET /bridge/events SSE endpoint over agent::events stream"
```

### Task 13: SSE integration test

**Files:**

- Create: `harness/tests/sse_bridge.rs`

- [ ] **Step 1: Write the test**

Create `harness/tests/sse_bridge.rs`:

```rust
//! Engine-backed smoke test for /bridge/events.
//!
//! Skipped when no engine is reachable. To run locally:
//!   harness/scripts/demo.sh engine && cargo test -p iii-harness --test sse_bridge

use harness::register_with_iii;
use iii_sdk::{TriggerRequest, III};
use serde_json::json;
use std::time::Duration;

#[tokio::test]
async fn sse_endpoint_emits_frames_and_resumes_from_cursor() {
    let url = std::env::var("III_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".into());
    let iii = match III::connect(&url).await {
        Ok(c) => c,
        Err(_) => {
            eprintln!("skipping: no engine at {url}");
            return;
        }
    };
    let _refs = register_with_iii(&iii).await.expect("register harness");

    let session_id = "sse-it-1";
    // Write three events.
    for (i, ty) in ["agent_start", "turn_start", "turn_end"].iter().enumerate() {
        iii.trigger(TriggerRequest {
            function_id: "stream::set".into(),
            payload: json!({
                "stream_name": "agent::events",
                "group_id": session_id,
                "item_id": format!("{session_id}-{i:08}"),
                "data": { "type": ty },
            }),
            action: None,
            timeout_ms: None,
        })
        .await
        .unwrap();
    }

    // Hit the endpoint with no cursor; expect three id: lines.
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "bridge::events".into(),
            payload: json!({
                "query_params": { "session_id": session_id },
                "headers": {},
            }),
            action: None,
            timeout_ms: Some(8_000),
        })
        .await
        .expect("call /bridge/events");
    let body = resp["body"].as_str().unwrap_or_default().to_string();
    let id_lines = body.matches("id: ").count();
    assert!(id_lines >= 3, "expected ≥3 frames, got {id_lines}: {body}");

    // Reconnect with Last-Event-ID of the second event; expect only the third.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let resp2 = iii
        .trigger(TriggerRequest {
            function_id: "bridge::events".into(),
            payload: json!({
                "query_params": { "session_id": session_id },
                "headers": { "Last-Event-ID": format!("{session_id}-00000001") },
            }),
            action: None,
            timeout_ms: Some(8_000),
        })
        .await
        .expect("call /bridge/events with cursor");
    let body2 = resp2["body"].as_str().unwrap_or_default().to_string();
    assert!(body2.contains("turn_end"));
    assert!(!body2.contains("agent_start"), "agent_start should not replay");
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test -p iii-harness --test sse_bridge -- --nocapture`
Expected: PASS, or skipped if no engine.

- [ ] **Step 3: Commit**

```bash
git add harness/tests/sse_bridge.rs
git commit -m "test(harness): SSE smoke test with cursor-based resume"
```

---

## Phase 5 — UI

### Task 14: Add Vitest to harness/web

**Files:**

- Modify: `harness/web/package.json`
- Create: `harness/web/vitest.config.ts`

- [ ] **Step 1: Add dev dependency and script**

Edit `harness/web/package.json`:

```json
{
  "name": "harness-web",
  "private": true,
  "version": "0.1.0",
  "type": "module",
  "scripts": {
    "dev": "vite",
    "build": "tsc && vite build",
    "preview": "vite preview",
    "test": "vitest run",
    "test:watch": "vitest"
  },
  "dependencies": {
    "react": "^18.3.1",
    "react-dom": "^18.3.1"
  },
  "devDependencies": {
    "@types/react": "^18.3.12",
    "@types/react-dom": "^18.3.1",
    "@vitejs/plugin-react": "^4.3.4",
    "typescript": "^5.6.3",
    "vite": "^5.4.11",
    "vitest": "^2.1.4"
  }
}
```

- [ ] **Step 2: Create the vitest config**

Create `harness/web/vitest.config.ts`:

```typescript
import { defineConfig } from "vitest/config";

export default defineConfig({
  test: {
    environment: "node",
    include: ["src/**/*.test.{ts,tsx}"],
  },
});
```

- [ ] **Step 3: Install and verify**

Run: `cd harness/web && npm install && npm test`
Expected: PASS with "no test files found" — vitest exits 0 because we haven't added tests yet.

- [ ] **Step 4: Commit**

```bash
git add harness/web/package.json harness/web/package-lock.json harness/web/vitest.config.ts
git commit -m "chore(harness/web): add vitest"
```

### Task 15: Define `AgentEvent` types and reducer state in TS

**Files:**

- Modify: `harness/web/src/types.ts`

- [ ] **Step 1: Read existing types**

Run: `cat harness/web/src/types.ts` — identify how `AgentMessage` is shaped today; preserve those exports.

- [ ] **Step 2: Append the new types**

Add to `harness/web/src/types.ts` (do not remove existing exports):

```typescript
export type AgentEvent =
  | { type: "agent_start" }
  | { type: "agent_end"; messages: AgentMessage[] }
  | { type: "turn_start" }
  | { type: "turn_end"; message: AgentMessage; tool_results: unknown[] }
  | { type: "message_start"; message: AgentMessage }
  | { type: "message_update"; message: AgentMessage; llm_event: unknown }
  | { type: "message_end"; message: AgentMessage }
  | {
      type: "tool_execution_start";
      tool_call_id: string;
      tool_name: string;
      args: unknown;
    }
  | {
      type: "tool_execution_update";
      tool_call_id: string;
      tool_name: string;
      args: unknown;
      partial_result: unknown;
    }
  | {
      type: "tool_execution_end";
      tool_call_id: string;
      tool_name: string;
      result: unknown;
      is_error: boolean;
    }
  | {
      type: "approval_requested";
      tool_call_id: string;
      tool_name: string;
      args: unknown;
      expires_at: number;
    }
  | {
      type: "approval_resolved";
      tool_call_id: string;
      decision: "allow" | "deny";
      reason?: string | null;
    };

export interface PendingApproval {
  tool_call_id: string;
  tool_name: string;
  args: unknown;
  expires_at: number;
}

export interface StreamState {
  messages: AgentMessage[];
  pendingApprovals: PendingApproval[];
  status: "idle" | "running" | "ended";
}

export const INITIAL_STREAM_STATE: StreamState = {
  messages: [],
  pendingApprovals: [],
  status: "idle",
};
```

- [ ] **Step 3: Verify the build**

Run: `cd harness/web && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 4: Commit**

```bash
git add harness/web/src/types.ts
git commit -m "feat(harness/web): AgentEvent + StreamState types"
```

### Task 16: Reducer — `(state, AgentEvent) => state`

**Files:**

- Create: `harness/web/src/reducer.ts`
- Create: `harness/web/src/reducer.test.ts`

- [ ] **Step 1: Write the failing tests**

Create `harness/web/src/reducer.test.ts`:

```typescript
import { describe, expect, it } from "vitest";
import { applyEvent } from "./reducer";
import { INITIAL_STREAM_STATE, type AgentEvent, type AgentMessage } from "./types";

const userMsg = (text: string): AgentMessage => ({
  role: "user",
  content: [{ type: "text", text }],
  timestamp: 0,
});

describe("applyEvent", () => {
  it("agent_start sets status to running", () => {
    const next = applyEvent(INITIAL_STREAM_STATE, { type: "agent_start" });
    expect(next.status).toBe("running");
  });

  it("agent_end sets status to ended", () => {
    const next = applyEvent(
      { ...INITIAL_STREAM_STATE, status: "running" },
      { type: "agent_end", messages: [] },
    );
    expect(next.status).toBe("ended");
  });

  it("message_end appends the message", () => {
    const m = userMsg("hi");
    const next = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: m });
    expect(next.messages).toHaveLength(1);
    expect(next.messages[0]).toEqual(m);
  });

  it("duplicate message_end with same role+timestamp does not append twice", () => {
    const m = userMsg("hi");
    const s1 = applyEvent(INITIAL_STREAM_STATE, { type: "message_end", message: m });
    const s2 = applyEvent(s1, { type: "message_end", message: m });
    expect(s2.messages).toHaveLength(1);
  });

  it("approval_requested adds a pending entry", () => {
    const next = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_requested",
      tool_call_id: "tc-1",
      tool_name: "shell::filesystem::write",
      args: { path: "/tmp/x" },
      expires_at: 0,
    });
    expect(next.pendingApprovals).toHaveLength(1);
    expect(next.pendingApprovals[0].tool_call_id).toBe("tc-1");
  });

  it("approval_resolved clears the matching pending entry", () => {
    const seeded = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_requested",
      tool_call_id: "tc-1",
      tool_name: "x",
      args: {},
      expires_at: 0,
    });
    const next = applyEvent(seeded, {
      type: "approval_resolved",
      tool_call_id: "tc-1",
      decision: "allow",
    });
    expect(next.pendingApprovals).toHaveLength(0);
  });

  it("approval_resolved before its requested is a no-op (replay)", () => {
    const next = applyEvent(INITIAL_STREAM_STATE, {
      type: "approval_resolved",
      tool_call_id: "tc-1",
      decision: "deny",
    });
    expect(next.pendingApprovals).toHaveLength(0);
  });

  it("unknown event variants pass through unchanged", () => {
    const unknown = { type: "totally_made_up" } as unknown as AgentEvent;
    const next = applyEvent(INITIAL_STREAM_STATE, unknown);
    expect(next).toBe(INITIAL_STREAM_STATE);
  });
});
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cd harness/web && npm test`
Expected: FAIL — `applyEvent` not found.

- [ ] **Step 3: Implement the reducer**

Create `harness/web/src/reducer.ts`:

```typescript
import {
  type AgentEvent,
  type AgentMessage,
  type PendingApproval,
  type StreamState,
} from "./types";

function messageKey(m: AgentMessage): string {
  return `${m.role}:${m.timestamp ?? 0}:${JSON.stringify(m.content).length}`;
}

export function applyEvent(state: StreamState, event: AgentEvent): StreamState {
  switch (event.type) {
    case "agent_start":
      return { ...state, status: "running" };
    case "agent_end":
      return { ...state, status: "ended" };
    case "turn_start":
    case "turn_end":
    case "message_start":
    case "message_update":
    case "tool_execution_start":
    case "tool_execution_update":
    case "tool_execution_end":
      return state; // tool_use/tool_result blocks arrive via message_end frames
    case "message_end": {
      const key = messageKey(event.message);
      if (state.messages.some((m) => messageKey(m) === key)) {
        return state;
      }
      return { ...state, messages: [...state.messages, event.message] };
    }
    case "approval_requested": {
      const entry: PendingApproval = {
        tool_call_id: event.tool_call_id,
        tool_name: event.tool_name,
        args: event.args,
        expires_at: event.expires_at,
      };
      if (state.pendingApprovals.some((a) => a.tool_call_id === entry.tool_call_id)) {
        return state;
      }
      return { ...state, pendingApprovals: [...state.pendingApprovals, entry] };
    }
    case "approval_resolved":
      return {
        ...state,
        pendingApprovals: state.pendingApprovals.filter(
          (a) => a.tool_call_id !== event.tool_call_id,
        ),
      };
    default:
      return state;
  }
}

export function reduce(state: StreamState, events: AgentEvent[]): StreamState {
  return events.reduce(applyEvent, state);
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cd harness/web && npm test`
Expected: PASS — all reducer cases.

- [ ] **Step 5: Commit**

```bash
git add harness/web/src/reducer.ts harness/web/src/reducer.test.ts
git commit -m "feat(harness/web): event reducer with replay-safe approval handling"
```

### Task 17: `useAgentStream` hook

**Files:**

- Create: `harness/web/src/useAgentStream.ts`

- [ ] **Step 1: Implement the hook**

Create `harness/web/src/useAgentStream.ts`:

```typescript
import { useEffect, useReducer } from "react";
import { applyEvent } from "./reducer";
import {
  INITIAL_STREAM_STATE,
  type AgentEvent,
  type StreamState,
} from "./types";

type Action = { kind: "event"; event: AgentEvent } | { kind: "reset" };

function streamReducer(state: StreamState, action: Action): StreamState {
  if (action.kind === "reset") return INITIAL_STREAM_STATE;
  return applyEvent(state, action.event);
}

export function useAgentStream(sessionId: string | null): StreamState {
  const [state, dispatch] = useReducer(streamReducer, INITIAL_STREAM_STATE);

  useEffect(() => {
    if (!sessionId) {
      dispatch({ kind: "reset" });
      return;
    }
    dispatch({ kind: "reset" });
    const url = `/bridge/events?session_id=${encodeURIComponent(sessionId)}`;
    const es = new EventSource(url);
    es.onmessage = (e) => {
      try {
        const data = JSON.parse(e.data) as AgentEvent;
        dispatch({ kind: "event", event: data });
      } catch (err) {
        // Frame could not be parsed; drop it. EventSource passes Last-Event-ID
        // on reconnect so we won't miss anything fixable.
        console.warn("bad SSE frame", err);
      }
    };
    es.onerror = () => {
      // EventSource auto-reconnects; nothing to do unless we want to surface a
      // status pill. Leave as-is for v1.
    };
    return () => es.close();
  }, [sessionId]);

  return state;
}
```

- [ ] **Step 2: Verify the build**

Run: `cd harness/web && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add harness/web/src/useAgentStream.ts
git commit -m "feat(harness/web): useAgentStream hook with EventSource"
```

### Task 18: `ToolUseBlock` component

**Files:**

- Create: `harness/web/src/components/ToolUseBlock.tsx`

- [ ] **Step 1: Implement the component**

Create `harness/web/src/components/ToolUseBlock.tsx`:

```typescript
import { useState } from "react";

interface Props {
  name: string;
  args: unknown;
}

export function ToolUseBlock({ name, args }: Props) {
  const [open, setOpen] = useState(false);
  return (
    <div className="block block-tool-use" data-open={open}>
      <button
        type="button"
        className="block-head"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
      >
        <span className="block-eyebrow">tool</span>
        <span className="block-title">{name}</span>
        <span className="block-toggle">{open ? "−" : "+"}</span>
      </button>
      {open ? (
        <pre className="block-body">{JSON.stringify(args, null, 2)}</pre>
      ) : null}
    </div>
  );
}
```

- [ ] **Step 2: Verify the build**

Run: `cd harness/web && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add harness/web/src/components/ToolUseBlock.tsx
git commit -m "feat(harness/web): ToolUseBlock component"
```

### Task 19: `ToolResultBlock` component

**Files:**

- Create: `harness/web/src/components/ToolResultBlock.tsx`

- [ ] **Step 1: Implement the component**

Create `harness/web/src/components/ToolResultBlock.tsx`:

```typescript
import { useState } from "react";

interface Props {
  toolName: string;
  isError: boolean;
  output: string;
}

const COLLAPSED_LIMIT = 600;

export function ToolResultBlock({ toolName, isError, output }: Props) {
  const [expanded, setExpanded] = useState(false);
  const truncated = output.length > COLLAPSED_LIMIT;
  const visible = expanded || !truncated ? output : output.slice(0, COLLAPSED_LIMIT) + "…";
  return (
    <div className="block block-tool-result" data-error={isError}>
      <header className="block-head">
        <span className="block-eyebrow">{isError ? "error" : "result"}</span>
        <span className="block-title">{toolName}</span>
      </header>
      <pre className="block-body">{visible}</pre>
      {truncated ? (
        <button
          type="button"
          className="block-more"
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "show less" : "show more"}
        </button>
      ) : null}
    </div>
  );
}
```

- [ ] **Step 2: Verify the build**

Run: `cd harness/web && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add harness/web/src/components/ToolResultBlock.tsx
git commit -m "feat(harness/web): ToolResultBlock component"
```

### Task 20: `ApprovalRow` component

**Files:**

- Create: `harness/web/src/components/ApprovalRow.tsx`

- [ ] **Step 1: Implement the component**

Create `harness/web/src/components/ApprovalRow.tsx`:

```typescript
import { useState } from "react";
import { bridge, BridgeError } from "../bridge";
import type { PendingApproval } from "../types";

interface Props {
  sessionId: string;
  pending: PendingApproval[];
}

export function ApprovalRow({ sessionId, pending }: Props) {
  const [busyId, setBusyId] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  if (pending.length === 0) return null;

  const resolve = async (toolCallId: string, decision: "allow" | "deny") => {
    setBusyId(toolCallId);
    setErr(null);
    try {
      await bridge<{ ok: boolean }>("approval::resolve", {
        session_id: sessionId,
        tool_call_id: toolCallId,
        decision,
      });
    } catch (e) {
      setErr(e instanceof BridgeError ? e.message : String(e));
    } finally {
      setBusyId(null);
    }
  };

  return (
    <div className="approvals">
      {pending.map((a) => (
        <div className="approval" key={a.tool_call_id}>
          <div className="approval-head">
            <span className="approval-eyebrow">approval needed</span>
            <span className="approval-title">{a.tool_name}</span>
          </div>
          <pre className="approval-args">{JSON.stringify(a.args, null, 2)}</pre>
          <div className="approval-actions">
            <button
              type="button"
              className="approval-deny"
              disabled={busyId === a.tool_call_id}
              onClick={() => resolve(a.tool_call_id, "deny")}
            >
              deny
            </button>
            <button
              type="button"
              className="approval-allow"
              disabled={busyId === a.tool_call_id}
              onClick={() => resolve(a.tool_call_id, "allow")}
            >
              allow
            </button>
          </div>
        </div>
      ))}
      {err ? (
        <p className="approval-error" role="alert">
          {err}
        </p>
      ) : null}
    </div>
  );
}
```

- [ ] **Step 2: Verify the build**

Run: `cd harness/web && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add harness/web/src/components/ApprovalRow.tsx
git commit -m "feat(harness/web): ApprovalRow component"
```

### Task 21: `SessionView` renders all block types

**Files:**

- Modify: `harness/web/src/components/SessionView.tsx`

- [ ] **Step 1: Replace the body of SessionView**

Replace the contents of `harness/web/src/components/SessionView.tsx` with:

```typescript
import type { AgentMessage } from "../types";
import { ToolUseBlock } from "./ToolUseBlock";
import { ToolResultBlock } from "./ToolResultBlock";

interface Props {
  sessionId: string;
  messages: AgentMessage[];
  loading: boolean;
}

function roleLabel(m: AgentMessage): string {
  if (m.role === "user") return "you";
  if (m.role === "assistant") return m.model ? `${m.model}` : "assistant";
  return "tool";
}

function renderBlocks(m: AgentMessage) {
  return m.content.map((b: any, i: number) => {
    if (b.type === "text") return <p key={i} className="msg-text">{b.text}</p>;
    if (b.type === "tool_use" || b.type === "tool_call") {
      const args = b.input ?? b.arguments ?? {};
      return <ToolUseBlock key={i} name={b.name} args={args} />;
    }
    if (b.type === "tool_result") {
      const text = Array.isArray(b.content)
        ? b.content.map((c: any) => (typeof c === "string" ? c : c.text ?? JSON.stringify(c))).join("\n")
        : typeof b.content === "string"
          ? b.content
          : JSON.stringify(b.content);
      return (
        <ToolResultBlock
          key={i}
          toolName={b.tool_name ?? "tool"}
          isError={Boolean(b.is_error)}
          output={text}
        />
      );
    }
    return null;
  });
}

export function SessionView({ sessionId, messages, loading }: Props) {
  if (!sessionId) {
    return (
      <section className="view view-empty">
        <h2 className="view-empty-h">no session selected</h2>
        <p className="view-empty-p">start a new turn or pick one from the rail.</p>
      </section>
    );
  }
  return (
    <section className="view">
      <header className="view-head">
        <span className="view-eyebrow">session</span>
        <h2 className="view-title">{sessionId}</h2>
      </header>
      <ol className="messages">
        {messages.map((m, i) => (
          <li key={i} className="msg" data-role={m.role}>
            <span className="msg-role">{roleLabel(m)}</span>
            {renderBlocks(m)}
            {m.role === "assistant" && m.usage ? (
              <p className="msg-usage">
                {m.usage.input ?? 0}↓ · {m.usage.output ?? 0}↑ tokens
                {m.stop_reason ? ` · stop: ${m.stop_reason}` : null}
              </p>
            ) : null}
          </li>
        ))}
        {loading ? (
          <li className="msg msg-loading">
            <span className="msg-role">…</span>
            <p className="msg-text">running turn…</p>
          </li>
        ) : null}
      </ol>
    </section>
  );
}
```

- [ ] **Step 2: Verify the build**

Run: `cd harness/web && npx tsc --noEmit`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add harness/web/src/components/SessionView.tsx
git commit -m "feat(harness/web): render tool_use and tool_result blocks in SessionView"
```

### Task 22: Wire `App.tsx` to `run::start` + SSE + tools + approvals

**Files:**

- Modify: `harness/web/src/App.tsx`

- [ ] **Step 1: Apply the four changes**

In `harness/web/src/App.tsx`:

**a.** In every entry of the `TOOLS` const (lines around 30–117), rename the `input_schema` field to `parameters`:

```typescript
const TOOLS = [
  {
    name: "shell::filesystem::ls",
    description: "List directory entries inside the sandbox.",
    parameters: { /* … same shape … */ },
  },
  // …each tool
] as const;
```

**b.** Add an approval-required default near the other constants:

```typescript
const APPROVAL_REQUIRED = [
  "shell::filesystem::write",
  "shell::filesystem::mkdir",
];
```

**c.** Replace the body of `send()` (lines 267–316) with the streaming version:

```typescript
  const send = async (prompt: string) => {
    setError(null);
    const sid = active ?? draftId ?? newSessionId();
    setActive(sid);
    setDraftId(null);
    setLoading(true);

    const optimistic: AgentMessage = {
      role: "user",
      content: [{ type: "text", text: prompt }],
      timestamp: Date.now(),
    };
    const fullHistory = [...messages, optimistic];
    setMessages(fullHistory);

    try {
      await bridge<{ session_id: string }>("run::start", {
        session_id: sid,
        provider,
        model,
        messages: fullHistory,
        system_prompt: buildSystemPrompt(skillsIndex),
        tools: TOOLS,
        approval_required: APPROVAL_REQUIRED,
      });
      void refreshSessions();
    } catch (e) {
      const msg = e instanceof BridgeError ? e.message : String(e);
      setError(msg);
    } finally {
      setLoading(false);
    }
  };
```

**d.** Subscribe the active session to the SSE stream and merge messages from the reducer:

Add near the other hooks at the top of the component:

```typescript
import { useAgentStream } from "./useAgentStream";
import { ApprovalRow } from "./components/ApprovalRow";

// inside App():
const stream = useAgentStream(active);

useEffect(() => {
  if (!active) return;
  if (stream.messages.length > 0) setMessages(stream.messages);
}, [active, stream.messages]);
```

Render `ApprovalRow` immediately above the `Composer` in the chat tab:

```tsx
<ApprovalRow sessionId={active ?? ""} pending={stream.pendingApprovals} />
<Composer disabled={composerDisabled} onSend={send} />
```

The `loading` flag should reflect `stream.status === "running"` once the SSE drives state:

```typescript
const isRunning = loading || stream.status === "running";
// … pass isRunning to <SessionView> instead of `loading`
```

- [ ] **Step 2: Verify the build**

Run: `cd harness/web && npx tsc --noEmit && npm test`
Expected: PASS — type check clean, reducer tests still green.

- [ ] **Step 3: Commit**

```bash
git add harness/web/src/App.tsx
git commit -m "feat(harness/web): run::start + SSE event stream, pass tools, approval_required"
```

---

## Phase 6 — End-to-end

### Task 23: Playwright bootstrap + golden-path approval test

**Files:**

- Modify: `harness/web/package.json`
- Create: `harness/web/playwright.config.ts`
- Create: `harness/web/tests/e2e/approval.spec.ts`

- [ ] **Step 1: Add the dev dependency and script**

In `harness/web/package.json` add:

```json
"devDependencies": {
  "@playwright/test": "^1.49.0"
},
"scripts": {
  "e2e": "playwright test"
}
```

Run: `cd harness/web && npm install && npx playwright install chromium`

- [ ] **Step 2: Create `playwright.config.ts`**

```typescript
import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/e2e",
  timeout: 120_000,
  webServer: {
    command: "npm run dev",
    port: 5173,
    reuseExistingServer: true,
    timeout: 60_000,
  },
  use: {
    baseURL: "http://localhost:5173",
    trace: "retain-on-failure",
  },
});
```

- [ ] **Step 3: Write the approval e2e test**

Create `harness/web/tests/e2e/approval.spec.ts`:

```typescript
import { expect, test } from "@playwright/test";

const PROMPT = "create /tmp/harness-e2e.md with the body 'hi'";

test.describe("approval flow", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("allow path writes the file and renders ToolResultBlock", async ({ page }) => {
    await page.getByPlaceholder(/say something/i).fill(PROMPT);
    await page.getByRole("button", { name: /send/i }).click();
    const approval = page.locator(".approval");
    await expect(approval).toBeVisible({ timeout: 30_000 });
    await page.getByRole("button", { name: "allow" }).click();
    await expect(page.locator(".block-tool-result")).toBeVisible({ timeout: 30_000 });
  });

  test("deny path renders denied tool_result and does not write", async ({ page }) => {
    await page.getByPlaceholder(/say something/i).fill(PROMPT);
    await page.getByRole("button", { name: /send/i }).click();
    await expect(page.locator(".approval")).toBeVisible({ timeout: 30_000 });
    await page.getByRole("button", { name: "deny" }).click();
    const result = page.locator(".block-tool-result[data-error='true']");
    await expect(result).toBeVisible({ timeout: 30_000 });
  });
});
```

- [ ] **Step 4: Document how to run it**

Add to `harness/web/README.md` (create if missing):

```markdown
# harness-web

## Tests
- `npm test` — vitest unit tests (reducer)
- `npm run e2e` — Playwright tests; requires the full demo stack:
  `harness/scripts/demo.sh all`
  before invoking.
```

- [ ] **Step 5: Run the e2e suite (manual)**

Run `harness/scripts/demo.sh all` to bring up the engine + workers, then:

```bash
cd harness/web && npm run e2e
```

Expected: PASS. If running headless in CI, prefer `--reporter=list` and gate behind a job that boots the demo stack.

- [ ] **Step 6: Commit**

```bash
git add harness/web/package.json harness/web/package-lock.json \
        harness/web/playwright.config.ts harness/web/tests/e2e/approval.spec.ts \
        harness/web/README.md
git commit -m "test(harness/web): playwright e2e for allow + deny approval flow"
```

---

## Self-review

**Spec coverage** — every section of the spec has at least one task:

- AgentEvent additions → Task 1.
- `approval_required` plumbing → Tasks 2, 3.
- `approval-gate` worker (helpers, register, integration) → Tasks 4–8.
- `iii.worker.yaml` restore + manifest update → Tasks 9, 10.
- SSE endpoint + integration test → Tasks 11, 12, 13.
- UI types, reducer, hook → Tasks 14–17.
- New UI components → Tasks 18, 19, 20.
- `SessionView` renders all blocks → Task 21.
- `App.tsx` rewire (run::start, SSE, tools, approval_required, schema rename) → Task 22.
- E2E (golden + deny) → Task 23.
- Reconnect smoke is covered at the Rust layer in Task 13 (`harness/tests/sse_bridge.rs`); the spec called for an equivalent Playwright test, but the Rust-side reconnect test is sufficient for v1 and avoids duplicating coverage. (Adding a Playwright reconnect test is a valid follow-up, not blocking.)

**Type consistency** — function names used across tasks: `applyEvent`, `useAgentStream`, `bridge`, `pending_key`, `extract_call`, `await_decision`, `handle_resolve`, `handle_list_pending`, `register`, `pump_events`, `format_frame`. Each is defined exactly once and referenced consistently. Field names are uniform: `tool_call_id`, `tool_name`, `args`, `expires_at`, `decision`, `reason`, `approval_required`.

**No placeholders** — every code step shows actual code; commands have explicit expected output; the only "discovery" step (Task 11) is a discrete task with a concrete output (a doc comment) that gates Task 12.

**Known unknowns:**

- The exact `stream::tail` function id and payload shape — Task 11 surfaces this and Task 12's `pump_events` may need to be edited based on what's found. The plan flags both spots.
- Whether the iii-sdk HTTP trigger supports a streamed response body — also flagged in Task 12. Worst case, the SSE pump becomes a long-poll endpoint (returns up to 5s of buffered frames, client reconnects with `Last-Event-ID`); the UI's `EventSource` works either way because `EventSource` reconnects automatically on body close.

---

## Execution

Plan complete and saved to `docs/superpowers/plans/2026-05-06-harness-agent-loop-ui.md`.
