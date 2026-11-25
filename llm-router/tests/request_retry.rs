//! Caller-specific retry ceilings on the real engine and router pipeline.
//! Only the provider is synthetic; no external model endpoint is called.
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::StreamChannelRef;
use iii_sdk::{CallRequest, IIIClient, RegisterFunction, register_worker};
use llm_router::chat::chat::{ChatCall, ChatFnInput};
use llm_router::config::entry::{register_entry, write_entry_value};
use llm_router::register::register_router;
use llm_router::types::router::ChatRequest;
use serde_json::{Value, json};
use tokio::sync::Semaphore;

#[path = "support/engine_fixture.rs"]
mod engine_fixture;
use engine_fixture::*;

async fn call(iii: &IIIClient, function_id: &str, payload: Value) -> Value {
    iii.call(CallRequest {
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
    provider.register_function(
        "provider::retry-fixture::stream",
        RegisterFunction::new_async(move |input: Value| {
            let (calls, gate) = (observed.clone(), gate.clone());
            async move {
                let attempt = calls.fetch_add(1, Ordering::SeqCst);
                if attempt == 0
                    && let Some((entered, release)) = gate
                {
                    entered.add_permits(1);
                    release.acquire().await.unwrap().forget();
                }
                let writer_ref: StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone()).unwrap();
                let writer = open_writer(&writer_ref);
                writer
                    .send_text(
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
    let pipe = consumer.pipe().await.unwrap();
    payload["writer_ref"] = serde_json::to_value(ref_of(&pipe.output)).unwrap();
    let terminals = Arc::new(AtomicU64::new(0));
    let observed = terminals.clone();
    let input = pipe.input;
    let pump = tokio::spawn(async move {
        use futures::StreamExt;
        let mut items = input.frames();
        while let Some(item) = items.next().await {
            if let iii_sdk::protocol::ChannelItem::Text(message) = item? {
                let frame: Value = serde_json::from_str(&message).unwrap();
                if matches!(frame["type"].as_str(), Some("done" | "error")) {
                    observed.fetch_add(1, Ordering::SeqCst);
                }
            }
        }
        Ok::<(), Error>(())
    });
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

/// Every dispatch answers a pre-content `rate_limited` error terminal, with
/// `error_details` when given (an account's usage limit, D147).
async fn rate_limited_provider(
    url: &str,
    id: &str,
    error_details: Option<Value>,
) -> (IIIClient, Arc<AtomicU64>) {
    let provider = register_worker(url, test_init_options());
    let calls = Arc::new(AtomicU64::new(0));
    let observed = calls.clone();
    let provider_id = id.to_string();
    provider.register_function(
        format!("provider::{id}::stream"),
        RegisterFunction::new_async(move |input: Value| {
            let (calls, details, provider_id) =
                (observed.clone(), error_details.clone(), provider_id.clone());
            async move {
                calls.fetch_add(1, Ordering::SeqCst);
                let writer_ref: StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone()).unwrap();
                let writer = open_writer(&writer_ref);
                let mut error = json!({
                    "role": "assistant", "content": [],
                    "stop_reason": "error", "model": input["model"],
                    "provider": provider_id, "timestamp": 1,
                    "error_message": "429 rate_limit_error",
                    "error_kind": "rate_limited"
                });
                if let Some(details) = details {
                    error["error_details"] = details;
                }
                writer
                    .send_text(&json!({ "type": "error", "error": error }).to_string())
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
            "id": id,
            "models": [{
                "id": format!("{id}-model"), "provider": id,
                "context_window": 1_000, "max_output_tokens": 100
            }]
        }),
    )
    .await;
    (provider, calls)
}

/// One streamed `router::chat` turn: the response and every terminal frame
/// the caller received.
async fn streamed_turn(consumer: &IIIClient, model: &str) -> (Value, Vec<Value>) {
    let pipe = consumer.pipe().await.unwrap();
    let payload = json!({
        "model": model,
        "messages": [],
        "writer_ref": serde_json::to_value(ref_of(&pipe.output)).unwrap(),
    });
    let input = pipe.input;
    let pump = tokio::spawn(async move {
        use futures::StreamExt;
        let mut terminals = Vec::new();
        let mut items = input.frames();
        while let Some(item) = items.next().await {
            if let iii_sdk::protocol::ChannelItem::Text(message) = item? {
                let frame: Value = serde_json::from_str(&message).unwrap();
                if matches!(frame["type"].as_str(), Some("done" | "error")) {
                    terminals.push(frame);
                }
            }
        }
        Ok::<Vec<Value>, Error>(terminals)
    });
    let response = call(consumer, "router::chat", payload).await;
    let terminals = tokio::time::timeout(Duration::from_secs(3), pump)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    (response, terminals)
}

#[tokio::test(flavor = "multi_thread")]
async fn a_usage_limit_is_not_retried_and_its_details_reach_the_caller() {
    let engine = engine_or_skip!();
    let router = start_router(&engine.url, 2).await;
    let details = json!({
        "usage_limit": true, "window": "five_hour", "resets_at_ms": 1_800_000_000_000u64
    });
    let (provider, calls) =
        rate_limited_provider(&engine.url, "limit-fixture", Some(details.clone())).await;
    let consumer = register_worker(&engine.url, test_init_options());

    let (response, terminals) = streamed_turn(&consumer, "limit-fixture-model").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a usage limit is never retried"
    );
    assert_eq!(response["ok"], false, "{response}");
    let error = &response["error"];
    assert_eq!(error["code"], "router/provider_rate_limited", "{response}");
    assert_eq!(error["kind"], "rate_limited", "{response}");
    assert_eq!(error["retryable"], false, "{response}");
    assert_eq!(error["error_details"], details, "{response}");
    assert_eq!(terminals.len(), 1, "{terminals:?}");
    assert_eq!(terminals[0]["error"]["error_kind"], "rate_limited");
    assert_eq!(terminals[0]["error"]["error_details"], details);

    let completed = call(
        &consumer,
        "router::complete",
        json!({ "model": "limit-fixture-model", "messages": [] }),
    )
    .await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        2,
        "complete does not retry either"
    );
    assert_eq!(
        completed["message"]["error_details"], details,
        "{completed}"
    );

    consumer.shutdown();
    provider.shutdown();
    router.shutdown();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_busy_rate_limit_is_still_retried() {
    let engine = engine_or_skip!();
    let router = start_router(&engine.url, 2).await;
    let (provider, calls) = rate_limited_provider(&engine.url, "busy-fixture", None).await;
    let consumer = register_worker(&engine.url, test_init_options());

    let (response, terminals) = streamed_turn(&consumer, "busy-fixture-model").await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        3,
        "first attempt plus two retries"
    );
    assert_eq!(response["error"]["retryable"], true, "{response}");
    assert!(
        response["error"].get("error_details").is_none(),
        "{response}"
    );
    assert_eq!(terminals.len(), 1, "{terminals:?}");
    assert!(terminals[0]["error"].get("error_details").is_none());

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
