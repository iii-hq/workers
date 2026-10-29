//! Caller-specific retry ceilings on the real engine and router pipeline.
//! Only the provider is synthetic; no external model endpoint is called.
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::channel::{ChannelWriter, StreamChannelRef};
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, IIIClient, RegisterFunction};
use llm_router::chat::chat::{ChatCall, ChatFnInput};
use llm_router::config::entry::{register_entry, write_entry_value};
use llm_router::register::register_router;
use llm_router::types::router::ChatRequest;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

#[path = "support/engine_fixture.rs"]
mod engine_fixture;
use engine_fixture::*;

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Value {
    iii.trigger(TriggerRequest {
        function_id: function_id.into(),
        payload,
        action: None,
        timeout_ms: Some(15_000),
    })
    .await
    .unwrap_or_else(|error| panic!("{function_id}: {error}"))
}

async fn start_router(url: &str, retry_max: u32) -> IIIClient {
    let router = register_worker(url, test_init_options());
    register_entry(&router, &BTreeMap::new()).await.unwrap();
    write_entry_value(&router, json!({ "settings": { "retry_max": retry_max } }))
        .await
        .unwrap();
    register_router(router.clone()).await.unwrap();
    router
}

/// Every dispatch fails before its first content frame, so every allowed retry
/// is observable. The optional gate holds the first dispatch across a config change.
async fn failing_provider(
    url: &str,
    gate: Option<(Arc<Semaphore>, Arc<Semaphore>)>,
) -> (IIIClient, Arc<AtomicU64>) {
    let provider = register_worker(url, test_init_options());
    let calls = Arc::new(AtomicU64::new(0));
    let observed = calls.clone();
    let address = url.to_owned();
    provider.register_function(
        "provider::retry-fixture::stream",
        RegisterFunction::new_async(move |input: Value| {
            let (calls, address, gate) = (observed.clone(), address.clone(), gate.clone());
            async move {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                if attempt == 0 {
                    if let Some((entered, release)) = gate {
                        entered.add_permits(1);
                        release.acquire().await.unwrap().forget();
                    }
                }
                let writer_ref: StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone()).unwrap();
                let writer = ChannelWriter::new(&address, &writer_ref);
                writer
                    .send_message(
                        &json!({
                            "type": "error",
                            "error": {
                                "role": "assistant", "content": [],
                                "stop_reason": "error", "model": input["model"],
                                "provider": "retry-fixture", "timestamp": 1,
                                "error_message": "synthetic transient failure",
                                "error_kind": "transient"
                            }
                        })
                        .to_string(),
                    )
                    .await
                    .map_err(|error| Error::Handler(error.to_string()))?;
                let _ = writer.close().await;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        }),
    );
    call(
        &provider,
        "router::provider::register",
        json!({
            "id": "retry-fixture",
            "models": [{
                "id": "retry-model", "provider": "retry-fixture",
                "context_window": 1_000, "max_output_tokens": 100
            }]
        }),
    )
    .await;
    (provider, calls)
}

