//! `github::on-called`: private, its handler returns the [`CalledEvent`], and
//! `run_and_emit` calls it fire-and-forget after a `github::*` function runs,
//! only while something observes it.

#[path = "support/webhook_engine.rs"]
#[allow(dead_code)]
mod engine;
#[path = "support/real_engine.rs"]
mod real_engine;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use engine::{wait, Engine};
use github::events::{register_on_called, run_and_emit, CalledEvent, ON_CALLED_FN};
use iii_sdk::{Deliver, Error, RegisterFunction, Source};
use real_engine::RealEngine;
use serde_json::{json, Value};

const TARGET: &str = "console::github-activity";

fn schema_of_called_event() -> Value {
    serde_json::to_value(
        schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<CalledEvent>(),
    )
    .unwrap()
}

/// The `invokefunction` frames the worker sent for `github::on-called`.
fn on_called_calls(engine: &Engine) -> Vec<Value> {
    engine
        .frames()
        .into_iter()
        .filter(|f| f["type"] == "invokefunction" && f["function_id"] == ON_CALLED_FN)
        .collect()
}

async fn list_prs() -> Result<Value, Error> {
    Ok(json!({ "value": [{ "number": 1 }, { "number": 2 }] }))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_called_is_private_and_called_once_per_run_while_observed() {
    let mut engine = Engine::start().await;
    let iii = engine.client("github").await;
    let console = engine.client("console").await;
    let emitter = register_on_called(&iii);
    engine.function("github", ON_CALLED_FN).await;

    let registration = engine
        .frames()
        .into_iter()
        .find(|f| f["type"] == "registerfunction" && f["id"] == ON_CALLED_FN)
        .unwrap();
    assert_eq!(registration["private"], true);
    assert!(registration["args_format"].is_null(), "takes no args");
    assert_eq!(registration["request_format"], schema_of_called_event());
    assert_eq!(registration["response_format"], schema_of_called_event());

    // Nothing observes it: the wrapped call returns as is and nothing is
    // called.
    let out = run_and_emit(
        &emitter,
        "github::pr::list",
        "pr list".into(),
        None,
        list_prs(),
    )
    .await
    .unwrap();
    assert_eq!(out, list_prs().await.unwrap());
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(on_called_calls(&engine).is_empty());

    console.register_function(
        TARGET,
        RegisterFunction::new(|_: Value| Ok::<_, Error>(Value::Null)),
    );
    engine.function("console", TARGET).await;
    let observer = console.observe(
        Source::function(ON_CALLED_FN)
            .namespace("github")
            .deliver(Deliver::Output),
        TARGET,
    );
    tokio::time::timeout(Duration::from_secs(10), observer.ready())
        .await
        .expect("observer answered")
        .expect("github::on-called accepts an observer without args");
    wait(|| iii.is_observed(ON_CALLED_FN)).await;

    // Observed: one fire-and-forget call per run, carrying the event.
    run_and_emit(
        &emitter,
        "github::pr::list",
        "pr list --repo=o/r".into(),
        Some("o/r".into()),
        list_prs(),
    )
    .await
    .unwrap();
    wait(|| !on_called_calls(&engine).is_empty()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let calls = on_called_calls(&engine);
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0]["invocation_id"].is_null(), "a void call");
    let event: CalledEvent = serde_json::from_value(calls[0]["data"].clone()).unwrap();
    assert_eq!(event.function_id, "github::pr::list");
    assert_eq!(event.repo.as_deref(), Some("o/r"));
    assert!(event.ok);
    assert_eq!(event.kind, "list");

    // Only the github worker calls it.
    let refused = console
        .call(
            iii_sdk::CallRequest::new(ON_CALLED_FN, serde_json::to_value(&event).unwrap())
                .namespace("github")
                .timeout(Duration::from_secs(5)),
        )
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("FORBIDDEN"), "{refused}");
    engine.shutdown().await;
}

/// Through a real engine: an observer with `deliver: output` and a filter on
/// `input.function_id` receives the event of each matching run once, and a
/// slow observer never holds the run back.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_observed_run_reaches_its_observer_once() {
    let Some(engine) = RealEngine::start().await else {
        eprintln!("skipping: set III_ENGINE_BIN to run against a real engine");
        return;
    };
    let iii = engine.client("github").await;
    let console = engine.client("console").await;
    let emitter = register_on_called(&iii);
    let row = engine.function(&console, ON_CALLED_FN).await;
    assert_eq!(row["private"], true, "{row}");
    assert_eq!(row["response_schema"]["title"], "CalledEvent", "{row}");
    let refused = real_engine::call(&console, ON_CALLED_FN, json!({}))
        .await
        .unwrap_err();
    assert!(refused.to_string().contains("FORBIDDEN"), "{refused}");

    let received: Arc<Mutex<Vec<Value>>> = Arc::default();
    let log = received.clone();
    console.register_function(
        TARGET,
        RegisterFunction::new_async(move |event: Value| {
            let log = log.clone();
            async move {
                log.lock().unwrap().push(event);
                // A slow observer: the run it observes does not wait for it.
                tokio::time::sleep(Duration::from_secs(2)).await;
                Ok::<_, Error>(Value::Null)
            }
        }),
    );
    engine.function(&console, TARGET).await;
    let observer = console.observe(
        Source::function(ON_CALLED_FN)
            .namespace("default")
            .filters(json!({ "input.function_id": { "$eq": "github::pr::list" } }))
            .deliver(Deliver::Output),
        TARGET,
    );
    tokio::time::timeout(Duration::from_secs(10), observer.ready())
        .await
        .expect("observer answered")
        .expect("observer accepted");
    tokio::time::timeout(Duration::from_secs(10), async {
        while !iii.is_observed(ON_CALLED_FN) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the host sees the observer");

    let started = std::time::Instant::now();
    run_and_emit(
        &emitter,
        "github::pr::view",
        "pr view 1".into(),
        Some("o/r".into()),
        async { Ok::<_, Error>(json!({ "value": { "number": 1 } })) },
    )
    .await
    .unwrap();
    run_and_emit(
        &emitter,
        "github::pr::list",
        "pr list --repo=o/r".into(),
        Some("o/r".into()),
        list_prs(),
    )
    .await
    .unwrap();
    assert!(started.elapsed() < Duration::from_secs(1));

    tokio::time::timeout(Duration::from_secs(10), async {
        while received.lock().unwrap().is_empty() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the observer receives the event");
    tokio::time::sleep(Duration::from_millis(500)).await;
    let events = received.lock().unwrap().clone();
    assert_eq!(events.len(), 1, "{events:?}");
    let event: CalledEvent = serde_json::from_value(events[0].clone()).unwrap();
    assert_eq!(event.function_id, "github::pr::list");
    assert_eq!(event.result_summary, "2 pull requests");

    console.shutdown_async().await;
    iii.shutdown_async().await;
}
