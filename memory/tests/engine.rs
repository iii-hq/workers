//! The memory worker against a real engine: how `memory::on-item-changed`
//! and `memory::on-bank-changed` register, that a write reaches observers
//! filtered on its bank, and that `memory::rest` answers the http worker's
//! request shape. Connects to `III_ENGINE_WS_URL` (default
//! `ws://127.0.0.1:49134`); skips when no engine answers, and fails instead
//! with `III_E2E_REQUIRE=1`.
//!
//! One test: the worker's functions have one host per engine, so the
//! checks share one registration.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::runtime::WorkerMetadata;
use iii_sdk::{
    register_worker, CallRequest, Deliver, Error, IIIClient, InitOptions, RegisterFunction,
    Source,
};
use memory::config::WorkerConfig;
use memory::deps::Deps;
use memory::events::{
    self, BankChangedEvent, Emitter, ItemChangedEvent, ON_BANK_CHANGED_DESC, ON_BANK_CHANGED_FN,
    ON_ITEM_CHANGED_DESC, ON_ITEM_CHANGED_FN,
};
use memory::functions::{self, rest};
use memory::store::Store;
use serde_json::{json, Value};
use tokio::sync::{mpsc, RwLock};
use tokio::time::{sleep, timeout};

const ON_ITEM_TARGET: &str = "memory-engine-test::on-item";
const ON_BANK_TARGET: &str = "memory-engine-test::on-bank";

fn ws_url() -> String {
    std::env::var("III_ENGINE_WS_URL").unwrap_or_else(|_| "ws://127.0.0.1:49134".to_string())
}

/// A client named `name` once the engine answers `engine::workers::list`, or
/// `None` (skip) when it never does. Each client of the process needs its own
/// name: the engine holds a worker name for one live connection.
async fn connect(url: &str, name: &str) -> Option<Arc<IIIClient>> {
    let metadata = WorkerMetadata {
        name: format!("{name}-{}", std::process::id()),
        ..WorkerMetadata::default()
    };
    let options = InitOptions {
        metadata: Some(metadata),
        ..InitOptions::default()
    };
    let iii = Arc::new(register_worker(url, options));
    for _ in 0..20 {
        let probe = iii
            .call(CallRequest {
                function_id: "engine::workers::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(800),
            })
            .await;
        if probe.is_ok() {
            return Some(iii);
        }
        sleep(Duration::from_millis(250)).await;
    }
    iii.shutdown_async().await;
    assert!(
        std::env::var("III_E2E_REQUIRE").as_deref() != Ok("1"),
        "III_E2E_REQUIRE=1 but no engine answers at {url}"
    );
    eprintln!("skipping: no iii engine at {url}; set III_ENGINE_WS_URL");
    None
}

/// The schema the SDK registers for a handler's request or response type.
fn schema<T: schemars::JsonSchema>() -> Value {
    let root = schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(root).unwrap()
}