async fn failing_turn(consumer: &IIIClient, streaming: bool, retry_max: Option<u32>) {
    let mut payload = json!({ "model": "retry-model", "messages": [] });
    if let Some(limit) = retry_max {
        payload["retry_max"] = json!(limit);
    }
    if !streaming {
        let response = call(consumer, "router::complete", payload).await;
        assert_eq!(response["message"]["stop_reason"], "error", "{response}");
        return;
    }
    let channel = iii_sdk::helpers::create_channel(consumer, None)
        .await
        .unwrap();
    payload["writer_ref"] = serde_json::to_value(&channel.writer_ref).unwrap();
    let terminals = Arc::new(AtomicU64::new(0));
    let observed = terminals.clone();
    channel
        .reader
        .on_message(move |message| {
            let frame: Value = serde_json::from_str(&message).unwrap();
            if matches!(frame["type"].as_str(), Some("done" | "error")) {
                observed.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await;
    let pump = tokio::spawn(async move { channel.reader.read_all().await });
    let response = call(consumer, "router::chat", payload).await;
    assert_eq!(response["ok"], false, "{response}");
    tokio::time::timeout(Duration::from_secs(3), pump)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(terminals.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn request_retry_ceiling_applies_to_chat_and_complete() {
    let engine = engine_or_skip!();
    let router = start_router(&engine.url, 2).await;
    let (provider, calls) = failing_provider(&engine.url, None).await;
    let consumer = register_worker(&engine.url, test_init_options());
    for streaming in [false, true] {
        for (limit, expected) in [
            (None, 3),
            (Some(0), 1),
            (Some(1), 2),
            (Some(u32::MAX), 3),
            (None, 3),
        ] {
            let before = calls.load(Ordering::SeqCst);
            failing_turn(&consumer, streaming, limit).await;
            assert_eq!(
                calls.load(Ordering::SeqCst) - before,
                expected,
                "streaming={streaming}, caller retry_max={limit:?}"
            );
        }
    }
    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn caller_cannot_enable_globally_disabled_retries() {
    let engine = engine_or_skip!();
    let router = start_router(&engine.url, 0).await;
    let (provider, calls) = failing_provider(&engine.url, None).await;
    let consumer = register_worker(&engine.url, test_init_options());
    for streaming in [false, true] {
        for limit in [None, Some(0), Some(2)] {
            let before = calls.load(Ordering::SeqCst);
            failing_turn(&consumer, streaming, limit).await;
            assert_eq!(calls.load(Ordering::SeqCst) - before, 1);
        }
    }
    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn retry_ceiling_is_frozen_for_an_inflight_turn() {
    let engine = engine_or_skip!();
    let router = start_router(&engine.url, 0).await;
    let entered = Arc::new(Semaphore::new(0));
    let release = Arc::new(Semaphore::new(0));
    let (provider, calls) =
        failing_provider(&engine.url, Some((entered.clone(), release.clone()))).await;
    let consumer = register_worker(&engine.url, test_init_options());
    let turn_client = consumer.clone();
    let turn = tokio::spawn(async move { failing_turn(&turn_client, false, Some(2)).await });
    tokio::time::timeout(Duration::from_secs(3), entered.acquire())
        .await
        .unwrap()
        .unwrap()
        .forget();
    write_entry_value(&router, json!({ "settings": { "retry_max": 2 } }))
        .await
        .unwrap();
    call(&router, "router::on_config_changed", json!({})).await;
    release.add_permits(1);
    turn.await.unwrap();
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "inflight call keeps its original ceiling"
    );
    failing_turn(&consumer, false, None).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        4,
        "next call sees the new global ceiling"
    );
    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[test]
fn retry_ceiling_survives_the_public_request_wire_contract() {
    for limit in [None, Some(0), Some(1), Some(u32::MAX)] {
        let mut value = json!({
            "writer_ref": { "channel_id": "fixture", "access_key": "fixture", "direction": "write" },
            "model": "retry-model", "messages": []
        });
        if let Some(limit) = limit {
            value["retry_max"] = json!(limit);
        }
        let request: ChatRequest = serde_json::from_value(value).unwrap();
        let wire = serde_json::to_value(request).unwrap();
        assert_eq!(wire.get("retry_max"), limit.map(|n| json!(n)).as_ref());
        let chat: ChatFnInput = serde_json::from_value(wire.clone()).unwrap();
        let complete: ChatCall = serde_json::from_value(wire).unwrap();
        assert_eq!(chat.call.retry_max, limit);
        assert_eq!(complete.retry_max, limit);
    }
    for invalid in [json!(-1), json!(1.5), json!("0"), json!(u64::MAX)] {
        let value = json!({
            "writer_ref": { "channel_id": "fixture", "access_key": "fixture", "direction": "write" },
            "model": "retry-model", "messages": [], "retry_max": invalid
        });
        assert!(serde_json::from_value::<ChatRequest>(value.clone()).is_err());
        assert!(serde_json::from_value::<ChatFnInput>(value.clone()).is_err());
        assert!(serde_json::from_value::<ChatCall>(value).is_err());
    }
}
