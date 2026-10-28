//! The `eval::completed` trigger type and the observable function next to it
//! (iii 0.25), `eval::on-completed`.
//!
//! Both share one [`SubscriberSet`]: the trigger type files bindings, the
//! function's [`ObserveHandler`] files observers, and
//! [`EvalEvents::emit_completed`] fans each terminal status out to both under
//! the same `evaluation_id` filter. Observers get it in-process with
//! `obs.call`; the function's own handler refuses every invocation
//! (`FORBIDDEN`), so no caller can make this worker deliver a forged event.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{observe_codes, TriggerRequest};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType, TriggerAction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::contract::EvalStatusV1;

pub const COMPLETED: &str = "eval::completed";

/// The observable function next to [`COMPLETED`].
pub const ON_COMPLETED: &str = "eval::on-completed";
pub const ON_COMPLETED_DESCRIPTION: &str =
    "Fires when a session-monitor analysis reaches a terminal status (completed, failed or \
     cancelled). Observe it with args { evaluation_id? } (the eval::completed binding config); \
     every matching observer receives { evaluation_id, status, timestamp }. Completion \
     describes the monitor, not a validated improvement. Only the eval worker fires it: \
     invoking it directly is refused.";

/// Config of an `eval::completed` binding, and the args of an
/// `eval::on-completed` observer (`args_format`).
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CompletedBindingConfigV1 {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evaluation_id: Option<String>,
}

/// What every binding and observer receives when an analysis ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CompletedEventV1 {
    pub evaluation_id: String,
    pub status: EvalStatusV1,
    /// Epoch millis at emit time.
    pub timestamp: i64,
}

fn parse_config(raw: &Value) -> Result<CompletedBindingConfigV1, String> {
    let raw = if raw.is_null() {
        json!({})
    } else {
        raw.clone()
    };
    serde_json::from_value(raw).map_err(|error| format!("invalid {COMPLETED} config: {error}"))
}

fn matches(filter: Option<&str>, evaluation_id: &str) -> bool {
    filter.is_none_or(|filter| filter == evaluation_id)
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    evaluation_id: Option<String>,
    metadata: Option<Value>,
}

/// One observer of `eval::on-completed` as a sink of [`SubscriberSet`]: the
/// SDK [`Observer`] in production, a recorder in tests (no engine needed).
#[async_trait]
trait CompletedObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    /// The observer's args, parsed as an `eval::completed` binding config.
    fn args_value(&self) -> &Value;
    /// Hands one event to the observer's target (`obs.call`).
    async fn deliver(&self, payload: Value) -> Result<(), Error>;
}

#[async_trait]
impl CompletedObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn args_value(&self) -> &Value {
        Observer::args_value(self)
    }

    async fn deliver(&self, payload: Value) -> Result<(), Error> {
        self.call::<Value>(payload).await.map(|_| ())
    }
}

#[derive(Clone)]
struct ObserverBinding {
    observer: Arc<dyn CompletedObserver>,
    evaluation_id: Option<String>,
}

#[derive(Clone, Default)]
struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverBinding>>>,
}

impl SubscriberSet {
    fn insert(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = parse_config(&config.config)?;
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                config.id,
                Binding {
                    function_id: config.function_id,
                    evaluation_id: filter.evaluation_id,
                    metadata: config.metadata,
                },
            );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Files an observer under its id; its args are parsed exactly as a
    /// binding config (`null` = every analysis, unknown fields rejected).
    fn insert_observer(&self, observer: Arc<dyn CompletedObserver>) -> Result<(), String> {
        let filter = parse_config(observer.args_value())?;
        self.observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                observer.id().to_string(),
                ObserverBinding {
                    observer,
                    evaluation_id: filter.evaluation_id,
                },
            );
        Ok(())
    }

    fn remove_observer(&self, id: &str) {
        self.observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(id);
    }

    fn observer_snapshot(&self) -> Vec<ObserverBinding> {
        self.observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .values()
            .cloned()
            .collect()
    }

    /// Hands `payload` to every observer whose filter selects
    /// `evaluation_id`, the filter a binding is matched with. Each call runs
    /// on its own task, in the current invocation's context, so a slow target
    /// holds up neither the caller nor the other sinks.
    fn notify_observers(
        &self,
        payload: &Value,
        evaluation_id: &str,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        let ctx = iii_sdk::current_invocation();
        let mut deliveries = Vec::new();
        for entry in self.observer_snapshot() {
            if !matches(entry.evaluation_id.as_deref(), evaluation_id) {
                continue;
            }
            let payload = payload.clone();
            let evaluation_id = evaluation_id.to_string();
            deliveries.push(tokio::spawn(iii_sdk::scope(ctx.clone(), async move {
                match entry.observer.deliver(payload).await {
                    Ok(()) => {}
                    Err(Error::Remote { code, .. })
                        if code == observe_codes::FILTERS_NOT_MATCHED => {}
                    Err(error) => tracing::warn!(
                        %evaluation_id,
                        observer = %entry.observer.id(),
                        %error,
                        "{ON_COMPLETED} delivery failed"
                    ),
                }
            })));
        }
        deliveries
    }
}

