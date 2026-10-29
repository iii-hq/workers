//! The three custom trigger types this worker emits, the observable
//! `router::on-*` function next to each one, and the fan-out machinery behind
//! them (README § Events).
//!
//! Reactivity model: old consumers bind handlers to these trigger types with
//! the standard two-step pattern; the engine routes each registration to our
//! [`TriggerHandler`]s. New consumers observe the function (iii 0.25) with the
//! same config as args; its [`ObserveHandler`] files observers in the same
//! subscriber set. Delivery is fire-and-forget (`TriggerAction::Void` for
//! bindings, `obs.call` for observers); failures are logged and swallowed so a
//! misbehaving subscriber must not break the write path. Events are routed
//! in-process: invoking an on-* function is refused with FORBIDDEN, so nobody
//! can forge one.
//!
//! After a worker restart the engine replays every existing trigger
//! registration of a re-registered type (and every observer of a
//! re-registered function) back to its owner, so the subscriber sets rebuild
//! themselves.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::router::{ModelsChangedPayload, ProviderChangedPayload, RouterReadyEvent};

/// Fires when a provider reconciles its catalog slice.
pub const MODELS_CHANGED: &str = "router::models::changed";
/// Fires when the provider registry changes (declare / availability flip).
pub const PROVIDER_CHANGED: &str = "router::provider::changed";
/// Fires once when the router finishes booting; providers re-declare on it.
pub const READY: &str = "router::ready";

/// Observable function next to [`READY`].
pub const ON_READY: &str = "router::on-ready";
/// Observable function next to [`MODELS_CHANGED`].
pub const ON_MODELS_CHANGED: &str = "router::on-models-changed";
/// Observable function next to [`PROVIDER_CHANGED`].
pub const ON_PROVIDER_CHANGED: &str = "router::on-provider-changed";

const READY_DESC: &str =
    "The router finished booting; providers bind here and re-declare after a restart.";
const MODELS_CHANGED_DESC: &str =
    "A provider reconciled its catalog slice. Payload: { provider, count }.";
const PROVIDER_CHANGED_DESC: &str =
    "The provider registry changed (declare / availability flip). Payload: { provider, op }.";

const ON_READY_DESC: &str =
    "The router finished booting; providers re-declare on it. Observe it with args {}; each \
     observer receives {}. Only the llm-router worker emits it: invoking it directly is refused.";
const ON_MODELS_CHANGED_DESC: &str =
    "A provider reconciled its catalog slice. Observe it with args {}; each observer receives \
     { provider, count }. Only the llm-router worker emits it: invoking it directly is refused.";
const ON_PROVIDER_CHANGED_DESC: &str =
    "The provider registry changed (declare / availability flip). Observe it with args {}; each \
     observer receives { provider, op: register | available | unavailable }. Only the \
     llm-router worker emits it: invoking it directly is refused.";

/// Codes the `router::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
}

/// Config accepted by all three router event bindings, and the args of the
/// three `router::on-*` observers. No filters in v1.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EventBindingConfig {}

/// A binding's config or an observer's args; null means `{}`.
fn parse_config(name: &str, raw: Value) -> Result<EventBindingConfig, String> {
    let raw = if raw.is_null() {
        Value::Object(serde_json::Map::new())
    } else {
        raw
    };
    serde_json::from_value::<EventBindingConfig>(raw)
        .map_err(|e| format!("invalid {name} config: {e}"))
}

/// One observer of a `router::on-*` function, as the emitter sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
#[async_trait]
pub trait EventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// delivery: never blocks the caller that produced the event.
    async fn deliver(&self, payload: Value);
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            match obs.call::<Value>(payload).await {
                Ok(_) => {}
                // The observer's own filters declined this event.
                Err(Error::Remote { code, .. }) if code == "FILTERS_NOT_MATCHED" => {}
                Err(e) => {
                    tracing::warn!(
                        observer = %obs.id(),
                        function_id = %obs.function_id(),
                        error = %e,
                        "router observer delivery failed"
                    );
                }
            }
        });
    }
}

/// Thread-safe subscriber registry for one trigger type: its bindings keyed
/// by trigger-instance id and the observers of its `router::on-*` function
/// keyed by observer id, kept apart.
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, TriggerConfig>>>,
    observers: Arc<Mutex<HashMap<String, Arc<dyn EventObserver>>>>,
}

