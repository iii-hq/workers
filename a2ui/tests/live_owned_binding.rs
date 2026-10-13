//! Live check of A2UI bindings to a worker-owned trigger type against a real
//! engine (run WITHOUT iii-stream) and the `owned_trigger_provider` example.
//!
//! ```bash
//! cargo run --example owned_trigger_provider -- ws://127.0.0.1:49671 &
//! A2UI_LIVE_URL=ws://127.0.0.1:49671 cargo test --test live_owned_binding -- --ignored --nocapture
//! ```
//!
//! The test plays the Console's role in-process: it validates bindings with
//! the real engine registry, registers the binding's trigger the way the page
//! does, and runs `a2ui::binding::refresh` logic on every notification.

use std::sync::Arc;
use std::time::Duration;

use a2ui::composer::Composer;
use a2ui::config::WorkerConfig;
use a2ui::functions::{
    delete_binding, refresh_binding, set_binding, BindingDeleteRequest, BindingRefreshRequest,
    BindingSetRequest, Deps,
};
use a2ui::protocol::{
    apply_messages, ServerMessage, SessionState, CATALOG_ID, PROTOCOL_VERSION,
    STREAM_BINDING_DEPRECATION,
};
use a2ui::store::Store;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{register_worker, Error, IIIClient, InitOptions, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::mpsc;

const SESSION: &str = "live-session";
const SURFACE: &str = "clicks";

fn owned_binding() -> Value {
    json!({
        "id": "clicks-count",
        "trigger_type": "demo::counter-changed",
        "config": {"counter": "clicks"},
        "target_path": "/count",
        "query": {
            "function_id": "demo::counter::get",
            "payload": {"counter": "clicks"},
            "result_path": "/value"
        }
    })
}

fn set_request(binding: Value) -> BindingSetRequest {
    serde_json::from_value(
        json!({"session_id": SESSION, "surface_id": SURFACE, "binding": binding}),
    )
    .unwrap()
}

fn refresh_request() -> BindingRefreshRequest {
    serde_json::from_value(
        json!({"session_id": SESSION, "surface_id": SURFACE, "binding_id": "clicks-count"}),
    )
    .unwrap()
}

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, Error> {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(5_000),
    })
    .await
}

async fn binding_count(iii: &IIIClient) -> u64 {
    call(iii, "demo::bindings::count", json!({})).await.unwrap()["count"]
        .as_u64()
        .unwrap()
}

