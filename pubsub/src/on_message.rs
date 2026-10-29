//! `pubsub::on-message`: the `subscribe` trigger type as an observable
//! function (iii 0.25 observe).
//!
//! The function is owner-delivered. Its observers' args are a `subscribe`
//! trigger's config ([`SubscribeTriggerSpec`]: `{ topic, condition_function_id? }`).
//! [`OnMessageObservers`] is its [`ObserveHandler`]: each observer added is
//! filed in the same [`Hub`] that serves the trigger type, under its topic, as
//! one more subscription keyed `observer:<id>` (never a trigger id); a removed
//! observer (disposed, or its worker gone past the engine's grace period) is
//! unsubscribed exactly like an unregistered trigger. The adapters fan each
//! message out through the [`Invoker`]; [`ObserverRouter`] hands the ones
//! addressed to an observer key to that observer with `obs.call`, and every
//! other one to the trigger's function as before. Delivery stays
//! fire-and-forget, as for a trigger subscriber.
//!
//! The function itself refuses invocations (`FORBIDDEN`): messages reach
//! observers through the hub only, so a caller cannot inject messages that
//! were never published.
//!
//! Old consumers keep binding the `subscribe` trigger type, which this worker
//! still registers on every engine. The function declares no
//! `legacy_trigger_type`: this worker is that trigger type's provider. On an
//! engine without observe the SDK registers this function as a plain function
//! and its hooks never run.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::observe_codes;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::adapters::Invoker;
use crate::hub::Hub;
use crate::trigger::SubscribeTriggerSpec;

/// The observable function a topic's messages are delivered through.
pub const ON_MESSAGE_FN: &str = "pubsub::on-message";

pub const ON_MESSAGE_DESCRIPTION: &str =
    "One invocation per message published to a pubsub topic. Observe it with args \
     { topic, condition_function_id? } (the subscribe trigger config); every observer of a \
     topic receives every message published to it, fire-and-forget, exactly as a subscribe \
     trigger does. Your target receives the published data. Only the pubsub worker delivers \
     its messages: invoking it directly is refused.";

/// Prefix of the hub subscription key (and routed function id) of an
/// observer: trigger ids never carry it.
pub const OBSERVER_PREFIX: &str = "observer:";

/// The hub subscription key of observer `id`.
pub fn observer_key(id: &str) -> String {
    format!("{OBSERVER_PREFIX}{id}")
}

/// Registers `pubsub::on-message` with an [`OnMessageObservers`] over `hub`
/// and `observers` as its hooks.
pub fn register(iii: &IIIClient, hub: Arc<Hub>, observers: Observers) -> FunctionRef {
    iii.register_function(
        ON_MESSAGE_FN,
        RegisterFunction::new(refuse_invocation)
            .description(ON_MESSAGE_DESCRIPTION)
            .observe(
                Observe::owner()
                    .args_format::<SubscribeTriggerSpec>()
                    .handler(OnMessageObservers::new(hub, observers)),
            ),
    )
}

/// What a `pubsub::on-message` target receives: the data published to the
/// topic, unchanged. Published data is not always an object, so this keeps a
/// lossless `Value` while publishing a real schema (the registry publish gate
/// rejects `AnyValue`).
#[derive(Debug, Clone, Deserialize)]
#[serde(transparent)]
pub struct PubSubMessage(pub Value);

impl JsonSchema for PubSubMessage {
    fn schema_name() -> String {
        "PubSubMessage".to_string()
    }

    fn json_schema(_: &mut schemars::r#gen::SchemaGenerator) -> schemars::schema::Schema {
        use schemars::schema::{InstanceType, Metadata, SchemaObject};
        SchemaObject {
            instance_type: Some(
                vec![
                    InstanceType::Null,
                    InstanceType::Boolean,
                    InstanceType::Number,
                    InstanceType::String,
                    InstanceType::Array,
                    InstanceType::Object,
                ]
                .into(),
            ),
            metadata: Some(Box::new(Metadata {
                description: Some("The data published to the topic, as published.".to_string()),
                ..Default::default()
            })),
            ..Default::default()
        }
        .into()
    }
}

/// The function's handler. Messages are delivered through the hub with
/// `obs.call`, never by invoking the function.
pub fn refuse_invocation(_message: PubSubMessage) -> Result<(), Error> {
    Err(Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_MESSAGE_FN} is delivered by the pubsub worker: observe it \
             (iii.observe({{ function_id: '{ON_MESSAGE_FN}', args: {{ topic }} }}, target)) \
             and publish with publish"
        ),
        stacktrace: None,
    })
}