impl SubscriberSet {
    fn insert(&self, config: TriggerConfig) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.insert(config.id.clone(), config);
    }

    fn remove(&self, id: &str) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(id);
    }

    /// Snapshot so the mutex isn't held across awaits.
    pub fn function_ids(&self) -> Vec<String> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.values().map(|c| c.function_id.clone()).collect()
    }

    /// File an observer of the trigger type's `router::on-*` function.
    pub fn insert_observer(&self, observer: Arc<dyn EventObserver>) {
        let id = observer.id().to_string();
        self.observers_lock().insert(id, observer);
    }

    pub fn remove_observer(&self, id: &str) {
        self.observers_lock().remove(id);
    }

    /// Snapshot of the observers, same rule as [`Self::function_ids`].
    pub fn observer_snapshot(&self) -> Vec<Arc<dyn EventObserver>> {
        self.observers_lock().values().cloned().collect()
    }

    fn observers_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Arc<dyn EventObserver>>> {
        self.observers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

struct RouterEventTriggerHandler {
    name: String,
    subscribers: SubscriberSet,
}

impl RouterEventTriggerHandler {
    fn new(name: &str, subscribers: SubscriberSet) -> Self {
        Self {
            name: name.into(),
            subscribers,
        }
    }
}

#[async_trait]
impl TriggerHandler for RouterEventTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        parse_config(&self.name, config.config.clone()).map_err(Error::Handler)?;
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            "trigger subscription unregistered"
        );
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

/// Files each observer of a `router::on-*` function in the subscriber set its
/// trigger type uses, and takes it out again when the engine removes it.
struct RouterObserveHandler {
    name: &'static str,
    subscribers: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for RouterObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        parse_config(self.name, obs.args_value().clone())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        self.subscribers.insert_observer(Arc::new(obs.clone()));
        tracing::info!(
            trigger_type = %self.name,
            observer = %obs.id(),
            function_id = %obs.function_id(),
            "observer added"
        );
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.subscribers.remove_observer(obs.id());
        tracing::info!(
            trigger_type = %self.name,
            observer = %obs.id(),
            "observer removed"
        );
    }
}

/// The error every `router::on-*` handler returns: events come only from this
/// worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the llm-router worker emits its events"
        ),
        stacktrace: None,
    }
}

/// Registers an observable `router::on-*` function, owner-delivered, with
/// args [`EventBindingConfig`] (the trigger config) and hooks filing
/// observers in `subscribers`. On an engine without observe it registers as a
/// plain function and its hooks never run.
fn register_observable<E>(
    iii: &IIIClient,
    function_id: &'static str,
    trigger_type: &'static str,
    description: &str,
    subscribers: SubscriberSet,
) where
    E: DeserializeOwned + JsonSchema + Send + 'static,
{
    iii.register_function(
        function_id,
        RegisterFunction::new(move |_event: E| -> Result<(), Error> {
            Err(refuse_invocation(function_id))
        })
        .description(description)
        .observe(
            Observe::owner()
                .args_format::<EventBindingConfig>()
                .handler(RouterObserveHandler {
                    name: trigger_type,
                    subscribers,
                }),
        ),
    );
    tracing::info!(function_id, "registered observable function");
}

/// Fan-out handle: register once at boot, clone into handlers that emit.
pub struct RouterEvents {
    iii: IIIClient,
    ready: SubscriberSet,
    models_changed: SubscriberSet,
    provider_changed: SubscriberSet,
}