async fn wait_for_count(iii: &IIIClient, expected: u64) {
    for _ in 0..50 {
        if binding_count(iii).await == expected {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("provider binding count never reached {expected}");
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine and the owned_trigger_provider example; set A2UI_LIVE_URL"]
async fn owned_trigger_type_binding_end_to_end() {
    let url = std::env::var("A2UI_LIVE_URL").expect("A2UI_LIVE_URL");
    let iii = Arc::new(register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                name: "a2ui-live-console".into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    ));
    let deps = Deps {
        iii: iii.clone(),
        config: Arc::new(tokio::sync::RwLock::new(Arc::new(WorkerConfig::default()))),
        store: Arc::new(Store::in_memory()),
        composer: Arc::new(Composer::new(iii.clone())),
    };

    // Provider must be up (its trigger type registered) before binding.
    let mut registered = false;
    for _ in 0..50 {
        if call(
            &iii,
            "engine::triggers::info",
            json!({"id": "demo::counter-changed"}),
        )
        .await
        .is_ok()
        {
            registered = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(registered, "start the owned_trigger_provider example first");
    let info = call(
        &iii,
        "engine::triggers::info",
        json!({"id": "demo::counter-changed"}),
    )
    .await
    .unwrap();
    println!("engine::triggers::info demo::counter-changed -> {info}");
    let stream_info = call(&iii, "engine::triggers::info", json!({"id": "stream"})).await;
    println!("engine::triggers::info stream -> {stream_info:?}");
    assert!(
        stream_info.is_err(),
        "this live run must use an engine without iii-stream"
    );

    // A function owned by this (non-provider) worker, marked read-only.
    iii.register_function(
        "a2ui-live::foreign::get",
        RegisterFunction::new_async(
            |_: Value| async move { Ok::<Value, Error>(json!({"value": 99})) },
        )
        .description("Foreign read-only function used to prove the owner check.")
        .metadata(json!({"read_only": true})),
    );
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut state = SessionState::empty(SESSION);
    let messages: Vec<ServerMessage> = serde_json::from_value(json!([
        {"version": PROTOCOL_VERSION, "createSurface": {"surfaceId": SURFACE, "catalogId": CATALOG_ID}},
        {"version": PROTOCOL_VERSION, "updateComponents": {"surfaceId": SURFACE, "components": [
            {"id": "root", "component": "Text", "text": {"path": "/count"}}
        ]}}
    ]))
    .unwrap();
    apply_messages(&mut state, &messages, None, &WorkerConfig::default()).unwrap();
    deps.store.save(&state).await.unwrap();

    // 1. Config and query validation errors from the real registry.
    let mut unregistered = owned_binding();
    unregistered["trigger_type"] = json!("demo::nope-changed");
    let error = set_binding(&deps, set_request(unregistered))
        .await
        .unwrap_err();
    println!("unregistered type -> {error}");
    assert!(error.contains("is not registered with the engine"));

    let mut bad_config = owned_binding();
    bad_config["config"] = json!({"counter": "clicks", "group": "x"});
    let error = set_binding(&deps, set_request(bad_config))
        .await
        .unwrap_err();
    println!("bad config -> {error}");
    assert!(error.contains("unsupported field `group`"));

    let mut missing = owned_binding();
    missing["config"] = json!({});
    let error = set_binding(&deps, set_request(missing)).await.unwrap_err();
    println!("missing config field -> {error}");
    assert!(error.contains("missing required field `counter`"));

    let mut mutating = owned_binding();
    mutating["query"]["function_id"] = json!("demo::counter::bump");
    let error = set_binding(&deps, set_request(mutating)).await.unwrap_err();
    println!("query without read_only -> {error}");
    assert!(error.contains("read_only"));

    let mut foreign = owned_binding();
    foreign["query"]["function_id"] = json!("a2ui-live::foreign::get");
    let error = set_binding(&deps, set_request(foreign)).await.unwrap_err();
    println!("query from another worker -> {error}");
    assert!(error.contains("must be registered by `a2ui-demo-provider`"));

    let mut reserved = owned_binding();
    reserved["trigger_type"] = json!("harness::hook::pre-trigger");
    let error = set_binding(&deps, set_request(reserved)).await.unwrap_err();
    println!("reserved type -> {error}");
    assert!(error.contains("reserved"));

    // 2. Legacy stream binding is still accepted, flagged deprecated, on an
    //    engine without iii-stream.
    let legacy = set_binding(
        &deps,
        set_request(json!({
            "id": "legacy-events",
            "trigger_type": "stream",
            "config": {"stream_name": "agent::events", "group_id": SESSION},
            "target_path": "/events"
        })),
    )
    .await
    .unwrap();
    println!(
        "legacy stream receipt -> {}",
        serde_json::to_string(&legacy).unwrap()
    );
    assert_eq!(
        legacy.deprecation.as_deref(),
        Some(STREAM_BINDING_DEPRECATION)
    );

    // 3. Valid owned binding.
    let receipt = set_binding(&deps, set_request(owned_binding()))
        .await
        .unwrap();
    println!(
        "owned binding receipt -> {}",
        serde_json::to_string(&receipt).unwrap()
    );
    assert!(receipt.deprecation.is_none());

    // 4. Console role: register the binding's trigger, then initial read.
    let (tx, mut rx) = mpsc::unbounded_channel::<Value>();
    let console_deps = deps.clone();
    iii.register_function(
        "a2ui-live::on-counter",
        RegisterFunction::new_async(move |event: Value| {
            let deps = console_deps.clone();
            let tx = tx.clone();
            async move {
                let refreshed = refresh_binding(&deps, refresh_request())
                    .await
                    .map_err(Error::Handler)?;
                let _ = tx.send(
                    json!({"event": event, "value": refreshed.value, "changed": refreshed.changed}),
                );
                Ok::<Value, Error>(json!({}))
            }
        })
        .description("Live check: emulates the A2UI page's binding handler."),
    );
    let trigger = iii
        .register_trigger(RegisterTriggerInput::new(
            "demo::counter-changed",
            "a2ui-live::on-counter",
            json!({"counter": "clicks"}),
        ))
        .unwrap();
    wait_for_count(&iii, 1).await;
    let initial = refresh_binding(&deps, refresh_request()).await.unwrap();
    println!(
        "initial read -> value={} changed={} revision={}",
        initial.value, initial.changed, initial.receipt.revision
    );
    let current = call(&iii, "demo::counter::get", json!({"counter": "clicks"}))
        .await
        .unwrap();
    assert_eq!(initial.value, current["value"]);
    let base = current["value"].as_u64().unwrap();

    // 5. Live updates: committed bumps arrive as notifications, then re-read.
    for step in 1..=2u64 {
        let expected = base + step;
        let bump = call(&iii, "demo::counter::bump", json!({"counter": "clicks"}))
            .await
            .unwrap();
        assert_eq!(bump["delivered"], 1, "{bump}");
        let update = tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("notification delivered")
            .unwrap();
        println!("update -> {update}");
        assert_eq!(update["value"], json!(expected));
        assert_eq!(update["event"]["revision"], json!(expected));
    }
    let stored = deps.store.load(SESSION).await.unwrap();
    assert_eq!(
        stored.get(SURFACE).unwrap().data_model["count"],
        json!(base + 2)
    );

    // Filter: another counter does not reach this binding.
    let other = call(&iii, "demo::counter::bump", json!({"counter": "other"}))
        .await
        .unwrap();
    assert_eq!(other["delivered"], 0);

    // 6. Unbind cleanup: provider drops the binding, no further delivery.
    trigger.unregister();
    wait_for_count(&iii, 0).await;
    let after = call(&iii, "demo::counter::bump", json!({"counter": "clicks"}))
        .await
        .unwrap();
    assert_eq!(after["delivered"], 0);
    assert!(tokio::time::timeout(Duration::from_millis(500), rx.recv())
        .await
        .is_err());
    let delete: BindingDeleteRequest = serde_json::from_value(
        json!({"session_id": SESSION, "surface_id": SURFACE, "binding_id": "clicks-count"}),
    )
    .unwrap();
    delete_binding(&deps, delete).await.unwrap();
    assert_eq!(
        refresh_binding(&deps, refresh_request()).await.unwrap_err(),
        "binding was not found"
    );
    println!("unbind cleanup -> provider bindings 0, no delivery, binding removed");
    iii.shutdown_async().await;
}

/// Same contract through the engine against the real a2ui binary (needs the
/// `configuration` engine worker, the `state` worker, a2ui and the provider).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "needs a live engine with a2ui, state and the provider; set A2UI_LIVE_URL"]
async fn binding_functions_through_the_engine() {
    let url = std::env::var("A2UI_LIVE_URL").expect("A2UI_LIVE_URL");
    let iii = register_worker(
        &url,
        InitOptions {
            metadata: Some(WorkerMetadata {
                runtime: "rust".into(),
                name: "a2ui-live-engine-client".into(),
                ..WorkerMetadata::default()
            }),
            ..InitOptions::default()
        },
    );
    // Fresh session per run: the state worker persists between runs.
    let session = format!("live-engine-{}", uuid::Uuid::new_v4());
    let session = session.as_str();
    let mut ready = false;
    for _ in 0..50 {
        if call(
            &iii,
            "engine::functions::info",
            json!({"function_id": "a2ui::binding::refresh"}),
        )
        .await
        .is_ok()
        {
            ready = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    assert!(
        ready,
        "a2ui worker with a2ui::binding::refresh must be running"
    );

    let applied = call(
        &iii,
        "a2ui::surface::apply",
        json!({
            "session_id": session,
            "messages": [
                {"version": PROTOCOL_VERSION, "createSurface": {"surfaceId": SURFACE, "catalogId": CATALOG_ID}},
                {"version": PROTOCOL_VERSION, "updateComponents": {"surfaceId": SURFACE, "components": [
                    {"id": "root", "component": "Text", "text": {"path": "/count"}}
                ]}}
            ]
        }),
    )
    .await
    .unwrap();
    println!("surface::apply -> {applied}");

    let set =
        |binding: Value| json!({"session_id": session, "surface_id": SURFACE, "binding": binding});
    let legacy = call(
        &iii,
        "a2ui::binding::set",
        set(json!({
            "id": "legacy-events",
            "trigger_type": "stream",
            "config": {"stream_name": "agent::events", "group_id": session},
            "target_path": "/events"
        })),
    )
    .await
    .unwrap();
    println!("binding::set stream -> {legacy}");
    assert_eq!(legacy["deprecation"], json!(STREAM_BINDING_DEPRECATION));

    let mut bad = owned_binding();
    bad["config"] = json!({"counter": "clicks", "group": "x"});
    let error = call(&iii, "a2ui::binding::set", set(bad))
        .await
        .unwrap_err()
        .to_string();
    println!("binding::set bad config -> {error}");
    assert!(error.contains("unsupported field `group`"));

    let owned = call(&iii, "a2ui::binding::set", set(owned_binding()))
        .await
        .unwrap();
    println!("binding::set owned -> {owned}");
    assert!(owned.get("deprecation").is_none());

    let ids = json!({"session_id": session, "surface_id": SURFACE, "binding_id": "clicks-count"});
    let current = call(&iii, "demo::counter::get", json!({"counter": "clicks"}))
        .await
        .unwrap();
    let first = call(&iii, "a2ui::binding::refresh", ids.clone())
        .await
        .unwrap();
    println!("binding::refresh initial -> {first}");
    assert_eq!(first["value"], current["value"]);
    let again = call(&iii, "a2ui::binding::refresh", ids.clone())
        .await
        .unwrap();
    assert_eq!(again["changed"], json!(false));
    assert_eq!(again["revision"], first["revision"]);

    call(&iii, "demo::counter::bump", json!({"counter": "clicks"}))
        .await
        .unwrap();
    let next = call(&iii, "a2ui::binding::refresh", ids.clone())
        .await
        .unwrap();
    println!("binding::refresh after bump -> {next}");
    assert_eq!(
        next["value"].as_u64(),
        first["value"].as_u64().map(|v| v + 1)
    );
    assert_eq!(next["changed"], json!(true));

    let mut apply = ids.clone();
    apply["value"] = json!(42);
    let error = call(&iii, "a2ui::binding::apply", apply)
        .await
        .unwrap_err()
        .to_string();
    println!("binding::apply on query binding -> {error}");
    assert!(error.contains("a2ui::binding::refresh"));

    call(&iii, "a2ui::binding::delete", ids.clone())
        .await
        .unwrap();
    let error = call(&iii, "a2ui::binding::refresh", ids)
        .await
        .unwrap_err()
        .to_string();
    println!("binding::refresh after delete -> {error}");
    assert!(error.contains("binding was not found"));
    iii.shutdown_async().await;
}
