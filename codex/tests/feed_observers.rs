//! `codex::on-agent-event` and `codex::on-raw-event` as registered on a live
//! engine: `Feeds::register` on one connection, observers from a second one.
//!
//! Runs against the engine named by `III_ENGINE_WS_URL`; skipped without it
//! unless `III_E2E_REQUIRE` is set.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use codex::agent_feed::{
    session_call, Feeds, MAX_OBSERVERS, MAX_SESSION_ID_CHARS, OBSERVER_LIMIT, ON_AGENT_EVENT_FN,
    ON_RAW_EVENT_FN, SOURCE,
};
use iii_helpers::observability::OtelConfig;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, Error, IIIClient, InitOptions, ObserverHandle,
    ObserverState, RegisterFunction, Source, WorkerIdentityMode,
};
use serde_json::{json, Value};

const WAIT: Duration = Duration::from_secs(15);
const EPOCH: &str = "epoch-feed-observers";

fn engine_url() -> Option<String> {
    match std::env::var("III_ENGINE_WS_URL") {
        Ok(url) => Some(url),
        Err(_) if std::env::var("III_E2E_REQUIRE").is_ok() => {
            panic!("III_E2E_REQUIRE is set: point III_ENGINE_WS_URL at a running iii engine")
        }
        Err(_) => {
            eprintln!(
                "[skip] III_ENGINE_WS_URL is not set; the feed observer test needs an engine"
            );
            None
        }
    }
}

async fn connect(url: &str, name: &str, namespace: &str) -> IIIClient {
    let iii = register_worker(
        url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            otel: Some(OtelConfig {
                enabled: Some(false),
                ..OtelConfig::default()
            }),
            namespace: Some(namespace.to_string()),
            identity: WorkerIdentityMode::Explicit,
            ..InitOptions::default()
        },
    );
    iii.wait_until_registered(WAIT)
        .await
        .expect("engine reachable");
    iii
}

