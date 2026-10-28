//! `pubsub::on-message` (iii 0.25 observe) without an engine: the function
//! refuses direct invocation, and an observer filed through its hooks
//! receives exactly the message a `subscribe` trigger subscriber receives.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_pubsub::adapters::{Invoker, LocalAdapter};
use iii_pubsub::hub::Hub;
use iii_pubsub::on_message::{
    observer_key, parse_args, refuse_invocation, ObserverRouter, ObserverSink, Observers,
    OnMessageObservers, PubSubMessage, ON_MESSAGE_FN,
};
use iii_pubsub::trigger::{SubscribeTriggerHandler, SubscribeTriggerSpec};
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use serde_json::{json, Value};

/// Records what reached a trigger's function through `iii.trigger`.
#[derive(Default)]
struct RecordingInvoker {
    calls: Mutex<Vec<(String, Value)>>,
}

#[async_trait]
impl Invoker for RecordingInvoker {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
        self.calls
            .lock()
            .unwrap()
            .push((function_id.to_string(), payload));
        Ok(None)
    }
}

/// Stands in for the SDK observer: records what `obs.call` would deliver.
struct RecordingSink {
    id: String,
    calls: Mutex<Vec<Value>>,
}

impl RecordingSink {
    fn new(id: &str) -> Arc<Self> {
        Arc::new(Self {
            id: id.to_string(),
            calls: Mutex::new(Vec::new()),
        })
    }

    fn calls(&self) -> Vec<Value> {
        self.calls.lock().unwrap().clone()
    }
}

#[async_trait]
impl ObserverSink for RecordingSink {
    fn id(&self) -> &str {
        &self.id
    }

    async fn call(&self, payload: Value) -> Result<Option<Value>, String> {
        self.calls.lock().unwrap().push(payload);
        Ok(None)
    }
}

struct Fixture {
    hub: Arc<Hub>,
    triggers: Arc<RecordingInvoker>,
    observers: Observers,
    hooks: OnMessageObservers,
    handler: SubscribeTriggerHandler,
}

/// The boot wiring (boot.rs) over the local adapter, with a recording
/// invoker in place of `iii.trigger`.
fn fixture() -> Fixture {
    let triggers = Arc::new(RecordingInvoker::default());
    let observers = Observers::default();
    let router: Arc<dyn Invoker> = Arc::new(ObserverRouter::new(
        observers.clone(),
        triggers.clone() as Arc<dyn Invoker>,
    ));
    let hub = Arc::new(Hub::new(Arc::new(LocalAdapter::new(router))));
    Fixture {
        hooks: OnMessageObservers::new(hub.clone(), observers.clone()),
        handler: SubscribeTriggerHandler::new(hub.clone()),
        hub,
        triggers,
        observers,
    }
}

fn spec(topic: &str) -> SubscribeTriggerSpec {
    SubscribeTriggerSpec {
        topic: topic.to_string(),
        condition_function_id: None,
    }
}

async fn bind_trigger(handler: &SubscribeTriggerHandler, id: &str, function_id: &str, topic: &str) {
    handler
        .register_trigger(TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config: json!({ "topic": topic }),
            metadata: None,
            namespace: None,
        })
        .await
        .expect("subscribe trigger registers");
}