/// `engine::functions::info` for `function_id`, once the engine has it.
async fn info(iii: &IIIClient, function_id: &str) -> Value {
    timeout(Duration::from_secs(15), async {
        loop {
            let request = CallRequest::new(
                "engine::functions::info",
                json!({ "function_id": function_id }),
            );
            if let Ok(info) = iii.call(request).await {
                return info;
            }
            sleep(Duration::from_millis(100)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("{function_id} never registered"))
}

async fn wait_until(what: &str, check: impl Fn() -> bool) {
    timeout(Duration::from_secs(15), async {
        while !check() {
            sleep(Duration::from_millis(25)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting until {what}"));
}

async fn next(rx: &mut mpsc::UnboundedReceiver<Value>, what: &str) -> Value {
    timeout(Duration::from_secs(15), rx.recv())
        .await
        .unwrap_or_else(|_| panic!("no {what} delivered"))
        .expect("channel open")
}

/// A target on `probe` that forwards what it receives.
fn forward_to_channel(probe: &IIIClient, id: &str) -> mpsc::UnboundedReceiver<Value> {
    let (tx, rx) = mpsc::unbounded_channel();
    probe.register_function(
        id,
        RegisterFunction::new(move |event: Value| {
            let _ = tx.send(event);
            Ok::<_, Error>(Value::Null)
        }),
    );
    rx
}

#[tokio::test]
async fn change_functions_register_private_and_deliver_writes_to_bank_filtered_observers() {
    let url = ws_url();
    let Some(worker) = connect(&url, "memory-engine-test").await else {
        return;
    };
    let probe = connect(&url, "memory-engine-test-probe")
        .await
        .expect("a second client connects");

    // The worker's side, as main.rs builds it, over a store in a temp dir.
    let dir = tempfile::tempdir().unwrap();
    let cfg = WorkerConfig {
        embeddings_enabled: false,
        ..WorkerConfig::default()
    };
    let deps = Arc::new(Deps {
        iii: worker.clone(),
        store: Arc::new(RwLock::new(Arc::new(
            Store::open(dir.path().to_path_buf()).unwrap(),
        ))),
        config: Arc::new(RwLock::new(Arc::new(cfg))),
        emitter: Arc::new(Emitter::new(worker.clone())),
    });
    events::register_all(&worker);
    functions::register_all(&worker, &deps);
    rest::register(&worker, &deps);

    // Registration: private, no args, the event as request and response.
    for (function_id, description, event) in [
        (
            ON_ITEM_CHANGED_FN,
            ON_ITEM_CHANGED_DESC,
            schema::<ItemChangedEvent>(),
        ),
        (
            ON_BANK_CHANGED_FN,
            ON_BANK_CHANGED_DESC,
            schema::<BankChangedEvent>(),
        ),
    ] {
        let info = info(&probe, function_id).await;
        assert_eq!(info["private"], true, "{function_id}: {info}");
        assert!(info.get("args_format").is_none(), "{function_id}: {info}");
        assert!(info.get("observable").is_none(), "{function_id}: {info}");
        assert_eq!(info["description"], description, "{function_id}");
        assert_eq!(info["request_schema"], event, "{function_id}");
        assert_eq!(info["response_schema"], event, "{function_id}");
    }

    // Observers filtered on the bank, receiving the event.
    let mut items = forward_to_channel(&probe, ON_ITEM_TARGET);
    let mut banks = forward_to_channel(&probe, ON_BANK_TARGET);
    assert!(!worker.is_observed(ON_ITEM_CHANGED_FN));
    let blog = json!({ "input.bank": { "$eq": "blog" } });
    let item_observer = probe.observe(
        Source::function(ON_ITEM_CHANGED_FN)
            .filters(blog.clone())
            .deliver(Deliver::Output),
        ON_ITEM_TARGET,
    );
    let bank_observer = probe.observe(
        Source::function(ON_BANK_CHANGED_FN)
            .filters(blog)
            .deliver(Deliver::Output),
        ON_BANK_TARGET,
    );
    for observer in [&item_observer, &bank_observer] {
        timeout(Duration::from_secs(15), observer.ready())
            .await
            .expect("the observer becomes active")
            .expect("the observer is accepted");
    }
    wait_until("the worker sees the item observer", || {
        worker.is_observed(ON_ITEM_CHANGED_FN)
    })
    .await;
    wait_until("the worker sees the bank observer", || {
        worker.is_observed(ON_BANK_CHANGED_FN)
    })
    .await;

    // A save into another bank: filtered out.
    probe
        .call(CallRequest::new(
            "memory::save",
            json!({ "bank": "other", "text": "The other bank keeps this memory" }),
        ))
        .await
        .expect("memory::save");

    // A save into `blog` through the REST adapter, with the request the http
    // worker sends a route's target.
    let response = probe
        .call(CallRequest::new(
            rest::REST_ID,
            json!({
                "query_params": {},
                "path_params": {},
                "headers": { "content-type": "application/json" },
                "path": "/memory/save",
                "method": "POST",
                "body": { "bank": "blog", "text": "The blog deploys on Fridays" },
            }),
        ))
        .await
        .expect("memory::rest");
    assert_eq!(response["status_code"], 200, "{response}");
    assert_eq!(response["body"]["created"], true, "{response}");
    let saved = &response["body"]["memory"];

    let event = next(&mut items, "memory::on-item-changed event").await;
    assert_eq!(event["event_type"], "created", "{event}");
    assert_eq!(event["bank"], "blog", "{event}");
    assert_eq!(event["memory"]["id"], saved["id"], "{event}");
    assert_eq!(event["memory"]["text"], "The blog deploys on Fridays");
    assert!(event.get("reason").is_none(), "{event}");

    let event = next(&mut banks, "memory::on-bank-changed event").await;
    assert_eq!(event, json!({ "event_type": "created", "bank": "blog" }));

    // Nothing from the `other` bank reached either observer.
    sleep(Duration::from_millis(500)).await;
    assert!(items.try_recv().is_err(), "an unfiltered item event arrived");
    assert!(banks.try_recv().is_err(), "an unfiltered bank event arrived");

    // Without observers, the gate the emitter checks is closed again.
    item_observer.dispose().await.expect("dispose");
    bank_observer.dispose().await.expect("dispose");
    wait_until("the worker sees no item observer", || {
        !worker.is_observed(ON_ITEM_CHANGED_FN)
    })
    .await;

    probe.shutdown_async().await;
    worker.shutdown_async().await;
}
