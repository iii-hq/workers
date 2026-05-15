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

// ─── abort helper round-trip ───────────────────────────────────────────────

#[tokio::test]
#[serial]
async fn abort_helper_set_and_clear_round_trip() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let session_id = format!("abort-rt-{}", common::nonce());

    // Unwritten key reads as false (default).
    assert!(
        !turn_orchestrator::abort::is_set(&h.iii, &session_id).await,
        "fresh session should have abort = false"
    );

    // Manually set via state::set (simulating router::abort).
    let _ = h
        .iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": turn_orchestrator::abort::abort_signal_key(&session_id),
                "value": true,
            }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
    assert!(
        turn_orchestrator::abort::is_set(&h.iii, &session_id).await,
        "abort flag should be true after state::set"
    );

    // Clear, verify.
    turn_orchestrator::abort::clear(&h.iii, &session_id).await;
    assert!(
        !turn_orchestrator::abort::is_set(&h.iii, &session_id).await,
        "abort flag should be false after clear"
    );

    // Clear is idempotent.
    turn_orchestrator::abort::clear(&h.iii, &session_id).await;
    assert!(!turn_orchestrator::abort::is_set(&h.iii, &session_id).await);
}

// ─── state-machine checkpoint tests (drive transitions::step directly) ────
//
// These exercise the abort-flag check at the top of each handler without
// involving the durable subscriber. We seed a record into the desired
// state, set the flag manually, and step the machine forward exactly once.

async fn set_abort(iii: &iii_sdk::III, session_id: &str) {
    let _ = iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": turn_orchestrator::abort::abort_signal_key(session_id),
                "value": true,
            }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;
}

#[tokio::test]
#[serial]
async fn abort_in_awaiting_routes_to_steering_without_burning_turn_count() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let cfg = std::sync::Arc::new(turn_orchestrator::TurnOrchestratorConfig::default());
    let session_id = format!("abort-awaiting-{}", common::nonce());

    let mut record = TurnStateRecord::new(&session_id, None);
    record.transition_to(TurnState::AwaitingAssistant);
    assert_eq!(record.turn_count, 0);

    set_abort(&h.iii, &session_id).await;

    turn_orchestrator::transitions::step(&h.iii, &cfg, &mut record)
        .await
        .expect("step succeeded");

    assert_eq!(
        record.state,
        TurnState::SteeringCheck,
        "abort in awaiting should route to SteeringCheck"
    );
    assert_eq!(
        record.turn_count, 0,
        "abort short-circuit must not burn a turn count"
    );
}

#[tokio::test]
#[serial]
async fn abort_in_prepare_drops_pending_calls_and_routes_to_steering() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let cfg = std::sync::Arc::new(turn_orchestrator::TurnOrchestratorConfig::default());
    let session_id = format!("abort-prepare-{}", common::nonce());

    let mut record = TurnStateRecord::new(&session_id, None);
    record.transition_to(TurnState::FunctionPrepare);
    record.pending_function_calls = vec![
        harness_types::FunctionCall {
            id: "fc-1".into(),
            function_id: "test::fn".into(),
            arguments: json!({}),
        },
        harness_types::FunctionCall {
            id: "fc-2".into(),
            function_id: "test::fn".into(),
            arguments: json!({}),
        },
    ];

    set_abort(&h.iii, &session_id).await;

    turn_orchestrator::transitions::step(&h.iii, &cfg, &mut record)
        .await
        .expect("step succeeded");

    assert_eq!(
        record.state,
        TurnState::SteeringCheck,
        "abort in prepare should route to SteeringCheck"
    );
    assert!(
        record.pending_function_calls.is_empty(),
        "abort in prepare must clear pending_function_calls; still had {:?}",
        record.pending_function_calls
    );
}

#[tokio::test]
#[serial]
async fn abort_in_execute_interrupts_call_loop() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let cfg = std::sync::Arc::new(turn_orchestrator::TurnOrchestratorConfig::default());
    let session_id = format!("abort-execute-{}", common::nonce());

    // Register a noisy test function so we can detect *any* unintended dispatch.
    let dispatch_log: std::sync::Arc<std::sync::Mutex<Vec<Value>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let dispatch_log_h = dispatch_log.clone();
    let _ref = h.iii.register_function((
        RegisterFunctionMessage::with_id("test::stop_should_skip".into())
            .with_description("sink: should never be invoked once abort is set".into()),
        move |payload: Value| {
            let log = dispatch_log_h.clone();
            async move {
                log.lock().unwrap().push(payload);
                Ok::<_, IIIError>(json!({ "ok": true }))
            }
        },
    ));
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Seed prepared_calls under the persistence key so handle_execute loads them.
    let prepared = vec![
        (
            harness_types::FunctionCall {
                id: "fc-skip-1".into(),
                function_id: "test::stop_should_skip".into(),
                arguments: json!({}),
            },
            None::<harness_types::FunctionResult>,
        ),
        (
            harness_types::FunctionCall {
                id: "fc-skip-2".into(),
                function_id: "test::stop_should_skip".into(),
                arguments: json!({}),
            },
            None,
        ),
    ];
    turn_orchestrator::persistence::save_prepared_calls(&h.iii, &session_id, &prepared).await;
    turn_orchestrator::persistence::save_executed_calls(&h.iii, &session_id, &Vec::new()).await;

    let mut record = TurnStateRecord::new(&session_id, None);
    record.transition_to(TurnState::FunctionExecute);

    set_abort(&h.iii, &session_id).await;

    turn_orchestrator::transitions::step(&h.iii, &cfg, &mut record)
        .await
        .expect("step succeeded");

    assert_eq!(
        record.state,
        TurnState::SteeringCheck,
        "abort in execute should route to SteeringCheck"
    );
    assert_eq!(
        dispatch_log.lock().unwrap().len(),
        0,
        "no test function should have been dispatched after abort"
    );
}

