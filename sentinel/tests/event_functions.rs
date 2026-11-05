//! The event functions on a live engine. A consumer client observes
//! `sentinel::on-group-changed` and `sentinel::on-investigation-changed`,
//! the worker's [`Emitter`] calls them from a provider client, and the test
//! checks what the consumer receives:
//!
//! - every observed call is delivered, with the event as its output;
//! - occurrence events are merged per group (the newest wins) and delivered
//!   at the end of the one-second window;
//! - a creation or a state change is delivered at once, and takes the place
//!   of the occurrence gathered for its group.
//!
//! Connect-or-skip, like `e2e_ingest`: `III_URL` names the engine (default
//! `ws://127.0.0.1:49134`), and `III_E2E_REQUIRE=1` turns a missing engine
//! into a failure.
//!
//! ```text
//! III_URL=ws://127.0.0.1:49134 III_E2E_REQUIRE=1 cargo test --test event_functions
//! ```

use std::sync::Arc;
use std::time::{Duration, Instant};

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, IIIClient, InitOptions, ObserverHandle,
    RegisterFunction, Source,
};
use sentinel::events::{self, group_event, ON_GROUP_CHANGED_FN, ON_INVESTIGATION_CHANGED_FN};
use sentinel::{
    Emitter, GroupChangeReasonV1, GroupChangedEventV1, GroupChangedOpV1, GroupStatusV1,
    InvestigationChangedEventV1, InvestigationChangedOpV1, InvestigationStatusV1,
};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const DEFAULT_URL: &str = "ws://127.0.0.1:49134";
const GROUP_RECORDER: &str = "sentinel-events-test::on-group";
const INVESTIGATION_RECORDER: &str = "sentinel-events-test::on-investigation";
/// Where in the emitter's one-second window a step emits: well before the
/// flush at its end, so an immediate delivery and a coalesced one cannot be
/// mistaken for each other.
const EMIT_AT: Duration = Duration::from_millis(200);
/// How long a step listens: past the flush that ends its window and through
/// the whole next window, so a second (unexpected) delivery would show.
const LISTEN: Duration = Duration::from_millis(2500);