/// Delivery is fire-and-forget (spawned): wait until `done` holds.
async fn eventually(done: impl Fn() -> bool) {
    for _ in 0..200 {
        if done() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!("delivery did not happen in time");
}

#[test]
fn direct_invocation_is_forbidden() {
    let err = refuse_invocation(PubSubMessage(json!({ "id": 1 })))
        .expect_err("on-message must refuse direct calls");
    match err {
        Error::Remote { code, message, .. } => {
            assert_eq!(code, observe_codes::FORBIDDEN);
            assert!(
                message.contains(ON_MESSAGE_FN),
                "message names the function: {message}"
            );
        }
        other => panic!("expected a FORBIDDEN remote error, got {other:?}"),
    }
}

#[test]
fn args_are_the_subscribe_trigger_config() {
    let parsed = parse_args(&json!({ "topic": "orders", "condition_function_id": "c::f" }))
        .expect("a subscribe config is valid observer args");
    assert_eq!(parsed.topic, "orders");
    assert_eq!(parsed.condition_function_id.as_deref(), Some("c::f"));

    for bad in [json!({}), json!({ "topic": "" }), json!({ "topic": 7 })] {
        let rejection = parse_args(&bad).expect_err("invalid args are rejected");
        assert_eq!(rejection.code, observe_codes::INVALID_ARGS, "args {bad}");
    }
}

#[tokio::test]
async fn observer_receives_the_message_a_trigger_subscriber_receives() {
    let f = fixture();
    bind_trigger(&f.handler, "trigger-1", "app::on-order", "orders").await;
    let sink = RecordingSink::new("obs-1");
    f.hooks
        .add(sink.clone(), spec("orders"))
        .await
        .expect("observer is filed");
    assert_eq!(f.hub.subscription_count().await, 2);
    assert!(f.observers.get(&observer_key("obs-1")).is_some());

    let data = json!({ "id": 42, "items": ["a", "b"] });
    f.hub.publish("orders", data.clone()).await.unwrap();
    // Another topic reaches neither.
    f.hub.publish("invoices", json!({ "id": 1 })).await.unwrap();

    eventually(|| !sink.calls().is_empty() && !f.triggers.calls.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    let trigger_calls = f.triggers.calls.lock().unwrap().clone();
    assert_eq!(
        trigger_calls,
        vec![("app::on-order".to_string(), data.clone())]
    );
    assert_eq!(sink.calls(), vec![data]);
    // The observer key never reaches iii.trigger.
    assert!(trigger_calls
        .iter()
        .all(|(id, _)| !id.starts_with("observer:")));
}

#[tokio::test]
async fn removed_observer_stops_receiving_and_triggers_are_unchanged() {
    let f = fixture();
    bind_trigger(&f.handler, "trigger-1", "app::on-order", "orders").await;
    let sink = RecordingSink::new("obs-1");
    f.hooks.add(sink.clone(), spec("orders")).await.unwrap();

    f.hooks.remove("obs-1").await;
    assert!(f.observers.is_empty());
    assert_eq!(f.hub.subscription_count().await, 1);

    f.hub.publish("orders", json!({ "id": 2 })).await.unwrap();
    eventually(|| !f.triggers.calls.lock().unwrap().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert!(sink.calls().is_empty(), "a removed observer gets nothing");
    assert_eq!(
        f.triggers.calls.lock().unwrap().clone(),
        vec![("app::on-order".to_string(), json!({ "id": 2 }))]
    );
}

#[tokio::test]
async fn re_adding_an_observer_replaces_it() {
    let f = fixture();
    let first = RecordingSink::new("obs-1");
    let second = RecordingSink::new("obs-1");
    f.hooks.add(first.clone(), spec("orders")).await.unwrap();
    f.hooks.add(second.clone(), spec("payments")).await.unwrap();
    assert_eq!(f.observers.len(), 1);
    assert_eq!(f.hub.subscription_count().await, 1);

    f.hub.publish("orders", json!(1)).await.unwrap();
    f.hub.publish("payments", json!(2)).await.unwrap();
    eventually(|| !second.calls().is_empty()).await;
    tokio::time::sleep(Duration::from_millis(20)).await;

    assert!(first.calls().is_empty());
    assert_eq!(second.calls(), vec![json!(2)]);
}

#[tokio::test]
async fn empty_topic_observer_is_rejected() {
    let f = fixture();
    let rejection = f
        .hooks
        .add(RecordingSink::new("obs-1"), spec(""))
        .await
        .expect_err("an empty topic can never receive a message");
    assert_eq!(rejection.code, observe_codes::INVALID_ARGS);
    assert!(f.observers.is_empty());
    assert_eq!(f.hub.subscription_count().await, 0);
}
