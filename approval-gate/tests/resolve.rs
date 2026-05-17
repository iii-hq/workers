//! Direct `approval::resolve` branch coverage. The lifecycle tests cover
//! the happy paths; these tests pin the wire errors and terminal-state
//! guards that protect duplicate execution.

mod common;

use std::sync::{Mutex, RwLock};

use approval_gate::record::{Outcome, Record, Status};
use approval_gate::{handle_resolve, pending_key, LayeredRules, StateBus, STATE_SCOPE};
use common::{FakeExecutor, InMemoryStateBus};
use serde_json::{json, Value};

fn pending(session: &str, cid: &str) -> Record {
    Record::pending(
        cid.into(),
        "shell::exec".into(),
        json!({"command": "echo", "args": ["ok"]}),
        session.into(),
        1_000,
        60_000,
    )
}

async fn seed(bus: &InMemoryStateBus, record: Record) {
    let key = pending_key(&record.session_id, &record.function_call_id);
    bus.set(STATE_SCOPE, &key, record.to_value()).await.unwrap();
}

fn empty_rules() -> RwLock<LayeredRules> {
    RwLock::new(LayeredRules::default())
}

#[tokio::test]
async fn resolve_rejects_missing_id_bad_decision_and_not_found() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    let rules = empty_rules();

    let missing = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s"}),
        2_000,
    )
    .await;
    assert_eq!(missing["error"], "missing_id");

    let bad_decision = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc", "decision": "maybe"}),
        2_000,
    )
    .await;
    assert_eq!(bad_decision["error"], "bad_decision");

    let not_found = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc", "decision": "allow"}),
        2_000,
    )
    .await;
    assert_eq!(not_found["error"], "not_found");
    assert!(exec.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resolve_rejects_corrupt_in_flight_done_and_bad_denial_rows() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    let rules = empty_rules();

    bus.set(STATE_SCOPE, "s/tc-corrupt", json!({"status": "pending"}))
        .await
        .unwrap();
    let corrupt = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc-corrupt", "decision": "allow"}),
        2_000,
    )
    .await;
    assert_eq!(corrupt["error"], "corrupt_record");

    seed(&bus, pending("s", "tc-inflight").in_flight(2_000)).await;
    let in_flight = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc-inflight", "decision": "allow"}),
        2_500,
    )
    .await;
    assert_eq!(in_flight["error"], "in_flight");

    seed(
        &bus,
        pending("s", "tc-done").done_at(2_000, Outcome::TimedOut),
    )
    .await;
    let done = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc-done", "decision": "allow"}),
        2_500,
    )
    .await;
    assert_eq!(done["error"], "already_resolved");

    seed(&bus, pending("s", "tc-bad-denial")).await;
    let bad_denial = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({
            "session_id": "s",
            "function_call_id": "tc-bad-denial",
            "decision": "deny",
            "denial": { "kind": "not_real" },
        }),
        2_500,
    )
    .await;
    assert_eq!(bad_denial["error"], "bad_denial");
    assert!(exec.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn resolve_flips_expired_pending_and_records_failed_invocation() {
    let bus = InMemoryStateBus::new();
    let exec = FakeExecutor::default();
    let rules = empty_rules();

    seed(&bus, pending("s", "tc-expired")).await;
    let timed_out = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc-expired", "decision": "allow"}),
        70_000,
    )
    .await;
    assert_eq!(timed_out["error"], "timed_out");
    let expired = bus
        .get(STATE_SCOPE, "s/tc-expired")
        .await
        .and_then(Record::from_value)
        .unwrap();
    assert_eq!(expired.status, Status::Done);
    assert!(matches!(expired.outcome, Some(Outcome::TimedOut)));

    *exec.response.lock().unwrap() = Some(Err("spawn failed".into()));
    seed(&bus, pending("s", "tc-fails")).await;
    let failed = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc-fails", "decision": "allow"}),
        2_000,
    )
    .await;
    assert_eq!(failed["ok"], true);
    let record = bus
        .get(STATE_SCOPE, "s/tc-fails")
        .await
        .and_then(Record::from_value)
        .unwrap();
    assert_eq!(record.status, Status::Done);
    assert!(
        matches!(record.outcome, Some(Outcome::Failed { ref error }) if error == "spawn failed")
    );
    assert_eq!(exec.calls.lock().unwrap().len(), 1);
}

struct ReturningFailSetBus {
    value: Mutex<Value>,
}

#[async_trait::async_trait]
impl StateBus for ReturningFailSetBus {
    async fn set(&self, _: &str, _: &str, _: Value) -> Result<(), iii_sdk::IIIError> {
        Err(iii_sdk::IIIError::Runtime("kv write failed".into()))
    }

    async fn get(&self, _: &str, _: &str) -> Option<Value> {
        Some(self.value.lock().unwrap().clone())
    }

    async fn list_prefix(&self, _: &str, _: &str) -> Vec<Value> {
        Vec::new()
    }

    async fn delete(&self, _: &str, _: &str) -> Result<bool, iii_sdk::IIIError> {
        Ok(false)
    }
}

#[tokio::test]
async fn resolve_fails_closed_when_state_write_fails() {
    let exec = FakeExecutor::default();
    let rules = empty_rules();
    let bus = ReturningFailSetBus {
        value: Mutex::new(pending("s", "tc").to_value()),
    };

    let denied = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc", "decision": "deny"}),
        2_000,
    )
    .await;
    assert_eq!(denied["error"], "state_write_failed");

    let allowed = handle_resolve(
        &bus,
        &exec,
        STATE_SCOPE,
        &rules,
        json!({"session_id": "s", "function_call_id": "tc", "decision": "allow"}),
        2_000,
    )
    .await;
    assert_eq!(allowed["error"], "state_write_failed");
    assert!(exec.calls.lock().unwrap().is_empty());
}
