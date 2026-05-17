//! `run::resume` contract tests. These cover the registered behavior that
//! the pure `build_resume_record` unit tests cannot: persisted terminal
//! records are reset and the durable subscriber is nudged.

mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunctionMessage};
use serde_json::{json, Value};
use serial_test::serial;
use tokio::time::timeout;

use common::Harness;
use turn_orchestrator::{persistence, run_start, TurnState, TurnStateRecord};

#[derive(Default, Clone)]
struct StepSink {
    publishes: Arc<Mutex<Vec<Value>>>,
}

impl StepSink {
    fn publishes(&self) -> Vec<Value> {
        self.publishes.lock().unwrap().clone()
    }
}

async fn register_durable_publish(iii: &iii_sdk::III, sink: &StepSink) {
    let log = sink.publishes.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id("iii::durable::publish".to_string())
            .with_description("test sink: capture run::resume's step publish".into()),
        move |payload: Value| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(payload);
                Ok::<_, IIIError>(json!({ "ok": true }))
            }
        },
    ));
    tokio::time::sleep(Duration::from_millis(300)).await;
}

async fn call_resume(iii: &iii_sdk::III, session_id: &str) -> Result<Value, IIIError> {
    run_start::execute_resume(
        iii.clone(),
        json!({
            "session_id": session_id,
        }),
    )
    .await
}

#[tokio::test]
#[serial]
async fn run_resume_resets_terminal_record_and_publishes_step() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let sink = StepSink::default();
    register_durable_publish(&h.iii, &sink).await;

    let session_id = format!("resume-stopped-{}", common::nonce());
    let mut stopped = TurnStateRecord::new(&session_id, Some(7));
    stopped.turn_count = 3;
    stopped.transition_to(TurnState::Stopped);
    persistence::save_record(&h.iii, &stopped).await;

    let resp = timeout(Duration::from_secs(3), call_resume(&h.iii, &session_id))
        .await
        .expect("run::resume responded in time")
        .expect("run::resume succeeded");

    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["resumed"], json!(true));
    assert_eq!(resp["session_id"], json!(session_id));

    let saved = persistence::load_record(&h.iii, &session_id)
        .await
        .expect("resume must save a replacement record");
    assert_eq!(saved.state, TurnState::Provisioning);
    assert_eq!(saved.max_turns, Some(7));
    assert_eq!(saved.turn_count, 0);
    assert!(!saved.is_terminal());

    let publishes = sink.publishes();
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0]["topic"], json!(run_start::STEP_TOPIC));
    assert_eq!(publishes[0]["payload"]["session_id"], json!(session_id));
}

#[tokio::test]
#[serial]
async fn run_resume_rekicks_active_record_without_resetting_state() {
    let Some(h) = Harness::boot().await else {
        return;
    };
    let sink = StepSink::default();
    register_durable_publish(&h.iii, &sink).await;

    let session_id = format!("resume-active-{}", common::nonce());
    let mut active = TurnStateRecord::new(&session_id, Some(5));
    active.turn_count = 2;
    active.transition_to(TurnState::AwaitingAssistant);
    persistence::save_record(&h.iii, &active).await;

    let resp = timeout(Duration::from_secs(3), call_resume(&h.iii, &session_id))
        .await
        .expect("run::resume responded in time")
        .expect("run::resume succeeded");

    assert_eq!(resp["ok"], json!(true));
    assert_eq!(resp["resumed"], json!(false));

    let saved = persistence::load_record(&h.iii, &session_id)
        .await
        .expect("active record should still exist");
    assert_eq!(saved.state, TurnState::AwaitingAssistant);
    assert_eq!(saved.turn_count, 2);
    assert_eq!(saved.max_turns, Some(5));

    let publishes = sink.publishes();
    assert_eq!(publishes.len(), 1);
    assert_eq!(publishes[0]["topic"], json!(run_start::STEP_TOPIC));
    assert_eq!(publishes[0]["payload"]["session_id"], json!(session_id));
}