impl RouterEvents {
    /// Register the three custom trigger types and the observable function
    /// next to each one. Must run before emitting `router::ready` so the
    /// engine can replay existing bindings and observers first.
    pub fn register(iii: &IIIClient) -> Arc<Self> {
        let ready = SubscriberSet::default();
        let models_changed = SubscriberSet::default();
        let provider_changed = SubscriberSet::default();

        // The trigger types stay for consumers that bind them, on 0.24 and
        // 0.25 engines alike (register_trigger_type is deprecated in 0.25).
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                READY,
                READY_DESC,
                RouterEventTriggerHandler::new(READY, ready.clone()),
            )
            .trigger_request_format::<EventBindingConfig>(),
        );
        tracing::info!(trigger_type = READY, "registered trigger type");
        register_observable::<RouterReadyEvent>(iii, ON_READY, READY, ON_READY_DESC, ready.clone());

        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                MODELS_CHANGED,
                MODELS_CHANGED_DESC,
                RouterEventTriggerHandler::new(MODELS_CHANGED, models_changed.clone()),
            )
            .trigger_request_format::<EventBindingConfig>(),
        );
        tracing::info!(trigger_type = MODELS_CHANGED, "registered trigger type");
        register_observable::<ModelsChangedPayload>(
            iii,
            ON_MODELS_CHANGED,
            MODELS_CHANGED,
            ON_MODELS_CHANGED_DESC,
            models_changed.clone(),
        );

        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                PROVIDER_CHANGED,
                PROVIDER_CHANGED_DESC,
                RouterEventTriggerHandler::new(PROVIDER_CHANGED, provider_changed.clone()),
            )
            .trigger_request_format::<EventBindingConfig>(),
        );
        tracing::info!(trigger_type = PROVIDER_CHANGED, "registered trigger type");
        register_observable::<ProviderChangedPayload>(
            iii,
            ON_PROVIDER_CHANGED,
            PROVIDER_CHANGED,
            ON_PROVIDER_CHANGED_DESC,
            provider_changed.clone(),
        );

        Arc::new(Self {
            iii: iii.clone(),
            ready,
            models_changed,
            provider_changed,
        })
    }

    fn subscribers_for(&self, trigger_type: &str) -> Option<&SubscriberSet> {
        match trigger_type {
            READY => Some(&self.ready),
            MODELS_CHANGED => Some(&self.models_changed),
            PROVIDER_CHANGED => Some(&self.provider_changed),
            _ => None,
        }
    }

    /// Fire `payload` to every subscriber bound to `trigger_type` and every
    /// observer of its `router::on-*` function.
    pub async fn emit(&self, trigger_type: &str, payload: Value) {
        let Some(set) = self.subscribers_for(trigger_type) else {
            tracing::warn!(trigger_type, "unknown router event type");
            return;
        };
        let targets = set.function_ids();
        for function_id in targets {
            let res = self
                .iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(
                    trigger_type,
                    function_id = %function_id,
                    error = %e,
                    "router event fan-out failed"
                );
            }
        }
        for observer in set.observer_snapshot() {
            observer.deliver(payload.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscriber_set_insert_remove_and_snapshot() {
        let set = SubscriberSet::default();
        assert!(set.function_ids().is_empty());
        set.insert(TriggerConfig {
            id: "sub-1".into(),
            function_id: "probe::on_ready".into(),
            config: Value::Object(serde_json::Map::new()),
            metadata: None,
            namespace: None,
        });
        set.insert(TriggerConfig {
            id: "sub-2".into(),
            function_id: "other::recv".into(),
            config: Value::Object(serde_json::Map::new()),
            metadata: None,
            namespace: None,
        });
        let mut fns = set.function_ids();
        fns.sort();
        assert_eq!(
            fns,
            vec!["other::recv".to_string(), "probe::on_ready".to_string()]
        );
        set.remove("sub-1");
        assert_eq!(set.function_ids(), vec!["other::recv".to_string()]);
    }

    /// An observer of a `router::on-*` function that records what it is handed.
    struct Recording {
        id: String,
        received: Mutex<Vec<Value>>,
    }

    impl Recording {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                received: Mutex::new(Vec::new()),
            })
        }

        fn received(&self) -> Vec<Value> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EventObserver for Recording {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) {
            self.received.lock().unwrap().push(payload);
        }
    }

    fn binding(id: &str, function_id: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: function_id.into(),
            config: Value::Object(serde_json::Map::new()),
            metadata: None,
            namespace: None,
        }
    }

    /// The fan-out handle as `register` builds it, on a client that never
    /// connects (a `Void` trigger to it returns at once).
    fn events() -> RouterEvents {
        RouterEvents {
            iii: IIIClient::new("ws://127.0.0.1:1"),
            ready: SubscriberSet::default(),
            models_changed: SubscriberSet::default(),
            provider_changed: SubscriberSet::default(),
        }
    }

    #[test]
    fn on_functions_refuse_direct_invocation() {
        assert_eq!(ON_READY, "router::on-ready");
        assert_eq!(ON_MODELS_CHANGED, "router::on-models-changed");
        assert_eq!(ON_PROVIDER_CHANGED, "router::on-provider-changed");
        for function_id in [ON_READY, ON_MODELS_CHANGED, ON_PROVIDER_CHANGED] {
            let error = refuse_invocation(function_id);
            assert!(
                matches!(&error, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{error:?}"
            );
        }
    }

    #[test]
    fn observer_args_are_the_trigger_config() {
        assert!(parse_config(READY, Value::Null).is_ok());
        assert!(parse_config(READY, serde_json::json!({})).is_ok());
        let refused = parse_config(MODELS_CHANGED, serde_json::json!({ "provider": "x" }));
        assert!(
            refused.as_ref().is_err_and(|e| e.contains(MODELS_CHANGED)),
            "{refused:?}"
        );
        assert!(parse_config(PROVIDER_CHANGED, serde_json::json!([1])).is_err());
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        let events = events();
        events
            .models_changed
            .insert(binding("b-1", "probe::on_models"));
        let observer = Recording::new("o-models");
        let ready_observer = Recording::new("o-ready");
        events.models_changed.insert_observer(observer.clone());
        events.ready.insert_observer(ready_observer.clone());

        // The binding this event reaches, and the observer next to it.
        assert_eq!(
            events.models_changed.function_ids(),
            vec!["probe::on_models".to_string()]
        );
        let payload = serde_json::json!({ "provider": "openai", "count": 3 });
        events.emit(MODELS_CHANGED, payload.clone()).await;
        assert_eq!(observer.received(), vec![payload.clone()]);
        assert!(
            ready_observer.received().is_empty(),
            "another trigger type's observer gets nothing"
        );

        events.emit(READY, serde_json::json!({})).await;
        assert_eq!(ready_observer.received(), vec![serde_json::json!({})]);
        assert_eq!(observer.received().len(), 1);

        // An unknown type reaches nobody.
        events.emit("router::nope", payload.clone()).await;
        assert_eq!(observer.received().len(), 1);

        // Removing the observer stops delivery; the binding stays.
        events.models_changed.remove_observer("o-models");
        events.emit(MODELS_CHANGED, payload).await;
        assert_eq!(observer.received().len(), 1);
        assert_eq!(events.models_changed.function_ids().len(), 1);
    }

    #[test]
    fn observers_and_bindings_are_kept_apart() {
        let set = SubscriberSet::default();
        set.insert(binding("same", "probe::recv"));
        set.insert_observer(Recording::new("same"));
        // Re-adding an observer id replaces it.
        set.insert_observer(Recording::new("same"));
        assert_eq!(set.observer_snapshot().len(), 1);
        assert_eq!(set.function_ids().len(), 1);
        set.remove_observer("same");
        assert!(set.observer_snapshot().is_empty());
        assert_eq!(set.function_ids(), vec!["probe::recv".to_string()]);
        set.remove("same");
        assert!(set.function_ids().is_empty());
    }

    /// The schema iii-sdk publishes for a handler type (`SchemaSettings::draft07()`).
    fn published_schema<T: JsonSchema>() -> Value {
        serde_json::to_value(
            schemars::r#gen::SchemaSettings::draft07()
                .into_generator()
                .into_root_schema_for::<T>(),
        )
        .unwrap()
    }

    /// Typed: a schema with a concrete `type`, never `AnyValue`, `true` or `{}`.
    fn assert_typed(function_id: &str, side: &str, schema: &Value, ty: &str) {
        assert!(schema.is_object(), "{function_id} {side}: {schema}");
        assert_ne!(
            schema["title"], "AnyValue",
            "{function_id} {side}: {schema}"
        );
        assert_eq!(schema["type"], ty, "{function_id} {side}: {schema}");
    }

    #[test]
    fn on_functions_publish_typed_request_and_response_schemas() {
        // Request: the event struct the refusing handler takes (as passed to
        // register_observable::<E>); response: the handler's `()`.
        let cases = [
            (ON_READY, published_schema::<RouterReadyEvent>(), vec![]),
            (
                ON_MODELS_CHANGED,
                published_schema::<ModelsChangedPayload>(),
                vec!["count", "provider"],
            ),
            (
                ON_PROVIDER_CHANGED,
                published_schema::<ProviderChangedPayload>(),
                vec!["op", "provider"],
            ),
        ];
        for (function_id, request, fields) in cases {
            assert_typed(function_id, "request", &request, "object");
            let mut keys: Vec<&str> = request["properties"]
                .as_object()
                .map(|p| p.keys().map(String::as_str).collect())
                .unwrap_or_default();
            keys.sort_unstable();
            assert_eq!(keys, fields, "{function_id} request properties");
            assert_typed(function_id, "response", &published_schema::<()>(), "null");
        }
    }
}
