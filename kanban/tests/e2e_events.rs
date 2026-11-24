//! `kanban::on-change` and `kanban::on-comment` against a live engine: the
//! added hook's rejection, what the deliver hook hands each observer (the
//! event itself, with the observer's metadata as the sidecar), and what a
//! caller gets back.
//!
//! Each test boots the worker's event functions and catalog in-process on one
//! connection (the owner, with its board in a temp folder) and observes them
//! from another (the consumer). Without an engine at `III_ENGINE_WS_URL`
//! (default `ws://127.0.0.1:49134`) the tests skip; `III_E2E_REQUIRE=1` makes
//! them fail instead.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use iii_kanban::config::KanbanConfig;
use iii_kanban::events::{self, ON_CHANGE_FN, ON_COMMENT_FN, TICKET_OR_PARENT_REQUIRED};
use iii_kanban::functions::{self, Ctx};
use iii_kanban::store::BoardStore;
use iii_sdk::errors::Error;
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    CallRequest, IIIClient, InitOptions, ObserverHandle, ObserverOptions, RegisterFunction, Source,
    register_worker,
};
use serde_json::{Value, json};

const DEFAULT_WS_URL: &str = "ws://127.0.0.1:49134";

/// Both tests register the same function ids; one runs at a time.
static ENGINE_SLOT: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| DEFAULT_WS_URL.to_string())
}

/// Whether anything accepts connections at the engine URL's host and port.
fn listening(url: &str) -> bool {
    use std::net::{TcpStream, ToSocketAddrs};
    let address = url
        .trim_start_matches("ws://")
        .trim_start_matches("wss://")
        .split('/')
        .next()
        .unwrap_or_default();
    address
        .to_socket_addrs()
        .into_iter()
        .flatten()
        .any(|addr| TcpStream::connect_timeout(&addr, Duration::from_secs(1)).is_ok())
}

fn skip_or_fail() -> Option<Arc<IIIClient>> {
    if std::env::var("III_E2E_REQUIRE").is_ok() {
        panic!("e2e requires a running iii engine at {}", ws_url());
    }
    eprintln!(
        "[skip] no iii engine reachable at {}; set III_E2E_REQUIRE=1 to fail instead",
        ws_url()
    );
    None
}

