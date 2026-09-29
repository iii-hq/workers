//! The one test that needs a real engine: a worker fails, and the group
//! appears.
//!
//! Everything else in this suite runs the worker's logic over a store it
//! controls. This runs the whole thing — the engine's trigger, the durable
//! queue, the `database` worker, the attribution, the frozen evidence — and
//! it is the only place that can catch the class of bug where each piece is
//! right and the wiring between them is not.
//!
//! Connect-or-skip: with no engine it prints why and returns, so CI and a
//! casual `cargo test` stay green. `III_E2E_REQUIRE=1` turns a missing engine
//! into a failure, for a run that means it.
//!
//! ```text
//! III_NAMESPACE=my-project III_E2E_REQUIRE=1 cargo test --test e2e_ingest -- --nocapture
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, IIIClient, InitOptions, RegisterFunction};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

const DEFAULT_URL: &str = "ws://127.0.0.1:49134";
/// A capture goes through a coalesced tick, a queue hop and a transaction.
const DEADLINE: Duration = Duration::from_secs(45);

fn url() -> String {
    std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_URL.into())
}

#[derive(Debug, Clone, Default, Deserialize, serde::Serialize, schemars::JsonSchema)]
struct BoomRequest {
    #[serde(default)]
    marker: String,
    #[serde(default)]
    #[schemars(skip)]
    _caller_worker_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
struct BoomResponse {
    never: bool,
}

/// Connect, or say why the test is being skipped.
async fn connect(name: &str) -> Option<Arc<IIIClient>> {
    let iii = Arc::new(register_worker(
        &url(),
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                version: "0.0.0".into(),
                name: name.into(),
                os: std::env::consts::OS.into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if call(&iii, "engine::workers::list", json!({})).await.is_ok() {
            return Some(iii);
        }
    }
    iii.shutdown_async().await;
    if std::env::var("III_E2E_REQUIRE").is_ok() {
        panic!("no engine at {} and III_E2E_REQUIRE is set", url());
    }
    eprintln!(
        "[skip] no iii engine at {} — set III_E2E_REQUIRE=1 to fail instead",
        url()
    );
    None
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(15_000),
    })
    .await
}

/// Whether the sentinel is up and ingesting on this engine.
async fn sentinel_is_ingesting(iii: &IIIClient) -> bool {
    matches!(
        call(iii, "sentinel::status", json!({})).await,
        Ok(status) if status.get("enabled").and_then(Value::as_bool) == Some(true)
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn a_real_failure_becomes_a_group_attributed_to_the_worker_that_owns_it() {
    let Some(iii) = connect("sentinel-e2e").await else {
        return;
    };
    if !sentinel_is_ingesting(&iii).await {
        eprintln!("[skip] the sentinel is not up and ingesting on this engine");
        iii.shutdown_async().await;
        return;
    }

    // A marker so the assertion cannot match somebody else's failure.
    let marker = format!("e2e-{}", sentinel::ids::now_ms());
    let expected = marker.clone();
    iii.register_function(
        "sentinel-e2e::boom",
        RegisterFunction::new_async(move |request: BoomRequest| {
            let marker = expected.clone();
            async move {
                Err::<BoomResponse, Error>(Error::Handler(format!(
                    "the e2e worker failed on purpose: {} {}",
                    marker, request.marker
                )))
            }
        })
        .description("Fails on purpose, so the sentinel has something to capture."),
    );
    tokio::time::sleep(Duration::from_millis(500)).await;

    // Three failures: one group, three occurrences.
    for _ in 0..3 {
        let outcome = call(&iii, "sentinel-e2e::boom", json!({ "marker": marker })).await;
        assert!(outcome.is_err(), "the e2e function is supposed to fail");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let group = wait_for_group(&iii, &marker)
        .await
        .unwrap_or_else(|| panic!("no group for {marker} within {DEADLINE:?}"));

    assert_eq!(
        group.get("service_name").and_then(Value::as_str),
        Some("sentinel-e2e"),
        "the group belongs to the worker that owns the function, not to whoever called it: {group}"
    );
    assert_eq!(
        group.get("function_id").and_then(Value::as_str),
        Some("sentinel-e2e::boom"),
        "{group}"
    );
    assert!(
        group
            .get("occurrence_count")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            >= 1,
        "{group}"
    );

    // The evidence was frozen at capture, not fetched now.
    let group_id = group
        .get("id")
        .and_then(Value::as_str)
        .expect("the group has an id");
    let detail = call(
        &iii,
        "sentinel::groups::get",
        json!({ "group_id": group_id }),
    )
    .await
    .expect("the group reads");
    let occurrence_id = detail["latest_occurrence"]["id"]
        .as_str()
        .expect("the group has an occurrence");
    let evidence = call(
        &iii,
        "sentinel::evidence::get",
        json!({ "occurrence_id": occurrence_id }),
    )
    .await
    .expect("the evidence reads");
    assert_eq!(
        evidence.get("pruned").and_then(Value::as_bool),
        Some(false),
        "the bundle is still there: {evidence}"
    );
    let origin = evidence["evidence"]["origin_span_id"]
        .as_str()
        .expect("the bundle names its origin span");
    let spans = evidence["evidence"]["spans"]
        .as_array()
        .expect("the bundle carries spans");
    let origin_span = spans
        .iter()
        .find(|span| span["span_id"].as_str() == Some(origin))
        .expect("the origin span is in the bundle");
    assert_eq!(
        origin_span["name"].as_str(),
        Some("execute sentinel-e2e::boom"),
        "the origin is the leaf that failed: {origin_span}"
    );

    // And the monitor did not monitor itself while doing all of that.
    let groups = call(
        &iii,
        "sentinel::groups::list",
        json!({ "status": [], "limit": 200 }),
    )
    .await
    .expect("the list reads");
    let own: Vec<&Value> = groups["groups"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter(|group| {
                    matches!(
                        group["service_name"].as_str(),
                        Some("sentinel") | Some("database") | Some("queue")
                    )
                })
                .collect()
        })
        .unwrap_or_default();
    assert!(
        own.is_empty(),
        "the sentinel's own pipeline must never become a group: {own:?}"
    );

    iii.shutdown_async().await;
}

/// Poll the list until the group shows up, or give up.
async fn wait_for_group(iii: &IIIClient, marker: &str) -> Option<Value> {
    let deadline = Instant::now() + DEADLINE;
    while Instant::now() < deadline {
        if let Ok(response) = call(
            iii,
            "sentinel::groups::list",
            json!({ "search": marker, "status": [], "limit": 10 }),
        )
        .await
        {
            if let Some(group) = response["groups"]
                .as_array()
                .and_then(|groups| groups.first())
            {
                return Some(group.clone());
            }
        }
        tokio::time::sleep(Duration::from_millis(1000)).await;
    }
    None
}