#[tokio::test]
#[serial]
async fn provisioning_clears_sticky_abort_flag() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    // Empty default-skills list so provisioning doesn't try directory::skills::get
    // (it would just soft-fail, but skipping keeps the test focused).
    let cfg = std::sync::Arc::new(turn_orchestrator::TurnOrchestratorConfig {
        system_default_skills: Vec::new(),
        ..turn_orchestrator::TurnOrchestratorConfig::default()
    });
    let session_id = format!("clear-on-prov-{}", common::nonce());

    // Seed a run_request so provisioning doesn't choke loading defaults.
    let _ = h
        .iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": turn_orchestrator::run_request_key(&session_id),
                "value": json!({
                    "provider": "test",
                    "model": "test",
                    "system_prompt": "",
                    "approval_required": [],
                }),
            }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;

    set_abort(&h.iii, &session_id).await;
    assert!(turn_orchestrator::abort::is_set(&h.iii, &session_id).await);

    let mut record = TurnStateRecord::new(&session_id, None);
    // record::new() already starts in Provisioning, but make it explicit.
    record.transition_to(TurnState::Provisioning);

    turn_orchestrator::transitions::step(&h.iii, &cfg, &mut record)
        .await
        .expect("step succeeded");

    assert!(
        !turn_orchestrator::abort::is_set(&h.iii, &session_id).await,
        "provisioning must clear the sticky abort flag at the top of every fresh turn"
    );
    assert_eq!(
        record.state,
        TurnState::AwaitingAssistant,
        "provisioning should still advance normally after clearing the flag"
    );
}

#[tokio::test]
#[serial]
async fn user_can_continue_same_session_after_stop() {
    // End-to-end sanity: stop the session, then start a fresh turn on the
    // same session_id. The flag set by `run::stop` must not leak into the
    // new turn — provisioning clears it. Mirrors the behavior we promise
    // users in the chat UI.
    let Some(h) = Harness::boot().await else {
        return;
    };
    let cfg = std::sync::Arc::new(turn_orchestrator::TurnOrchestratorConfig {
        system_default_skills: Vec::new(),
        ..turn_orchestrator::TurnOrchestratorConfig::default()
    });
    let sink = Sink::new();
    register_primitives(&h.iii, &sink).await;
    run_stop::register(&h.iii);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let session_id = format!("continue-{}", common::nonce());

    // --- First turn: seed a record mid-flight, then stop it. ---
    seed_record(&h.iii, &session_id, TurnState::AwaitingAssistant).await;

    let stop_resp = call_run_stop(&h.iii, &session_id).await.unwrap();
    assert_eq!(stop_resp["accepted"], json!(true));
    // run::stop wires through `router::abort` which (in production) writes the
    // flag. Our test sink for `router::abort` is a no-op, so simulate that
    // side-effect here: the contract is "router::abort sets the flag".
    set_abort(&h.iii, &session_id).await;
    assert!(turn_orchestrator::abort::is_set(&h.iii, &session_id).await);

    // --- Second turn: start fresh on the same session_id. ---
    let _ = h
        .iii
        .trigger(TriggerRequest {
            function_id: "state::set".into(),
            payload: json!({
                "scope": STATE_SCOPE,
                "key": turn_orchestrator::run_request_key(&session_id),
                "value": json!({
                    "provider": "test",
                    "model": "test",
                    "system_prompt": "",
                    "approval_required": [],
                }),
            }),
            action: None,
            timeout_ms: Some(2_000),
        })
        .await;

    let mut record = TurnStateRecord::new(&session_id, None);
    turn_orchestrator::transitions::step(&h.iii, &cfg, &mut record)
        .await
        .expect("provisioning step succeeded");

    assert!(
        !turn_orchestrator::abort::is_set(&h.iii, &session_id).await,
        "second-turn provisioning must scrub the abort flag left by the prior stop"
    );
    assert_eq!(
        record.state,
        TurnState::AwaitingAssistant,
        "fresh turn proceeds normally"
    );
}
