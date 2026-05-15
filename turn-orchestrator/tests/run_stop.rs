//! `run::stop` lifecycle: idempotency, primitives invoked, payload validation.
//!
//! Boots a minimal iii engine + session worker. The state-machine-driven
//! cases (stop-during-streaming, stop-during-execute) require a fully
//! orchestrated turn loop with provider/sandbox mocks and are exercised
//! end-to-end during manual QA per the plan's Verification section.
//! Here we cover the contract `run::stop` directly exposes.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::time::timeout;

use common::Harness;
use turn_orchestrator::{run_stop, TurnState, TurnStateRecord};

const STATE_SCOPE: &str = "agent";

/// Sink for `router::abort` and `approval::sweep_session` invocations.
#[derive(Default, Clone)]
struct Sink {
    abort_calls: Arc<Mutex<Vec<Value>>>,
    sweep_calls: Arc<Mutex<Vec<Value>>>,
    step_publishes: Arc<Mutex<Vec<Value>>>,
}

impl Sink {
    fn new() -> Self {
        Self::default()
    }
    fn abort(&self) -> Vec<Value> {
        self.abort_calls.lock().unwrap().clone()
    }
    fn sweep(&self) -> Vec<Value> {
        self.sweep_calls.lock().unwrap().clone()
    }
}