/// A connection under its own worker name, or `None` (skip) when no engine
/// answers and `III_E2E_REQUIRE` is unset.
async fn connect(name: &str) -> Option<Arc<IIIClient>> {
    if !listening(&ws_url()) {
        return skip_or_fail();
    }
    let iii = Arc::new(register_worker(
        &ws_url(),
        InitOptions {
            metadata: Some(WorkerMetadata {
                name: name.to_string(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    for _ in 0..20 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".to_string(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if probe.is_ok() {
            return Some(iii);
        }
    }
    iii.shutdown_async().await;
    skip_or_fail()
}

/// The kanban worker as `main` boots it, minus the configuration worker and
/// the UIs: the event functions with their hooks, then the catalog, with the
/// board in `folder`.
fn boot_owner(iii: &Arc<IIIClient>, folder: &std::path::Path) -> Arc<Ctx> {
    let config = KanbanConfig {
        data_path: folder.join("data").to_string_lossy().into_owned(),
        agents_path: folder.join("agents").to_string_lossy().into_owned(),
        ..KanbanConfig::default()
    };
    let config = Arc::new(RwLock::new(config));
    events::register(iii);
    let ctx = Arc::new(Ctx {
        iii: iii.clone(),
        store: BoardStore::new(config.clone()),
        config,
        dispatch: Default::default(),
    });
    functions::register_functions(&ctx);
    ctx
}

/// Registers `id` on `iii`; every payload it receives lands in the result.
fn recorder(iii: &IIIClient, id: &str) -> Arc<Mutex<Vec<Value>>> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();
    iii.register_function(
        id,
        RegisterFunction::new(move |payload: Value| {
            sink.lock().unwrap().push(payload);
            Ok::<_, Error>(json!({ "ok": true }))
        }),
    );
    received
}

/// One delivery: the payload and the metadata sidecar it came with.
type Delivery = (Value, Option<Value>);

/// Registers `id` on `iii`; every payload it receives lands in the result
/// with its metadata sidecar.
fn sidecar_recorder(iii: &IIIClient, id: &str) -> Arc<Mutex<Vec<Delivery>>> {
    let received = Arc::new(Mutex::new(Vec::new()));
    let sink = received.clone();
    iii.register_function(
        id,
        RegisterFunction::new(move |payload: Value, metadata: Option<Value>| {
            sink.lock().unwrap().push((payload, metadata));
            Ok::<_, Error>(json!({ "ok": true }))
        }),
    );
    received
}

fn taken<T: Clone>(received: &Mutex<Vec<T>>) -> Vec<T> {
    received.lock().unwrap().clone()
}

async fn settled(handle: &ObserverHandle) -> Result<(), Error> {
    tokio::time::timeout(Duration::from_secs(10), handle.ready())
        .await
        .expect("observer neither became active nor failed within 10 s")
}

/// Waits until every recorder in `wanted` holds one payload.
async fn arrived<T>(wanted: &[(&str, &Arc<Mutex<Vec<T>>>)]) {
    for _ in 0..100 {
        if wanted
            .iter()
            .all(|(_, got)| !got.lock().unwrap().is_empty())
        {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let missing: Vec<&str> = wanted
        .iter()
        .filter(|(_, got)| got.lock().unwrap().is_empty())
        .map(|(name, _)| *name)
        .collect();
    panic!("no delivery reached {missing:?} within 5 s");
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Value {
    iii.call(CallRequest {
        function_id: function_id.to_string(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
    .unwrap_or_else(|e| panic!("{function_id}: {e}"))
}

async fn create_ticket(iii: &IIIClient, title: &str) -> String {
    let ticket = call(iii, "kanban::ticket::create", json!({ "title": title })).await;
    ticket["key"]
        .as_str()
        .unwrap_or_else(|| panic!("no key in {ticket}"))
        .to_string()
}

/// The whole ticket carries its comments, activity and progress; the summary
/// carries only the board-card fields.
fn assert_full(ticket: &Value) {
    for field in ["comments", "activity", "progress", "children"] {
        assert!(
            ticket.get(field).is_some(),
            "full ticket lacks {field}: {ticket}"
        );
    }
}

fn assert_summary(ticket: &Value) {
    assert!(
        ticket["comment_count"].is_u64(),
        "summary lacks comment_count: {ticket}"
    );
    for field in ["comments", "activity", "progress", "children"] {
        assert!(
            ticket.get(field).is_none(),
            "summary carries {field}: {ticket}"
        );
    }
}

#[tokio::test]
async fn on_comment_rejects_an_observer_without_ticket_or_parent_and_delivers_per_observer() {
    let _slot = ENGINE_SLOT.lock().await;
    let Some(owner) = connect("kanban-e2e-comment-owner").await else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    boot_owner(&owner, folder.path());
    let Some(consumer) = connect("kanban-e2e-comment-consumer").await else {
        owner.shutdown_async().await;
        return;
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let key = create_ticket(&consumer, "commented").await;
    let other = create_ticket(&consumer, "untouched").await;

    let summary = recorder(&consumer, "e2e::comment-summary");
    let full = recorder(&consumer, "e2e::comment-full");
    let filtered = recorder(&consumer, "e2e::comment-filtered");
    let elsewhere = recorder(&consumer, "e2e::comment-elsewhere");
    let unscoped = recorder(&consumer, "e2e::comment-unscoped");

    // Neither ticket_id nor parent_id: the added hook rejects it.
    let rejected = consumer.observe(
        Source::function(ON_COMMENT_FN).args(json!({})),
        "e2e::comment-unscoped",
    );
    match settled(&rejected).await {
        Err(Error::Remote { code, .. }) => assert_eq!(code, TICKET_OR_PARENT_REQUIRED),
        other => panic!("expected {TICKET_OR_PARENT_REQUIRED}, got {other:?}"),
    }

    let handles = [
        consumer.observe(
            Source::function(ON_COMMENT_FN).args(json!({ "ticket_id": key, "ticket": "summary" })),
            "e2e::comment-summary",
        ),
        consumer.observe(
            Source::function(ON_COMMENT_FN).args(json!({ "ticket_id": key })),
            "e2e::comment-full",
        ),
        // Its args select the comment; its filters do not.
        consumer.observe(
            Source::function(ON_COMMENT_FN)
                .args(json!({ "ticket_id": key }))
                .filters(json!({ "input.author": { "$eq": "someone-else" } })),
            "e2e::comment-filtered",
        ),
        // Its args select another ticket.
        consumer.observe(
            Source::function(ON_COMMENT_FN).args(json!({ "ticket_id": other })),
            "e2e::comment-elsewhere",
        ),
    ];
    for handle in &handles {
        settled(handle).await.expect("a scoped on-comment observer");
    }

    call(
        &consumer,
        "kanban::comment::create",
        json!({ "ticket_id": key, "body": "looks good", "author": "reviewer" }),
    )
    .await;
    arrived(&[("summary", &summary), ("full", &full)]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    // The target receives the event itself, its ticket cut to the summary
    // when the observer asked for it.
    let summary = taken(&summary);
    assert_eq!(summary.len(), 1, "{summary:?}");
    let event = &summary[0];
    assert_eq!(event["event"], "comment.created");
    assert_eq!(event["ticket_key"], key.as_str());
    assert_eq!(event["author"], "reviewer");
    assert_eq!(event["comment"]["body"], "looks good");
    assert_summary(&event["ticket"]);
    assert_eq!(event["ticket"]["comment_count"], 1);

    let full = taken(&full);
    assert_eq!(full.len(), 1, "{full:?}");
    let event = &full[0];
    assert_eq!(event["event"], "comment.created");
    assert_full(&event["ticket"]);
    assert_eq!(event["ticket"]["comments"][0]["body"], "looks good");

    assert!(
        taken(&filtered).is_empty(),
        "a filter miss got {:?}",
        taken(&filtered)
    );
    assert!(
        taken(&elsewhere).is_empty(),
        "another ticket's observer got {:?}",
        taken(&elsewhere)
    );
    assert!(
        taken(&unscoped).is_empty(),
        "the rejected observer got {:?}",
        taken(&unscoped)
    );

    for handle in &handles {
        let _ = handle.dispose().await;
    }
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
async fn on_change_delivers_the_summary_or_the_whole_ticket_and_skips_filter_misses() {
    let _slot = ENGINE_SLOT.lock().await;
    let Some(owner) = connect("kanban-e2e-change-owner").await else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    boot_owner(&owner, folder.path());
    let Some(consumer) = connect("kanban-e2e-change-consumer").await else {
        owner.shutdown_async().await;
        return;
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let key = create_ticket(&consumer, "moving").await;
    let other = create_ticket(&consumer, "parked").await;

    let summary = recorder(&consumer, "e2e::change-summary");
    let full = recorder(&consumer, "e2e::change-full");
    let filtered = recorder(&consumer, "e2e::change-filtered");
    let elsewhere = recorder(&consumer, "e2e::change-elsewhere");

    let handles = [
        consumer.observe(
            Source::function(ON_CHANGE_FN).args(json!({ "ticket_id": key, "ticket": "summary" })),
            "e2e::change-summary",
        ),
        consumer.observe(
            Source::function(ON_CHANGE_FN).args(json!({ "ticket_id": key })),
            "e2e::change-full",
        ),
        consumer.observe(
            Source::function(ON_CHANGE_FN)
                .args(json!({ "ticket_id": key }))
                .filters(json!({ "input.event": { "$eq": "ticket.deleted" } })),
            "e2e::change-filtered",
        ),
        consumer.observe(
            Source::function(ON_CHANGE_FN).args(json!({ "ticket_id": other })),
            "e2e::change-elsewhere",
        ),
    ];
    for handle in &handles {
        settled(handle).await.expect("an on-change observer");
    }

    call(
        &consumer,
        "kanban::ticket::move",
        json!({ "id": key, "status": "in_progress" }),
    )
    .await;
    arrived(&[("summary", &summary), ("full", &full)]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let summary = taken(&summary);
    assert_eq!(summary.len(), 1, "{summary:?}");
    let event = &summary[0];
    assert_eq!(event["event"], "ticket.moved");
    assert_eq!(event["ticket_key"], key.as_str());
    assert_summary(&event["ticket"]);
    assert_eq!(event["ticket"]["status"], "in_progress");

    let full = taken(&full);
    assert_eq!(full.len(), 1, "{full:?}");
    let event = &full[0];
    assert_eq!(event["event"], "ticket.moved");
    assert_full(&event["ticket"]);
    assert_eq!(event["ticket"]["status"], "in_progress");

    assert!(
        taken(&filtered).is_empty(),
        "a filter miss got {:?}",
        taken(&filtered)
    );
    assert!(
        taken(&elsewhere).is_empty(),
        "another ticket's observer got {:?}",
        taken(&elsewhere)
    );

    for handle in &handles {
        let _ = handle.dispose().await;
    }
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

/// The raw event a target receives: no `Observed` envelope around it.
fn assert_raw_event(payload: &Value, event: &str) {
    assert_eq!(payload["event"], event, "{payload}");
    for envelope_field in ["observer_id", "activation_id", "function_id"] {
        assert!(
            payload.get(envelope_field).is_none(),
            "an envelope field {envelope_field} reached the target: {payload}"
        );
    }
}

/// The harness binds an agent to an event function with an observer whose
/// metadata is `{ __binding: <id> }` and reads that pointer from the
/// delivery's sidecar. The deliver hooks call targets through the observer,
/// so a target gets the event itself (summary-cut when its args ask) with the
/// observer's metadata as the sidecar; and a caller of either function gets
/// the event back.
#[tokio::test]
async fn targets_get_the_event_with_the_observer_metadata_and_callers_get_the_event() {
    let _slot = ENGINE_SLOT.lock().await;
    let Some(owner) = connect("kanban-e2e-binding-owner").await else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    boot_owner(&owner, folder.path());
    let Some(consumer) = connect("kanban-e2e-binding-consumer").await else {
        owner.shutdown_async().await;
        return;
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let key = create_ticket(&consumer, "bound").await;

    let change = sidecar_recorder(&consumer, "e2e::binding-change");
    let summary = sidecar_recorder(&consumer, "e2e::binding-summary");
    let comment = sidecar_recorder(&consumer, "e2e::binding-comment");
    let config = sidecar_recorder(&consumer, "e2e::binding-config");
    let bound =
        |binding: &str| ObserverOptions::default().metadata(json!({ "__binding": binding }));
    let handles = [
        consumer.observe_with(
            Source::function(ON_CHANGE_FN)
                .args(json!({ "ticket_id": key, "events": ["ticket.moved"] })),
            "e2e::binding-change",
            bound("binding-change"),
        ),
        consumer.observe_with(
            Source::function(ON_CHANGE_FN).args(json!({
                "ticket_id": key,
                "ticket": "summary",
                "events": ["ticket.moved"],
            })),
            "e2e::binding-summary",
            bound("binding-summary"),
        ),
        consumer.observe_with(
            Source::function(ON_COMMENT_FN).args(json!({ "ticket_id": key })),
            "e2e::binding-comment",
            bound("binding-comment"),
        ),
        consumer.observe_with(
            Source::function(ON_CHANGE_FN).args(json!({ "events": ["config.updated"] })),
            "e2e::binding-config",
            bound("binding-config"),
        ),
    ];
    for handle in &handles {
        settled(handle).await.expect("a bound observer");
    }

    call(
        &consumer,
        "kanban::ticket::move",
        json!({ "id": key, "status": "in_progress" }),
    )
    .await;
    call(
        &consumer,
        "kanban::comment::create",
        json!({ "ticket_id": key, "body": "on it", "author": "agent" }),
    )
    .await;
    arrived(&[
        ("change", &change),
        ("summary", &summary),
        ("comment", &comment),
    ])
    .await;

    // The host calls its private event function itself; the deliver hook
    // hands the caller the event it was called with.
    let event = json!({ "event": "config.updated", "at": "2026-10-30T00:00:00Z" });
    let answer = owner
        .call(CallRequest {
            function_id: ON_CHANGE_FN.to_string(),
            payload: event.clone(),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .expect("the host calls kanban::on-change");
    assert_eq!(answer, event);
    arrived(&[("config", &config)]).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let change = taken(&change);
    assert_eq!(change.len(), 1, "{change:?}");
    let (payload, metadata) = &change[0];
    assert_raw_event(payload, "ticket.moved");
    assert_full(&payload["ticket"]);
    assert_eq!(metadata, &Some(json!({ "__binding": "binding-change" })));

    let summary = taken(&summary);
    assert_eq!(summary.len(), 1, "{summary:?}");
    let (payload, metadata) = &summary[0];
    assert_raw_event(payload, "ticket.moved");
    assert_summary(&payload["ticket"]);
    assert_eq!(metadata, &Some(json!({ "__binding": "binding-summary" })));

    let comment = taken(&comment);
    assert_eq!(comment.len(), 1, "{comment:?}");
    let (payload, metadata) = &comment[0];
    assert_raw_event(payload, "comment.created");
    assert_eq!(payload["comment"]["body"], "on it");
    assert_eq!(metadata, &Some(json!({ "__binding": "binding-comment" })));

    let config = taken(&config);
    assert_eq!(config.len(), 1, "{config:?}");
    let (payload, metadata) = &config[0];
    // The event as called, with the engine's caller stamp of every call a
    // worker makes.
    let mut delivered = payload.clone();
    delivered
        .as_object_mut()
        .expect("an event object")
        .remove("_caller_worker_id");
    assert_eq!(delivered, event);
    assert_eq!(metadata, &Some(json!({ "__binding": "binding-config" })));

    for handle in &handles {
        let _ = handle.dispose().await;
    }
    consumer.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
async fn question_observers_and_boot_reconciliation_use_the_harness_contract() {
    use iii_kanban::questions;
    let _slot = ENGINE_SLOT.lock().await;
    let Some(owner) = connect("kanban-e2e-questions").await else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let ctx = boot_owner(&owner, folder.path());
    questions::register(&ctx);
    let Some(harness) = connect("kanban-e2e-harness").await else {
        owner.shutdown_async().await;
        return;
    };
    tokio::time::sleep(Duration::from_millis(200)).await;
    let key = create_ticket(&owner, "linked by session metadata").await;
    let other = create_ticket(&owner, "settled before ask").await;

    let comment_events = recorder(&owner, "e2e::question-comments");
    let comments_handle = owner.observe(
        Source::function(ON_COMMENT_FN).args(json!({"ticket_id":other})),
        "e2e::question-comments",
    );
    settled(&comments_handle).await.unwrap();
    // Before the harness comes up, the observers remain pending and replay retries.
    assert!(questions::reconcile(&ctx).await.is_err());
    questions::spawn_reconcile(ctx.clone());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let pending = json!({
        "session_id":"session-1","question_id":"call-1","turn_id":"turn-1",
        "asked_at":1795262400000_i64,"about":{},"state":"open","on_expiry":"block",
        "questions":[{"header":"Builds","question":"Isolate?","multi_select":false,
            "options":[{"label":"Yes","recommended":true},{"label":"No"}]}]
    });
    let mut settled = pending.clone();
    settled["question_id"] = json!("call-2");
    settled["about"] = json!({"ticket":other});
    settled["state"] = json!("answered");
    settled["outcome"] = json!("answered");
    settled["by"] = json!({"channel":"chat","user":"Pat"});
    settled["answer_text"] = json!("Yes");
    settled["settled_at"] = json!(1795262460000_i64);
    let calls = Arc::new(Mutex::new(Vec::new()));
    let repeated = Arc::new(Mutex::new(false));
    let requests = calls.clone();
    let repeat = repeated.clone();
    let first = pending.clone();
    let second = settled.clone();
    harness.register_function(
        "harness::ask::list",
        RegisterFunction::new(move |request: Value| {
            requests.lock().unwrap().push(request.clone());
            Ok::<_, Error>(if *repeat.lock().unwrap() {
                json!({"entries":[],"cursor":"loop"})
            } else if request["cursor"].is_null() {
                json!({"entries":[first],"cursor":"page-2"})
            } else {
                assert_eq!(request["cursor"], "page-2");
                json!({"entries":[second]})
            })
        }),
    );
    let session_calls = Arc::new(Mutex::new(Vec::new()));
    let sessions = session_calls.clone();
    let fallback_key = key.clone();
    harness.register_function(
        "session::get",
        RegisterFunction::new(move |request: Value| {
            sessions.lock().unwrap().push(request);
            Ok::<_, Error>(json!({"meta":{"metadata":{"kanban_ticket":fallback_key}}}))
        }),
    );
    for source in ["harness::on-ask", "harness::on-ask-settled"] {
        harness.register_function(
            source,
            RegisterFunction::new(|entry: Value| Ok::<_, Error>(entry)),
        );
    }
    let answers = recorder(&harness, "harness::ask::answer");
    // Retry is five seconds; wait for the complete two-page boot replay.
    let mut complete = false;
    for _ in 0..100 {
        let board = ctx.store.read().await.unwrap();
        if board
            .tickets
            .values()
            .map(|ticket| ticket.comments.len())
            .sum::<usize>()
            == 3
            && taken(&comment_events).len() == 2
        {
            complete = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(complete, "boot retry did not replay both pages");
    arrived(&[("question pair", &comment_events)]).await;
    let pair = taken(&comment_events);
    assert_eq!(pair.len(), 2);
    assert!(pair[0]["comment"]["question"].is_object());
    assert_eq!(pair[1]["comment"]["parent_id"], pair[0]["comment"]["id"]);
    assert_eq!(pair[0]["ticket"]["open_questions"], 1);
    assert_eq!(pair[1]["ticket"]["open_questions"], 0);
    assert_eq!(taken(&calls).len(), 2);
    let mut sessions = taken(&session_calls);
    sessions[0]
        .as_object_mut()
        .unwrap()
        .remove("_caller_worker_id");
    assert_eq!(sessions, vec![json!({"session_id":"session-1"})]);
    let ticket = call(&owner, "kanban::ticket::get", json!({"id":key})).await;
    assert_eq!(ticket["open_questions"], 1);
    let comment_id = ticket["comments"][0]["id"].clone();
    let other_ticket = call(&owner, "kanban::ticket::get", json!({"id":other})).await;
    assert_eq!(other_ticket["open_questions"], 0);
    assert_eq!(
        other_ticket["comments"][0]["created_at"],
        "2026-11-21T12:00:00.000Z"
    );
    assert_eq!(
        other_ticket["comments"][1]["created_at"],
        "2026-11-21T12:01:00.000Z"
    );
    // A second replay is idempotent, including the fallback-linked entry.
    questions::reconcile(&ctx).await.unwrap();
    assert_eq!(taken(&session_calls).len(), 1);
    call(&owner,"kanban::question::answer",json!({"ticket_id":key,"comment_id":comment_id,"answers":[{"header":"Builds","choice":"Yes"}],"user":"Pat"})).await;
    let mut forwarded = taken(&answers);
    forwarded[0]
        .as_object_mut()
        .unwrap()
        .remove("_caller_worker_id");
    assert_eq!(
        forwarded,
        vec![
            json!({"session_id":"session-1","question_id":"call-1","answers":[{"header":"Builds","choice":"Yes"}],"by":{"channel":"kanban","user":"Pat"}})
        ]
    );
    assert_eq!(
        call(&owner, "kanban::ticket::get", json!({"id":key})).await["open_questions"],
        1
    );
    let mut event = settled.clone();
    event["question_id"] = json!("call-1");
    event["about"] = json!({});
    call(&harness, "harness::on-ask-settled", event).await;
    let mut closed = false;
    for _ in 0..50 {
        if call(&owner, "kanban::ticket::get", json!({"id":key})).await["open_questions"] == 0 {
            closed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(closed, "settled observer did not deliver its input entry");
    call(&harness, "harness::on-ask", pending).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        call(&owner, "kanban::ticket::get", json!({"id":key})).await["comments"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    let recorded = call(&owner,"kanban::question::record",json!({"ticket_id":key,"questions":[{"header":"Storage","question":"Where?"}],"answer_text":"Existing store","recorded_by":"backend-engineer"})).await;
    assert_eq!(recorded["state"], "recorded");
    assert_eq!(recorded["comment"]["recorded_by"], "backend-engineer");
    let assumptions = call(
        &owner,
        "kanban::question::list",
        json!({"state":"recorded"}),
    )
    .await;
    assert_eq!(assumptions["count"], 1);
    assert_eq!(
        assumptions["questions"][0]["answer"]["answer"]["answer_text"],
        "Existing store"
    );
    comments_handle.dispose().await.unwrap();
    *repeated.lock().unwrap() = true;
    assert!(
        questions::reconcile(&ctx)
            .await
            .unwrap_err()
            .to_string()
            .contains("repeated its cursor")
    );
    harness.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
async fn a_missing_harness_ask_keeps_the_answer_on_the_card_as_not_delivered() {
    use iii_kanban::questions::{self, AskEntry};
    let _slot = ENGINE_SLOT.lock().await;
    let Some(owner) = connect("kanban-e2e-missing-ask").await else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let ctx = boot_owner(&owner, folder.path());
    let Some(harness) = connect("kanban-e2e-missing-ask-harness").await else {
        owner.shutdown_async().await;
        return;
    };
    harness.register_function(
        "harness::ask::answer",
        RegisterFunction::new(|_: Value| {
            Err::<Value, _>(Error::Handler(
                "harness/invalid_request: ask entry not found".into(),
            ))
        }),
    );
    let key = create_ticket(&owner, "session was deleted").await;
    let entry: AskEntry = serde_json::from_value(json!({
        "session_id":"deleted-session","question_id":"call-1",
        "asked_at":1795262400000_i64,"state":"open",
        "about":{"ticket":key},
        "questions":[{"header":"Builds","question":"Isolate?","multi_select":false,
            "options":[{"label":"Yes"},{"label":"No"}]}]
    }))
    .unwrap();
    ctx.store
        .update(|board, _| questions::mirror(board, &entry, None, "2026-11-21T12:00:00Z"))
        .await
        .unwrap();
    let ticket = call(&owner, "kanban::ticket::get", json!({"id":key})).await;
    let result = call(
        &owner,
        "kanban::question::answer",
        json!({"ticket_id":key,"comment_id":ticket["comments"][0]["id"],
            "answers":[{"header":"Builds","choice":"Yes"}],"user":"Pat"}),
    )
    .await;
    assert_eq!(result, json!({"outcome":"not_delivered"}));
    let ticket = call(&owner, "kanban::ticket::get", json!({"id":key})).await;
    assert_eq!(ticket["open_questions"], 0);
    assert_eq!(ticket["comments"][1]["answer"]["outcome"], "not_delivered");
    assert_eq!(
        ticket["comments"][1]["answer"]["answer_text"],
        "Builds: Yes"
    );
    assert_eq!(
        ticket["comments"][1]["answer"]["by"],
        json!({
            "channel":"kanban","user":"Pat"
        })
    );

    let race_key = create_ticket(&owner, "withdrawal won the race").await;
    let mut race = entry.clone();
    race.question.question_id = "call-2".into();
    race.question.about.as_mut().unwrap().ticket = Some(race_key.clone());
    ctx.store
        .update(|board, _| {
            questions::mirror(board, &race, None, "2026-11-21T12:01:00Z")?;
            race.state = questions::AskState::Withdrawn;
            race.outcome = Some(questions::AnswerOutcome::Withdrawn);
            race.settled_at = Some(1795262460000_i64);
            questions::mirror(board, &race, None, "2026-11-21T12:02:00Z")
        })
        .await
        .unwrap();
    let before = call(&owner, "kanban::ticket::get", json!({"id":race_key})).await;
    let result = call(
        &owner,
        "kanban::question::answer",
        json!({"ticket_id":race_key,"comment_id":before["comments"][0]["id"],
            "answers":[{"header":"Builds","choice":"No"}],"user":"Pat"}),
    )
    .await;
    assert_eq!(result, json!({"outcome":"withdrawn"}));
    let after = call(&owner, "kanban::ticket::get", json!({"id":race_key})).await;
    assert_eq!(after["comments"], before["comments"]);
    harness.shutdown_async().await;
    owner.shutdown_async().await;
}

#[tokio::test]
async fn dispatch_keeps_questions_in_progress_and_parks_stops_on_events_and_at_boot() {
    use iii_kanban::dispatch;
    let _slot = ENGINE_SLOT.lock().await;
    let Some(owner) = connect("kanban-e2e-dispatch").await else {
        return;
    };
    let folder = tempfile::tempdir().unwrap();
    let ctx = boot_owner(&owner, folder.path());
    let statuses = Arc::new(Mutex::new(serde_json::Map::new()));
    let replies = statuses.clone();
    owner.register_function(
        "harness::status",
        RegisterFunction::new(move |request: Value| {
            Ok::<_, Error>(replies.lock().unwrap()[request["session_id"].as_str().unwrap()].clone())
        }),
    );
    let mut claims = Vec::new();
    for title in ["event question", "boot question", "boot stop"] {
        let key = create_ticket(&owner, title).await;
        call(
            &owner,
            "kanban::ticket::update",
            json!({"id":key,"status":"in_progress","actor":dispatch::ACTOR,
                "pills":[{"icon":"loader","text":"starting"}]}),
        )
        .await;
        let board = ctx.store.read().await.unwrap();
        let seq =
            dispatch::claim_seq(iii_kanban::board::find_ticket(&board, &key).unwrap()).unwrap();
        let session = dispatch::session_id(&key, seq);
        statuses.lock().unwrap().insert(
            session.clone(),
            json!({"status":"completed","expects_wake":false,
                "open_asks":if title == "boot stop" { 0 } else { 1 }}),
        );
        claims.push((key, session));
    }
    dispatch::register(&ctx);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (event_key, event_session) = &claims[0];
    call(
        &owner,
        dispatch::ON_TURN_COMPLETED_FN,
        json!({"input":{"session_id":event_session,"terminal":true,
            "status":"completed","open_asks":1}}),
    )
    .await;
    assert_eq!(
        call(&owner, "kanban::ticket::get", json!({"id":event_key})).await["status"],
        "in_progress"
    );

    ctx.config.write().unwrap().dispatch = dispatch::normalize(Some(&json!([
        {"assignee":"deepwork","model":"test/model",
            "projects":[{"project":"Tests","root":"/repo"}]}
    ])));
    dispatch::spawn_sweep(ctx.clone());
    let (boot_key, _) = &claims[1];
    let (stopped_key, _) = &claims[2];
    let mut swept = false;
    for _ in 0..100 {
        let board = ctx.store.read().await.unwrap();
        let waiting = iii_kanban::board::find_ticket(&board, boot_key).unwrap();
        let stopped = iii_kanban::board::find_ticket(&board, stopped_key).unwrap();
        if waiting.pills.is_empty() && stopped.status == "backlog" {
            assert_eq!(waiting.status, "in_progress");
            assert_eq!(stopped.pills, vec![dispatch::unfinished_pill()]);
            assert!(stopped.labels.iter().any(|label| label == "unfinished"));
            swept = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        swept,
        "boot sweep did not preserve the question and park the stop"
    );
    call(
        &owner,
        dispatch::ON_TURN_COMPLETED_FN,
        json!({"input":{"session_id":event_session,"terminal":true,
            "status":"failed","open_asks":0}}),
    )
    .await;
    let stopped = call(&owner, "kanban::ticket::get", json!({"id":event_key})).await;
    assert_eq!(stopped["status"], "backlog");
    assert_eq!(stopped["pills"], json!([dispatch::unfinished_pill()]));
    assert!(
        stopped["comments"][0]["body"]
            .as_str()
            .unwrap()
            .contains("Unfinished")
    );
    owner.shutdown_async().await;
}