/// An observer's args, parsed as the trigger type's config. An empty topic
/// is rejected: a trigger with one registers nothing, and an observer that
/// can never receive a message is a caller error.
pub fn parse_args(args: &Value) -> Result<SubscribeTriggerSpec, ObserveRejection> {
    let spec: SubscribeTriggerSpec = serde_json::from_value(args.clone()).map_err(|e| {
        ObserveRejection::new(
            observe_codes::INVALID_ARGS,
            format!("invalid {ON_MESSAGE_FN} args: {e}"),
        )
    })?;
    if spec.topic.is_empty() {
        return Err(ObserveRejection::new(
            observe_codes::INVALID_ARGS,
            format!("invalid {ON_MESSAGE_FN} args: topic is empty"),
        ));
    }
    Ok(spec)
}

/// Where a message for an observer goes: the SDK [`Observer`] in production,
/// a fake in tests.
#[async_trait]
pub trait ObserverSink: Send + Sync + 'static {
    fn id(&self) -> &str;
    async fn call(&self, payload: Value) -> Result<Option<Value>, String>;
}

/// What a finished `obs.call` means: the observer's filters skipping the
/// message is not a failure; any other error is reported to the caller (the
/// fan-out logs it, as for a trigger target).
pub fn call_outcome(result: Result<Value, Error>) -> Result<Option<Value>, String> {
    match result {
        Ok(output) => Ok(Some(output)),
        Err(Error::Remote { code, .. }) if code == observe_codes::FILTERS_NOT_MATCHED => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

#[async_trait]
impl ObserverSink for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn call(&self, payload: Value) -> Result<Option<Value>, String> {
        call_outcome(Observer::call::<Value>(self, payload).await)
    }
}

/// The live observers by hub subscription key ([`observer_key`]), shared by
/// the hooks and the [`ObserverRouter`].
#[derive(Clone, Default)]
pub struct Observers {
    sinks: Arc<RwLock<HashMap<String, Arc<dyn ObserverSink>>>>,
}

impl Observers {
    pub fn get(&self, key: &str) -> Option<Arc<dyn ObserverSink>> {
        self.sinks
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .get(key)
            .cloned()
    }

    fn insert(&self, key: String, sink: Arc<dyn ObserverSink>) -> bool {
        self.sinks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, sink)
            .is_some()
    }

    fn remove(&self, key: &str) {
        self.sinks
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(key);
    }

    pub fn len(&self) -> usize {
        self.sinks.read().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// The adapters' [`Invoker`]: a delivery addressed to an observer key goes to
/// that observer (`obs.call`); every other one to `inner` (the trigger's
/// function through `iii.trigger`), unchanged.
pub struct ObserverRouter {
    observers: Observers,
    inner: Arc<dyn Invoker>,
}

impl ObserverRouter {
    pub fn new(observers: Observers, inner: Arc<dyn Invoker>) -> Self {
        Self { observers, inner }
    }
}

#[async_trait]
impl Invoker for ObserverRouter {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
        if function_id.starts_with(OBSERVER_PREFIX) {
            return match self.observers.get(function_id) {
                Some(sink) => sink.call(payload).await,
                // Removed between the fan-out snapshot and this call.
                None => Err(format!("{function_id} is no longer observing")),
            };
        }
        self.inner.call(function_id, payload).await
    }
}

/// The hooks of `pubsub::on-message`: files each observer in the hub.
#[derive(Clone)]
pub struct OnMessageObservers {
    hub: Arc<Hub>,
    observers: Observers,
}

impl OnMessageObservers {
    pub fn new(hub: Arc<Hub>, observers: Observers) -> Self {
        Self { hub, observers }
    }

    /// Files the observer behind `sink` under `spec`'s topic. The subscription
    /// is bound before the SDK acks the observer, so no message published
    /// after `ready()` resolves is missed. Re-adding a live id replaces it.
    pub async fn add(
        &self,
        sink: Arc<dyn ObserverSink>,
        spec: SubscribeTriggerSpec,
    ) -> Result<(), ObserveRejection> {
        if spec.topic.is_empty() {
            return Err(ObserveRejection::new(
                observe_codes::INVALID_ARGS,
                format!("invalid {ON_MESSAGE_FN} args: topic is empty"),
            ));
        }
        let key = observer_key(sink.id());
        // The sink is routable before the hub can deliver to its key.
        if self.observers.insert(key.clone(), sink) {
            self.hub.unsubscribe(&key).await;
        }
        tracing::info!(topic = %spec.topic, observer = %key, "PubSub observer added");
        self.hub.subscribe(&key, &spec.topic, &key).await;
        Ok(())
    }

    /// Unsubscribes the observer `id`, exactly as an unregistered trigger.
    pub async fn remove(&self, id: &str) {
        let key = observer_key(id);
        self.hub.unsubscribe(&key).await;
        self.observers.remove(&key);
    }
}

#[async_trait]
impl ObserveHandler for OnMessageObservers {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        let spec = parse_args(obs.args_value())?;
        self.add(Arc::new(obs.clone()), spec).await
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.remove(obs.id()).await;
    }
}