fn url() -> String {
    std::env::var("III_URL").unwrap_or_else(|_| DEFAULT_URL.into())
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
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await;
        if probe.is_ok() {
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

type Received = mpsc::UnboundedReceiver<(Instant, Value)>;

/// Register a function on `consumer` that records what it is called with.
fn recorder(consumer: &IIIClient, function_id: &str) -> Received {
    let (tx, rx) = mpsc::unbounded_channel();
    consumer.register_function(
        function_id,
        RegisterFunction::new(move |payload: Value| -> Result<Value, iii_sdk::Error> {
            let _ = tx.send((Instant::now(), payload));
            Ok(Value::Null)
        }),
    );
    rx
}

async fn await_active(handle: &ObserverHandle, function_id: &str) {
    match tokio::time::timeout(Duration::from_secs(10), handle.ready()).await {
        Ok(Ok(())) => {}
        Ok(Err(e)) => panic!("observer of {function_id} refused: {e}"),
        Err(_) => panic!(
            "observer of {function_id} not active after 10 s: {:?} ({:?})",
            handle.state(),
            handle.reason()
        ),
    }
}

async fn await_observed(provider: &IIIClient, function_id: &str, observed: bool) {
    for _ in 0..100 {
        if provider.is_observed(function_id) == observed {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the provider never saw {function_id} observed = {observed}");
}

/// Sleep until [`EMIT_AT`] into the emitter's current window. Its flusher
/// ticks once when the emitter is built and then every second.
async fn into_window(started: Instant) {
    let window = Duration::from_secs(1);
    let into = Duration::from_nanos((started.elapsed().as_nanos() % window.as_nanos()) as u64);
    let wait = if into <= EMIT_AT {
        EMIT_AT - into
    } else {
        window - into + EMIT_AT
    };
    tokio::time::sleep(wait).await;
}

/// Everything delivered within [`LISTEN`].
async fn listen(rx: &mut Received) -> Vec<(Instant, Value)> {
    let deadline = tokio::time::Instant::now() + LISTEN;
    let mut got = Vec::new();
    while let Ok(Some(item)) = tokio::time::timeout_at(deadline, rx.recv()).await {
        got.push(item);
    }
    got
}

fn value(event: &GroupChangedEventV1) -> Value {
    serde_json::to_value(event).expect("serialize the group event")
}

fn occurrence(group_id: &str, count: u64) -> GroupChangedEventV1 {
    group_event(
        GroupChangedOpV1::Occurrence,
        group_id,
        GroupStatusV1::New,
        count,
        None,
    )
}

#[tokio::test(flavor = "multi_thread")]
async fn observed_event_functions_deliver_and_coalesce_occurrences() {
    let Some(provider) = connect("sentinel-events-provider").await else {
        return;
    };
    let Some(consumer) = connect("sentinel-events-consumer").await else {
        provider.shutdown_async().await;
        return;
    };
    events::register_functions(&provider);

    // Group ids of this run only, so nothing else on the engine is counted.
    let tag = sentinel::ids::now_ms();
    let [a, b, c, d] = ["a", "b", "c", "d"].map(|g| format!("grp_events_{tag}_{g}"));

    let mut groups = recorder(&consumer, GROUP_RECORDER);
    let mut investigations = recorder(&consumer, INVESTIGATION_RECORDER);
    let group_observer = consumer.observe(
        Source::function(ON_GROUP_CHANGED_FN)
            .filters(json!({ "input.group_id": { "$in": [&a, &b, &c, &d] } }))
            .deliver(Deliver::Output),
        GROUP_RECORDER,
    );
    let investigation_observer = consumer.observe(
        Source::function(ON_INVESTIGATION_CHANGED_FN)
            .filters(json!({ "input.group_id": { "$eq": &a } }))
            .deliver(Deliver::Output),
        INVESTIGATION_RECORDER,
    );
    await_active(&group_observer, ON_GROUP_CHANGED_FN).await;
    await_active(&investigation_observer, ON_INVESTIGATION_CHANGED_FN).await;
    await_observed(&provider, ON_GROUP_CHANGED_FN, true).await;
    await_observed(&provider, ON_INVESTIGATION_CHANGED_FN, true).await;

    let emitter = Emitter::new(provider.clone());
    let started = Instant::now();

    // Occurrences of two groups are gathered; a creation between them goes
    // out at once, ahead of the flush.
    into_window(started).await;
    let created = group_event(GroupChangedOpV1::Created, &a, GroupStatusV1::New, 1, None);
    let emitted = Instant::now();
    emitter.group_changed(occurrence(&b, 2)).await;
    emitter.group_changed(occurrence(&b, 3)).await;
    emitter.group_changed(occurrence(&c, 7)).await;
    emitter.group_changed(created.clone()).await;
    emitter.group_changed(occurrence(&b, 4)).await;
    let got = listen(&mut groups).await;
    let payloads: Vec<&Value> = got.iter().map(|(_, v)| v).collect();
    assert_eq!(
        got.len(),
        3,
        "one creation and one occurrence per group: {payloads:?}"
    );
    assert_eq!(got[0].1, value(&created), "the creation is delivered first");
    let mut flushed: Vec<Value> = got[1..].iter().map(|(_, v)| v.clone()).collect();
    flushed.sort_by_key(|v| v["group_id"].as_str().unwrap_or_default().to_owned());
    assert_eq!(
        flushed,
        vec![value(&occurrence(&b, 4)), value(&occurrence(&c, 7))],
        "occurrences merge per group and the newest count wins"
    );
    for (at, event) in &got[1..] {
        assert!(
            at.duration_since(emitted) >= Duration::from_millis(400),
            "an occurrence waits for the end of its window: {event} after {:?}",
            at.duration_since(emitted)
        );
    }

    // A state change goes out at once and replaces the occurrence gathered
    // for its group; another group's occurrence still waits for the flush.
    into_window(started).await;
    let resolved = GroupChangedEventV1 {
        op: GroupChangedOpV1::Status,
        group_id: d.clone(),
        status: GroupStatusV1::Resolved,
        previous_status: Some(GroupStatusV1::New),
        occurrence_count: 5,
        reason: Some(GroupChangeReasonV1::Resolved),
    };
    emitter.group_changed(occurrence(&d, 5)).await;
    emitter.group_changed(occurrence(&c, 8)).await;
    emitter.group_changed(resolved.clone()).await;
    let got: Vec<Value> = listen(&mut groups)
        .await
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        got,
        vec![value(&resolved), value(&occurrence(&c, 8))],
        "the state change first, its group's occurrence dropped"
    );

    // Investigation events are delivered as they are called, through the
    // observer's filter.
    let other = InvestigationChangedEventV1 {
        op: InvestigationChangedOpV1::Created,
        investigation_id: format!("inv_events_{tag}_other"),
        group_id: b.clone(),
        session_id: format!("ses_events_{tag}_other"),
        status: InvestigationStatusV1::Running,
        error: None,
    };
    let finished = InvestigationChangedEventV1 {
        op: InvestigationChangedOpV1::Finished,
        investigation_id: format!("inv_events_{tag}"),
        group_id: a.clone(),
        session_id: format!("ses_events_{tag}"),
        status: InvestigationStatusV1::Completed,
        error: None,
    };
    emitter.investigation_changed(other).await;
    emitter.investigation_changed(finished.clone()).await;
    let got: Vec<Value> = listen(&mut investigations)
        .await
        .into_iter()
        .map(|(_, v)| v)
        .collect();
    assert_eq!(
        got,
        vec![serde_json::to_value(&finished).expect("serialize the investigation event")],
        "only the observed group's investigation event arrives"
    );

    group_observer
        .dispose()
        .await
        .expect("dispose the group observer");
    investigation_observer
        .dispose()
        .await
        .expect("dispose the investigation observer");
    await_observed(&provider, ON_GROUP_CHANGED_FN, false).await;
    await_observed(&provider, ON_INVESTIGATION_CHANGED_FN, false).await;

    consumer.shutdown_async().await;
    provider.shutdown_async().await;
}