async fn register_primitives(iii: &iii_sdk::III, sink: &Sink) {
    let abort_log = sink.abort_calls.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("router::abort".to_string())
            .with_description("test sink: capture run::stop's abort flag write".into()),
        move |payload: Value| {
            let log = abort_log.clone();
            async move {
                log.lock().unwrap().push(payload);
                Ok::<_, IIIError>(json!({ "ok": true }))
            }
        },
    ));

    let sweep_log = sink.sweep_calls.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("approval::sweep_session".to_string())
            .with_description("test sink: capture approval sweep".into()),
        move |payload: Value| {
            let log = sweep_log.clone();
            async move {
                log.lock().unwrap().push(payload);
                Ok::<_, IIIError>(json!({ "ok": true, "swept": 0 }))
            }
        },
    ));

    let step_log = sink.step_publishes.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("iii::durable::publish".to_string())
            .with_description("test sink: capture turn::step publishes".into()),
        move |payload: Value| {
            let log = step_log.clone();
            async move {
                log.lock().unwrap().push(payload);
                Ok::<_, IIIError>(json!({ "ok": true }))
            }
        },
    ));

    // Allow registrations to settle on the engine. 200ms is not always enough
    // when the engine is busy; 800ms matches what `Harness::boot` already
    // waits for the session worker.
    tokio::time::sleep(Duration::from_millis(800)).await;

    // Sanity-probe each registration so a slow engine doesn't leave us
    // chasing a phantom failure later. If any probe errors, fail loudly.
    for fn_id in ["router::abort", "approval::sweep_session"] {
        iii.trigger(TriggerRequest {
            function_id: fn_id.into(),
            payload: json!({ "session_id": "__probe__" }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await
        .unwrap_or_else(|e| panic!("test sink {fn_id} not reachable: {e}"));
    }

    // Probes count as calls — wipe the sink before tests assert behavior.
    sink.abort_calls.lock().unwrap().clear();
    sink.sweep_calls.lock().unwrap().clear();
    sink.step_publishes.lock().unwrap().clear();
}

async fn seed_record(iii: &iii_sdk::III, session_id: &str, state: TurnState) {
    let mut rec = TurnStateRecord::new(session_id, None);
    rec.transition_to(state);
    let value = serde_json::to_value(&rec).expect("serialize record");
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": turn_orchestrator::turn_state_key(session_id),
                "value": value,
            }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
}

async fn call_run_stop(iii: &iii_sdk::III, session_id: &str) -> Result<Value, IIIError> {
    iii.trigger(TriggerRequest {
        function_id: run_stop::FUNCTION_ID.into(),
        payload: json!({ "session_id": session_id }),
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
}

#[tokio::test]
#[serial]
async fn run_stop_requires_session_id() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    run_stop::register(&h.iii);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let r = h
        .iii
        .trigger(TriggerRequest {
            function_id: run_stop::FUNCTION_ID.into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
    assert!(
        r.is_err(),
        "expected handler error for missing session_id, got {:?}",
        r
    );
}

#[tokio::test]
#[serial]
async fn run_stop_returns_no_record_for_unknown_session() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let sink = Sink::new();
    register_primitives(&h.iii, &sink).await;
    run_stop::register(&h.iii);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let session_id = format!("nonexistent-{}", common::nonce());
    let resp = timeout(Duration::from_secs(3), call_run_stop(&h.iii, &session_id))
        .await
        .expect("run::stop responded in time")
        .expect("run::stop succeeded");

    assert_eq!(resp["accepted"], json!(false));
    assert_eq!(resp["reason"], json!("no_record"));
    assert!(
        sink.abort().is_empty(),
        "router::abort should not be invoked for unknown session"
    );
    assert!(
        sink.sweep().is_empty(),
        "approval::sweep_session should not be invoked for unknown session"
    );
}

#[tokio::test]
#[serial]
async fn run_stop_short_circuits_when_already_stopped() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let sink = Sink::new();
    register_primitives(&h.iii, &sink).await;
    run_stop::register(&h.iii);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let session_id = format!("already-stopped-{}", common::nonce());
    seed_record(&h.iii, &session_id, TurnState::Stopped).await;

    let resp = timeout(Duration::from_secs(3), call_run_stop(&h.iii, &session_id))
        .await
        .expect("run::stop responded in time")
        .expect("run::stop succeeded");

    assert_eq!(resp["accepted"], json!(false));
    assert_eq!(resp["reason"], json!("already_stopped"));
    assert!(
        sink.abort().is_empty(),
        "router::abort should not be invoked when already stopped"
    );
    assert!(
        sink.sweep().is_empty(),
        "approval::sweep_session should not be invoked when already stopped"
    );
}

#[tokio::test]
#[serial]
async fn run_stop_invokes_all_primitives_on_happy_path() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let sink = Sink::new();
    register_primitives(&h.iii, &sink).await;
    run_stop::register(&h.iii);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let session_id = format!("happy-{}", common::nonce());
    seed_record(&h.iii, &session_id, TurnState::AwaitingAssistant).await;

    let resp = timeout(Duration::from_secs(3), call_run_stop(&h.iii, &session_id))
        .await
        .expect("run::stop responded in time")
        .expect("run::stop succeeded");

    assert_eq!(resp["accepted"], json!(true));
    assert_eq!(resp["prior_state"], json!("awaiting_assistant"));

    let abort_calls = sink.abort();
    assert_eq!(abort_calls.len(), 1, "router::abort invoked once");
    assert_eq!(abort_calls[0]["session_id"], json!(session_id));

    let sweep_calls = sink.sweep();
    assert_eq!(sweep_calls.len(), 1, "approval::sweep_session invoked once");
    assert_eq!(sweep_calls[0]["session_id"], json!(session_id));
    assert_eq!(sweep_calls[0]["reason"], json!("run_stopped"));
}

#[tokio::test]
#[serial]
async fn run_stop_is_idempotent_when_repeated() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let sink = Sink::new();
    register_primitives(&h.iii, &sink).await;
    run_stop::register(&h.iii);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let session_id = format!("double-{}", common::nonce());
    seed_record(&h.iii, &session_id, TurnState::AwaitingAssistant).await;

    let r1 = call_run_stop(&h.iii, &session_id).await.unwrap();
    assert_eq!(r1["accepted"], json!(true));

    // Simulate the orchestrator's teardown by manually transitioning to Stopped.
    seed_record(&h.iii, &session_id, TurnState::Stopped).await;

    let r2 = call_run_stop(&h.iii, &session_id).await.unwrap();
    assert_eq!(r2["accepted"], json!(false));
    assert_eq!(r2["reason"], json!("already_stopped"));

    // Only the first call should have invoked the primitives.
    assert_eq!(sink.abort().len(), 1);
    assert_eq!(sink.sweep().len(), 1);
}
