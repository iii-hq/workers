use std::sync::Arc;
use std::time::Duration;

use llm_router::bus::{handler, Bus};
use llm_router::chat::relay::{ChannelFactory, ReadEvent, RelayRead};
use llm_router::register::register_router;
use llm_router::testkit::fake_bus::FakeBus;
use llm_router::testkit::fake_channels::{ChannelHub, FakeChannel};
use serde_json::json;

#[tokio::test]
async fn boots_serves_the_full_surface_and_emits_ready_to_prebound_subscribers() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();

    // a provider-side subscriber binds BEFORE the router boots (boot-order recovery)
    let ready = Arc::new(std::sync::Mutex::new(0u32));
    let r2 = ready.clone();
    bus.register_function(
        "prov::on_ready",
        handler(move |_| {
            let r = r2.clone();
            async move {
                *r.lock().unwrap() += 1;
                Ok(serde_json::Value::Null)
            }
        }),
    );
    bus.register_trigger("router::ready", "prov::on_ready", json!({}));

    register_router(
        bus.clone() as Arc<dyn Bus>,
        hub.clone() as Arc<dyn ChannelFactory>,
    )
    .await
    .unwrap();
    assert_eq!(*ready.lock().unwrap(), 1);

    // register → topology flip → models list → chat → complete, all over the bus surface
    let reg = bus
        .trigger(
            "router::provider::register",
            json!({
                "id": "fake", "worker_id": "w1", "defaults": { "max_tokens": 8192 },
                "models": [{ "id": "m1", "provider": "fake", "context_window": 1000, "max_output_tokens": 500 }]
            }),
            None,
        )
        .await
        .unwrap();
    assert!(reg["registration_token"].as_str().unwrap().len() >= 16);
    bus.emit_topic(
        "engine::workers-available",
        json!({ "event": "worker_connected", "worker_id": "w1" }),
    )
    .await;

    let list = bus
        .trigger("router::models::list", json!({}), None)
        .await
        .unwrap();
    assert_eq!(list["models"][0]["id"], "m1");
    let providers = bus
        .trigger("router::provider::list", json!({}), None)
        .await
        .unwrap();
    assert_eq!(providers["providers"][0]["available"], true);

    // scripted provider for the chat path
    let hub2 = hub.clone();
    bus.register_function(
        "provider::fake::stream",
        handler(move |input: serde_json::Value| {
            let hub = hub2.clone();
            async move {
                let r: llm_router::types::channel::StreamChannelRef =
                    serde_json::from_value(input["writer_ref"].clone()).unwrap();
                let writer = hub.writer_for(&r).unwrap();
                let message = json!({
                    "role": "assistant", "content": [{ "type": "text", "text": "hi" }],
                    "stop_reason": "end", "model": input["model"], "provider": "fake", "timestamp": 2
                });
                use llm_router::chat::relay::FrameSink;
                writer.send(&json!({ "type": "start", "partial": { "role": "assistant", "content": [], "stop_reason": "end", "model": input["model"], "provider": "fake", "timestamp": 1 } }).to_string()).unwrap();
                writer
                    .send(&json!({ "type": "done", "message": message }).to_string())
                    .unwrap();
                writer.close();
                Ok(json!({ "ok": true }))
            }
        }),
    );

    // chat: caller channel comes from the hub so the router can open_sink its ref
    let caller = FakeChannel::new();
    hub.adopt(&caller); // registers the caller's writer under its channel_id
    let res = bus
        .trigger(
            "router::chat",
            json!({ "writer_ref": caller.writer_ref, "model": "m1", "messages": [] }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(res["ok"], true);
    assert_eq!(res["stop_reason"], "end");
    let mut reader = caller.reader;
    assert!(matches!(
        reader.next(Duration::from_millis(100)).await,
        ReadEvent::Msg(_)
    ));

    let completed = bus
        .trigger(
            "router::complete",
            json!({ "model": "m1", "messages": [] }),
            None,
        )
        .await
        .unwrap();
    assert_eq!(completed["message"]["content"][0]["text"], "hi");
}

#[tokio::test]
async fn restart_restores_the_registry_from_state() {
    let bus = FakeBus::new();
    let hub = ChannelHub::new();
    register_router(
        bus.clone() as Arc<dyn Bus>,
        hub.clone() as Arc<dyn ChannelFactory>,
    )
    .await
    .unwrap();
    bus.trigger(
        "router::provider::register",
        json!({ "id": "fake", "worker_id": "w1" }),
        None,
    )
    .await
    .unwrap();

    register_router(bus.clone() as Arc<dyn Bus>, hub as Arc<dyn ChannelFactory>)
        .await
        .unwrap(); // "restart"
    let providers = bus
        .trigger("router::provider::list", json!({}), None)
        .await
        .unwrap();
    assert_eq!(providers["providers"][0]["id"], "fake");
}