struct CompletedTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for CompletedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.insert(config).map_err(Error::Handler)
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// Files each observer of `eval::on-completed` in the set the trigger type
/// fans out from, and takes it out when the engine removes it.
struct CompletedObserveHandler {
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for CompletedObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format; parsed again here so a
        // config the trigger type would refuse is refused the same way.
        self.set
            .insert_observer(Arc::new(obs.clone()))
            .map_err(|error| ObserveRejection::new(observe_codes::INVALID_ARGS, error))?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), "{ON_COMPLETED} observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
        tracing::info!(observer = %obs.id(), "{ON_COMPLETED} observer removed");
    }
}

/// The error every direct invocation of `eval::on-completed` gets: events
/// only come from this worker's own fan-out.
pub fn forbidden() -> Error {
    Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_COMPLETED} is an event source: observe it \
             (iii.observe({{ function_id: '{ON_COMPLETED}', args: {{ evaluation_id? }} }}, \
             target)); only the eval worker fires its events"
        ),
        stacktrace: None,
    }
}

/// The handler of `eval::on-completed`: refuses any input with
/// [`forbidden`], before and regardless of its shape.
pub fn refuse_invocation(_input: Value) -> Result<(), Error> {
    Err(forbidden())
}

/// JSON Schema of `T`, generated as iii-sdk does at registration.
fn schema_value<T: JsonSchema>() -> Value {
    let schema = schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(schema).unwrap_or(Value::Null)
}

#[derive(Clone)]
pub struct EvalEvents {
    iii: Arc<IIIClient>,
    completed: SubscriberSet,
}