async fn until(what: &str, cond: impl Fn() -> bool) {
    let deadline = Instant::now() + WAIT;
    while !cond() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

fn code(error: &Error) -> &str {
    match error {
        Error::Remote { code, .. } => code,
        other => panic!("expected a coded error, got {other}"),
    }
}

/// A target function that records every payload it receives.
#[derive(Clone, Default)]
struct Inbox(Arc<Mutex<Vec<Value>>>);

impl Inbox {
    fn register(iii: &IIIClient, id: &str) -> Self {
        let inbox = Self::default();
        let sink = inbox.clone();
        iii.register_function(
            id,
            RegisterFunction::new_async(move |payload: Value| {
                let sink = sink.clone();
                async move {
                    sink.0.lock().unwrap().push(payload);
                    Ok::<_, Error>(Value::Null)
                }
            }),
        );
        inbox
    }

    fn received(&self) -> Vec<Value> {
        self.0.lock().unwrap().clone()
    }
}

fn observe(iii: &IIIClient, function_id: &str, args: Value, target: &str) -> ObserverHandle {
    iii.observe(
        Source::function(function_id)
            .args(args)
            .deliver(Deliver::Output),
        target,
    )
}

async fn function_row(iii: &IIIClient, function_id: &str, namespace: &str) -> Value {
    let deadline = Instant::now() + WAIT;
    loop {
        let row = iii
            .call(CallRequest::new(
                "engine::functions::info",
                json!({ "function_id": function_id, "namespace": namespace }),
            ))
            .await;
        match row {
            Ok(row) => return row,
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "{function_id} never registered: {error}"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn feeds_are_private_session_keyed_and_capped_on_an_engine() {
    let Some(url) = engine_url() else {
        return;
    };
    let namespace = format!("feeds-{}", &uuid::Uuid::new_v4().simple().to_string()[..8]);
    let provider = connect(&url, "codex-feeds-provider", &namespace).await;
    let feeds = Feeds::new(EPOCH);
    feeds.register(&provider);
    let consumer = connect(&url, "codex-feeds-consumer", &namespace).await;
    let s1 = Inbox::register(&consumer, "feedtest::s1");
    let s2 = Inbox::register(&consumer, "feedtest::s2");

    // Both functions: private, `{ session_id }` args.
    for function_id in [ON_AGENT_EVENT_FN, ON_RAW_EVENT_FN] {
        let row = function_row(&provider, function_id, &namespace).await;
        assert_eq!(row["private"], true, "{function_id}: {row}");
        let args = &row["args_format"];
        assert_eq!(
            args["required"],
            json!(["session_id"]),
            "{function_id}: {row}"
        );
        assert_eq!(args["additionalProperties"], false, "{function_id}: {row}");
        assert_eq!(args["properties"]["session_id"]["minLength"], 1);
        assert_eq!(
            args["properties"]["session_id"]["maxLength"],
            MAX_SESSION_ID_CHARS
        );
    }

    // Args outside FeedArgs never reach the worker's hooks.
    for args in [
        json!({}),
        json!({ "session_id": "" }),
        json!({ "session_id": "s1", "stream": "x" }),
        json!({ "session_id": "x".repeat(MAX_SESSION_ID_CHARS + 1) }),
    ] {
        let handle = observe(&consumer, ON_AGENT_EVENT_FN, args.clone(), "feedtest::s1");
        let error = handle.ready().await.expect_err("bad args are refused");
        assert_eq!(code(&error), "INVALID_ARGS", "args {args}: {error}");
    }
    assert_eq!(feeds.agent.observer_count(), 0);

    // A frame of s1 reaches the s1 observer only.
    let on_s1 = observe(
        &consumer,
        ON_AGENT_EVENT_FN,
        json!({ "session_id": "s1" }),
        "feedtest::s1",
    );
    let on_s2 = observe(
        &consumer,
        ON_AGENT_EVENT_FN,
        json!({ "session_id": "s2" }),
        "feedtest::s2",
    );
    on_s1.ready().await.expect("s1 observer active");
    on_s2.ready().await.expect("s2 observer active");
    assert_eq!(feeds.agent.observer_count(), 2);
    until("the s1 observer to reach the host", || {
        provider.is_observed_for(ON_AGENT_EVENT_FN, &session_call("s1"))
    })
    .await;
    assert!(!provider.is_observed_for(ON_AGENT_EVENT_FN, &session_call("s3")));

    let frame = json!({ "type": "agent_end", "messages": [] });
    assert!(
        !feeds
            .agent
            .emit(&provider, "s3", || panic!(
                "a frame nobody observes is not built"
            ))
            .await
    );
    assert!(
        !feeds
            .raw
            .emit(&provider, "s1", || panic!(
                "the raw feed of s1 is not observed"
            ))
            .await
    );
    assert!(feeds.agent.emit(&provider, "s1", || frame.clone()).await);
    until("the s1 frame", || s1.received().len() == 1).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(s2.received().is_empty(), "s2: {:?}", s2.received());
    // Each feed and session counts on its own: the unobserved s3 and raw
    // frames took their own seq 0.
    assert_eq!(
        s1.received()[0],
        json!({
            "session_id": "s1",
            "event_id": format!("s1-{EPOCH}-00000000"),
            "seq": 0,
            "epoch": EPOCH,
            "source": SOURCE,
            "event": frame,
        })
    );

    // The observer past MAX_OBSERVERS is rejected with OBSERVER_LIMIT.
    let fill: Vec<ObserverHandle> = (2..MAX_OBSERVERS)
        .map(|i| {
            observe(
                &consumer,
                ON_AGENT_EVENT_FN,
                json!({ "session_id": format!("fill-{i}") }),
                "feedtest::s2",
            )
        })
        .collect();
    for handle in &fill {
        handle
            .ready()
            .await
            .expect("an observer under the cap is accepted");
    }
    assert_eq!(feeds.agent.observer_count(), MAX_OBSERVERS);
    let over = observe(
        &consumer,
        ON_AGENT_EVENT_FN,
        json!({ "session_id": "over" }),
        "feedtest::s2",
    );
    let error = over.ready().await.expect_err("observer 257 is refused");
    assert_eq!(code(&error), OBSERVER_LIMIT, "{error}");
    assert_eq!(over.state(), ObserverState::Error);
    assert_eq!(feeds.agent.observer_count(), MAX_OBSERVERS);

    // Disposing an observer releases its slot and its session key.
    on_s1.dispose().await.expect("dispose");
    until("the release hook", || {
        feeds.agent.observer_count() == MAX_OBSERVERS - 1
    })
    .await;
    until("s1 to be unobserved", || {
        !provider.is_observed_for(ON_AGENT_EVENT_FN, &session_call("s1"))
    })
    .await;
    let again = observe(
        &consumer,
        ON_AGENT_EVENT_FN,
        json!({ "session_id": "again" }),
        "feedtest::s2",
    );
    again.ready().await.expect("the freed slot is taken");
    assert_eq!(feeds.agent.observer_count(), MAX_OBSERVERS);

    consumer.shutdown_async().await;
    provider.shutdown_async().await;
}
