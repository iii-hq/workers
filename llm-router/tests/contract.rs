use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use llm_router::bus::Bus;
use llm_router::chat::relay::{ChannelFactory, FrameSink, ReadEvent, RelayRead};
use llm_router::register::register_router;
use llm_router::testkit::fake_bus::FakeBus;
use llm_router::testkit::fake_channels::{ChannelHub, FakeChannel};
use llm_router::testkit::scripted_provider::{start_scripted_provider, ScriptedOptions};
use serde_json::json;

fn model(id: &str, provider: &str) -> llm_router::types::model::Model {
    serde_json::from_value(
        json!({ "id": id, "provider": provider, "context_window": 100000, "max_output_tokens": 8192 }),
    )
    .unwrap()
}

#[tokio::test(start_paused = true)]
async fn provider_before_router_boot_order_declares_until_acked() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();
    let provider = start_scripted_provider(bus.clone(), hub.clone(), ScriptedOptions::default());
    tokio::time::sleep(Duration::from_millis(50)).await; // several failed declares
    assert!(provider.declare_attempts.load(Ordering::SeqCst) > 1);
    assert!(provider.token.lock().unwrap().is_none());

    register_router(bus.clone() as Arc<dyn Bus>, hub as Arc<dyn ChannelFactory>)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await; // retry loop or ready re-declare lands
    assert!(provider.token.lock().unwrap().is_some());
}

#[tokio::test(start_paused = true)]
async fn router_restart_redeclare_is_idempotent_with_the_same_token() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();
    register_router(
        bus.clone() as Arc<dyn Bus>,
        hub.clone() as Arc<dyn ChannelFactory>,
    )
    .await
    .unwrap();
    let provider = start_scripted_provider(bus.clone(), hub.clone(), ScriptedOptions::default());
    tokio::time::sleep(Duration::from_millis(50)).await;
    let before = provider.token.lock().unwrap().clone().unwrap();

    register_router(bus.clone() as Arc<dyn Bus>, hub as Arc<dyn ChannelFactory>)
        .await
        .unwrap(); // "restart"
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(provider.token.lock().unwrap().clone().unwrap(), before);
}

#[tokio::test(start_paused = true)]
async fn paste_a_key_config_edit_debounced_refresh_reconcile_models_listed() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();
    register_router(
        bus.clone() as Arc<dyn Bus>,
        hub.clone() as Arc<dyn ChannelFactory>,
    )
    .await
    .unwrap();
    let provider = start_scripted_provider(
        bus.clone(),
        hub,
        ScriptedOptions {
            supports_model_listing: true,
            discover: Some(Arc::new(|| Some(vec![model("disc-1", "scripted")]))),
            ..Default::default()
        },
    );
    tokio::time::sleep(Duration::from_millis(50)).await;

    bus.trigger(
        "configuration::set",
        json!({ "id": "llm-router", "value": { "providers": { "scripted": { "api_key": "sk-pasted" } } } }),
        None,
    )
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(2100)).await; // > debounce (paused clock: instant)
    assert_eq!(provider.refresh_calls.load(Ordering::SeqCst), 1);
    let list = bus
        .trigger("router::models::list", json!({}), None)
        .await
        .unwrap();
    assert_eq!(list["models"][0]["id"], "disc-1");
}

#[tokio::test(start_paused = true)]
async fn abort_via_router_abort_stops_upstream_and_terminates_with_done_aborted() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();
    register_router(
        bus.clone() as Arc<dyn Bus>,
        hub.clone() as Arc<dyn ChannelFactory>,
    )
    .await
    .unwrap();
    let upstream_aborted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let ua = upstream_aborted.clone();
    start_scripted_provider(
        bus.clone(),
        hub.clone(),
        ScriptedOptions {
            models: Some(vec![model("m1", "scripted")]),
            stream: Some(Arc::new(move |input, writer| {
                let ua = ua.clone();
                Box::pin(async move {
                    writer.send(&json!({ "type": "start", "partial": { "role": "assistant", "content": [], "stop_reason": "end", "model": input["model"], "provider": "scripted", "timestamp": 1 } }).to_string()).ok();
                    loop {
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        if writer.send(&json!({ "type": "ping" }).to_string()).is_err() {
                            ua.store(true, Ordering::SeqCst); // channel closed = upstream abort signal
                            return json!({ "ok": true, "status": "aborted" });
                        }
                    }
                })
            })),
            ..Default::default()
        },
    );
    tokio::time::sleep(Duration::from_millis(50)).await;

    let caller = FakeChannel::new();
    hub.adopt(&caller);
    let pending = {
        let bus = bus.clone();
        let writer_ref = caller.writer_ref.clone();
        tokio::spawn(async move {
            bus.trigger(
                "router::chat",
                json!({ "writer_ref": writer_ref, "request_id": "abort-me", "model": "m1", "messages": [] }),
                None,
            )
            .await
        })
    };
    tokio::time::sleep(Duration::from_millis(60)).await;
    let aborted = bus
        .trigger("router::abort", json!({ "request_id": "abort-me" }), None)
        .await
        .unwrap();
    assert_eq!(aborted, json!({ "aborted": true }));
    let res = pending.await.unwrap().unwrap();
    assert_eq!(res["stop_reason"], "aborted");
    tokio::time::sleep(Duration::from_millis(60)).await;
    assert!(upstream_aborted.load(Ordering::SeqCst));

    let mut reader = caller.reader;
    let mut last = None;
    while let ReadEvent::Msg(m) = reader.next(Duration::from_millis(50)).await {
        last = Some(serde_json::from_str::<serde_json::Value>(&m).unwrap());
    }
    let last = last.unwrap();
    assert_eq!(last["type"], "done");
    assert_eq!(last["message"]["stop_reason"], "aborted");
}

#[tokio::test(start_paused = true)]
async fn update_credential_persists_and_resolves_back() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();
    register_router(
        bus.clone() as Arc<dyn Bus>,
        hub.clone() as Arc<dyn ChannelFactory>,
    )
    .await
    .unwrap();
    let provider = start_scripted_provider(bus.clone(), hub, ScriptedOptions::default());
    tokio::time::sleep(Duration::from_millis(50)).await;
    let token = provider.token.lock().unwrap().clone().unwrap();

    bus.trigger(
        "router::provider::update_credential",
        json!({
            "id": "scripted", "token": token,
            "credential": { "type": "oauth", "access_token": "at-1", "expires_at": 999 }
        }),
        None,
    )
    .await
    .unwrap();
    let resolved = bus
        .trigger(
            "router::provider::resolve",
            json!({ "id": "scripted", "token": token }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(resolved["source"], "config");
    assert_eq!(resolved["credential"]["access_token"], "at-1");
}