impl EvalEvents {
    /// Registers the `eval::completed` trigger type and the owner-delivered
    /// `eval::on-completed` next to it, both over one subscriber set. On an
    /// engine without observe the function registers as a plain function and
    /// its hooks never run.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let completed = SubscriberSet::default();
        // Kept until the trigger type is retired; old consumers bind it on
        // 0.24 and 0.25 engines.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                COMPLETED,
                "A session-monitor analysis reached a terminal status (completed, failed or cancelled). Completion describes the monitor, not a validated improvement.",
                CompletedTriggerHandler {
                    set: completed.clone(),
                },
            )
            .trigger_request_format::<CompletedBindingConfigV1>(),
        );
        iii.register_function(
            ON_COMPLETED,
            RegisterFunction::new(refuse_invocation)
                .description(ON_COMPLETED_DESCRIPTION)
                .request_format(schema_value::<CompletedEventV1>())
                .observe(
                    Observe::owner()
                        .args_format::<CompletedBindingConfigV1>()
                        .handler(CompletedObserveHandler {
                            set: completed.clone(),
                        }),
                ),
        );
        Self {
            iii: iii.clone(),
            completed,
        }
    }

    /// Events that reach no subscriber, for tools that share the engine with
    /// a running `eval` and must not take over its trigger type or its
    /// observable function.
    pub fn detached(iii: &Arc<IIIClient>) -> Self {
        Self {
            iii: iii.clone(),
            completed: SubscriberSet::default(),
        }
    }

    pub async fn emit_completed(&self, evaluation_id: &str, status: EvalStatusV1) {
        let payload = serde_json::to_value(CompletedEventV1 {
            evaluation_id: evaluation_id.to_string(),
            status,
            timestamp: crate::ids::now_ms(),
        })
        .unwrap_or(Value::Null);
        for binding in self.completed.snapshot() {
            if !matches(binding.evaluation_id.as_deref(), evaluation_id) {
                continue;
            }
            let request = TriggerRequest {
                function_id: binding.function_id,
                payload: payload.clone(),
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let result = match binding.metadata {
                Some(metadata) => self.iii.trigger(request.metadata(metadata)).await,
                None => self.iii.trigger(request).await,
            };
            if let Err(error) = result {
                tracing::warn!(%evaluation_id, %error, "eval::completed delivery failed");
            }
        }
        // Observers get the same payload under the same filter, through
        // `obs.call` (the SDK applies their own filters too).
        let _ = self.completed.notify_observers(&payload, evaluation_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Stands in for the SDK observer: records what `obs.call` would carry.
    struct Recorder {
        id: String,
        args: Value,
        seen: Mutex<Vec<Value>>,
    }

    impl Recorder {
        fn new(id: &str, args: Value) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                args,
                seen: Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl CompletedObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn args_value(&self) -> &Value {
            &self.args
        }

        async fn deliver(&self, payload: Value) -> Result<(), Error> {
            self.seen.lock().unwrap().push(payload);
            Ok(())
        }
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: "app::on-eval-completed".into(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    fn payload(evaluation_id: &str) -> Value {
        serde_json::to_value(CompletedEventV1 {
            evaluation_id: evaluation_id.into(),
            status: EvalStatusV1::Completed,
            timestamp: 1_700_000_000_000,
        })
        .unwrap()
    }

    async fn notify(set: &SubscriberSet, payload: &Value, evaluation_id: &str) {
        for delivery in set.notify_observers(payload, evaluation_id) {
            delivery.await.unwrap();
        }
    }

    #[test]
    fn on_completed_refuses_direct_invocation() {
        match refuse_invocation(json!({ "evaluation_id": "ev-1", "status": "completed" })) {
            Err(Error::Remote { code, message, .. }) => {
                assert_eq!(code, observe_codes::FORBIDDEN);
                assert!(message.contains(ON_COMPLETED));
            }
            other => panic!("expected FORBIDDEN, got {other:?}"),
        }
    }

    #[test]
    fn completed_event_keeps_the_binding_payload_shape() {
        assert_eq!(
            payload("ev-1"),
            json!({ "evaluation_id": "ev-1", "status": "completed", "timestamp": 1_700_000_000_000_i64 })
        );
    }

    #[tokio::test]
    async fn observer_receives_the_event_a_binding_receives() {
        let set = SubscriberSet::default();
        let config = json!({ "evaluation_id": "ev-1" });
        set.insert(binding("b-1", config.clone())).unwrap();
        let scoped = Recorder::new("o-scoped", config);
        let every = Recorder::new("o-every", Value::Null);
        set.insert_observer(scoped.clone()).unwrap();
        set.insert_observer(every.clone()).unwrap();

        let event = payload("ev-1");
        notify(&set, &event, "ev-1").await;
        notify(&set, &payload("ev-2"), "ev-2").await;

        // The binding with the same config is selected for the same
        // analyses; emit_completed triggers it with this very payload.
        let bindings = set.snapshot();
        assert_eq!(bindings.len(), 1);
        assert!(matches(bindings[0].evaluation_id.as_deref(), "ev-1"));
        assert!(!matches(bindings[0].evaluation_id.as_deref(), "ev-2"));

        assert_eq!(scoped.seen(), vec![event.clone()]);
        assert_eq!(every.seen(), vec![event, payload("ev-2")]);
    }

    #[tokio::test]
    async fn removed_observer_gets_nothing_and_bindings_stay() {
        let set = SubscriberSet::default();
        set.insert(binding("b-1", Value::Null)).unwrap();
        let observer = Recorder::new("o-1", json!({}));
        set.insert_observer(observer.clone()).unwrap();
        set.remove_observer("o-1");

        notify(&set, &payload("ev-1"), "ev-1").await;
        assert!(observer.seen().is_empty());
        assert!(set.observer_snapshot().is_empty());
        assert_eq!(set.snapshot().len(), 1);
    }

    #[test]
    fn observer_args_are_checked_like_a_binding_config() {
        let set = SubscriberSet::default();
        for bad in [json!({ "unknown": true }), json!({ "evaluation_id": 7 })] {
            assert!(set
                .insert_observer(Recorder::new("o", bad.clone()))
                .is_err());
            assert!(set.insert(binding("b", bad)).is_err());
        }
        assert!(set.observer_snapshot().is_empty());
        assert!(set.snapshot().is_empty());
    }

    #[test]
    fn re_adding_an_observer_replaces_it() {
        let set = SubscriberSet::default();
        set.insert_observer(Recorder::new("o-1", json!({ "evaluation_id": "a" })))
            .unwrap();
        set.insert_observer(Recorder::new("o-1", json!({ "evaluation_id": "b" })))
            .unwrap();
        let observers = set.observer_snapshot();
        assert_eq!(observers.len(), 1);
        assert_eq!(observers[0].evaluation_id.as_deref(), Some("b"));
    }
}
