//! Durable subscriber trigger handling.
//!
//! The handler itself no longer owns any consumer loop or store access --
//! `register_trigger`/`unregister_trigger` translate a `TriggerConfig` into
//! [`crate::adapter::QueueAdapter::subscribe`]/`unsubscribe` calls against the
//! (possibly hot-swapped) [`SwappableAdapter`]. All delivery, retry, and DLQ
//! behavior lives in the adapter (e.g. [`crate::adapters::builtin::BuiltinAdapter`]).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

use crate::adapter::{QueueAdapter, SwappableAdapter};
use crate::subscriber_config::SubscriberQueueConfig;

/// Queue-delivered functions may run for many minutes (e.g. streaming an LLM
/// response). The iii SDK defaults an omitted timeout to 30 seconds, which
/// lets the adapter retry an in-flight delivery while the original invocation
/// is still running. 30 minutes covers the longest intended job budget.
const FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS: u64 = 30 * 60 * 1_000;

/// How often [`IiiInvoker::connection_lost_since`] samples the engine epoch
/// while an invocation is in flight, and how long one sample may take.
const ENGINE_EPOCH_PROBE_INTERVAL_MS: u64 = 1_000;
const ENGINE_EPOCH_PROBE_TIMEOUT_MS: u64 = 3_000;

/// The engine's boot identity: the earliest `connected_at_ms` among its
/// in-process (`runtime == "engine"`) workers, which attach once at engine
/// startup. A restarted engine reports a new epoch — the signal that every
/// invocation in flight across the restart lost its result routing. `None`
/// when the engine cannot be reached (outage in progress) or the response
/// shape is unrecognized.
async fn engine_epoch_ms(iii: &IIIClient) -> Option<u64> {
    let response = iii
        .trigger(TriggerRequest {
            function_id: "engine::workers::list".to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(ENGINE_EPOCH_PROBE_TIMEOUT_MS),
        })
        .await
        .ok()?;
    response
        .get("workers")?
        .as_array()?
        .iter()
        .filter(|worker| worker.get("runtime").and_then(Value::as_str) == Some("engine"))
        .filter_map(|worker| worker.get("connected_at_ms").and_then(Value::as_u64))
        .min()
}

#[async_trait]
pub trait Invoker: Send + Sync + 'static {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String>;

    async fn call_with_timeout(
        &self,
        function_id: &str,
        payload: Value,
        _timeout_ms: u64,
    ) -> Result<Option<Value>, String> {
        self.call(function_id, payload).await
    }

    /// Dispatch one SUBSCRIPTION DELIVERY: the payload plus the trigger's
    /// stored metadata, which the target may need to know which binding this
    /// is (for a harness-managed binding it is the `__binding` pointer the
    /// delivery hop resolves — without it every delivery is an unresolvable
    /// fire, dropped on arrival; discovery run 4 lost all 18 messages to
    /// exactly that). Condition checks and infrastructure calls stay on
    /// [`Self::call`]. The default ignores metadata so bare test invokers
    /// keep working; the iii-backed invoker overrides it.
    async fn call_delivery(
        &self,
        function_id: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let _ = metadata;
        self.call(function_id, payload).await
    }

    /// Whether the target is currently registered with the engine.
    ///
    /// Adapters used in unit tests and embedded deployments can keep the
    /// optimistic default. The real iii-backed invoker overrides this so a
    /// durable job restored before its worker boots is held instead of
    /// burning delivery retries on a transient `FUNCTION_NOT_FOUND` error.
    async fn function_available(&self, _function_id: &str) -> Result<bool, String> {
        Ok(true)
    }

    /// Capture the engine epoch immediately before a restart-sensitive
    /// invocation. `None` disables restart watching for this invocation.
    async fn connection_epoch(&self) -> Result<Option<u64>, String> {
        Ok(None)
    }

    /// Resolve when the engine moves away from the captured epoch.
    async fn connection_lost_since(&self, _baseline: u64) {
        std::future::pending::<()>().await
    }
}

#[derive(Clone)]
pub struct IiiInvoker {
    iii: Arc<IIIClient>,
}

impl IiiInvoker {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl Invoker for IiiInvoker {
    async fn call(&self, function_id: &str, payload: Value) -> Result<Option<Value>, String> {
        self.call_delivery(function_id, payload, None).await
    }

    async fn call_with_timeout(
        &self,
        function_id: &str,
        payload: Value,
        timeout_ms: u64,
    ) -> Result<Option<Value>, String> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(timeout_ms),
            })
            .await
            .map(Some)
            .map_err(|e| e.to_string())
    }

    async fn call_delivery(
        &self,
        function_id: &str,
        payload: Value,
        metadata: Option<Value>,
    ) -> Result<Option<Value>, String> {
        let request = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(FUNCTION_QUEUE_INVOCATION_TIMEOUT_MS),
        };
        match metadata {
            Some(m) => self.iii.trigger(request.metadata(m)).await,
            None => self.iii.trigger(request).await,
        }
        .map(Some)
        .map_err(|e| e.to_string())
    }

    async fn function_available(&self, function_id: &str) -> Result<bool, String> {
        match self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::info".to_string(),
                payload: json!({ "function_id": function_id }),
                action: None,
                timeout_ms: Some(5_000),
            })
            .await
        {
            Ok(value) => Ok(!value.is_null() && value.get("error").is_none()),
            Err(error) => {
                let message = error.to_string();
                if message.to_ascii_uppercase().contains("NOT_FOUND") {
                    Ok(false)
                } else {
                    Err(message)
                }
            }
        }
    }

    async fn connection_epoch(&self) -> Result<Option<u64>, String> {
        Ok(engine_epoch_ms(&self.iii).await)
    }

    async fn connection_lost_since(&self, baseline: u64) {
        // Neither the SDK connection state nor plain liveness probes can see
        // a fast restart: the reconnect loop reports `Connected` through its
        // silent 2s retry sleep, and outbound messages buffered during the
        // outage are answered by the NEW engine as if nothing happened
        // (both verified against a SIGKILLed-and-respawned engine). The
        // engine's boot epoch is the reliable signal — a buffered probe
        // answered by a restarted engine reveals the changed epoch.
        loop {
            tokio::time::sleep(Duration::from_millis(ENGINE_EPOCH_PROBE_INTERVAL_MS)).await;
            // An unreadable epoch (outage in progress) never trips by
            // itself: nothing can progress until the engine is back, and
            // the first successful sample afterwards decides.
            if let Some(epoch) = engine_epoch_ms(&self.iii).await {
                if epoch != baseline {
                    return;
                }
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SubscriberSpec {
    /// Queue/topic name to consume.
    #[serde(alias = "topic")]
    pub queue: String,
    #[serde(default, alias = "maxRetries", skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    #[serde(
        default,
        alias = "backoffDelayMs",
        skip_serializing_if = "Option::is_none"
    )]
    pub backoff_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition_function_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub queue_config: Option<SubscriberQueueConfig>,
}

#[derive(Debug, Clone)]
pub struct RegisteredSubscriber {
    pub trigger_id: String,
    pub function_id: String,
    /// The trigger's stored metadata, delivered verbatim with every message.
    /// For a harness-managed binding this carries `{"__binding": <id>}` — the
    /// pointer the delivery hop resolves; dropping it makes every delivery an
    /// unresolvable fire.
    pub metadata: Option<Value>,
    pub spec: SubscriberSpec,
}

impl RegisteredSubscriber {
    /// The durable identity the adapter keys this subscription's queue by.
    ///
    /// It must survive re-registration, or the replacement subscriber gets a
    /// fresh empty queue and the old one sits bound to the exchange collecting
    /// a copy of every publish forever (the SDK mints a new trigger id per
    /// `register_trigger` call, so `trigger_id` itself cannot be the key).
    /// A harness binding's `__binding` id is that identity; a plain SDK
    /// subscriber falls back to its target function id — the pre-binding queue
    /// naming, which also keeps existing deployments attached to their
    /// backlogs across the upgrade. Distinct same-function subscribers without
    /// a binding id therefore share one subscription (first registration's
    /// config wins), exactly as they shared one function queue before.
    fn subscription_key(&self) -> &str {
        self.metadata
            .as_ref()
            .and_then(|m| m.get("__binding"))
            .and_then(Value::as_str)
            .unwrap_or(&self.function_id)
    }
}

/// Merge the spec's legacy top-level `max_retries`/`backoff_ms` into its
/// `queue_config`. Precedence: explicit top-level field wins, then whatever
/// `queue_config` already carries, then the adapter's own defaults (3
/// retries / 1000ms backoff) when both are absent -- the adapter applies
/// those defaults itself when the returned fields are `None`.
fn merged_queue_config(spec: &SubscriberSpec) -> SubscriberQueueConfig {
    let mut config = spec.queue_config.clone().unwrap_or_default();
    if let Some(max_retries) = spec.max_retries {
        config.max_retries = Some(max_retries);
    }
    if let Some(backoff_ms) = spec.backoff_ms {
        config.backoff_delay_ms = Some(backoff_ms);
    }
    config
}

#[derive(Clone)]
pub struct QueueTriggerHandler {
    adapter: Arc<SwappableAdapter>,
    registrations: Arc<Mutex<HashMap<String, RegisteredSubscriber>>>,
}

impl QueueTriggerHandler {
    pub fn new(adapter: Arc<SwappableAdapter>) -> Self {
        Self {
            adapter,
            registrations: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Snapshot of every currently registered subscriber -- consumed by the
    /// configuration hot-swap path to re-subscribe against the newly built
    /// adapter after a transport swap.
    pub async fn registrations(&self) -> Vec<RegisteredSubscriber> {
        self.registrations.lock().await.values().cloned().collect()
    }

    /// Clears every tracked registration and tears down the current
    /// adapter's active consumers.
    pub async fn shutdown(&self) {
        self.registrations.lock().await.clear();
        self.adapter.shutdown().await;
    }

    /// Re-subscribe every currently tracked registration directly against
    /// the adapter, bypassing [`Self::register_subscriber`]'s duplicate-id
    /// check and leaving the registrations map untouched. Used by the
    /// configuration hot-swap path (`crate::configuration::swap_adapter`)
    /// *after* [`SwappableAdapter::replace`] has pointed `self.adapter` at
    /// the newly built transport, so these `subscribe` calls attach
    /// consumers to the new adapter rather than the one being retired.
    pub async fn resubscribe_all(&self) {
        let mut seen = HashSet::new();
        for registration in self.registrations().await {
            let key = registration.subscription_key().to_string();
            if !seen.insert((registration.spec.queue.clone(), key.clone())) {
                continue; // registrations sharing a key share one subscription
            }
            let queue_config = merged_queue_config(&registration.spec);
            self.adapter
                .subscribe(
                    &registration.spec.queue,
                    &key,
                    &registration.function_id,
                    registration.metadata.clone(),
                    registration.spec.condition_function_id.clone(),
                    Some(queue_config),
                )
                .await;
        }
    }

    pub async fn register_subscriber(
        &self,
        registration: RegisteredSubscriber,
    ) -> Result<(), String> {
        if registration.spec.queue.trim().is_empty() {
            return Err("queue is required for durable:subscriber trigger".to_string());
        }

        let mut registrations = self.registrations.lock().await;
        if registrations.contains_key(&registration.trigger_id) {
            return Err(format!(
                "durable:subscriber trigger '{}' is already registered",
                registration.trigger_id
            ));
        }

        // Registrations sharing a subscription key (e.g. a re-registered SDK
        // subscriber whose previous trigger was never unregistered) share the
        // one live subscription instead of stacking duplicate consumers that
        // would each deliver every message again.
        let key = registration.subscription_key().to_string();
        let already_live = registrations
            .values()
            .any(|r| r.spec.queue == registration.spec.queue && r.subscription_key() == key);
        if !already_live {
            let queue_config = merged_queue_config(&registration.spec);
            self.adapter
                .subscribe(
                    &registration.spec.queue,
                    &key,
                    &registration.function_id,
                    registration.metadata.clone(),
                    registration.spec.condition_function_id.clone(),
                    Some(queue_config),
                )
                .await;
        }

        registrations.insert(registration.trigger_id.clone(), registration);
        Ok(())
    }

    /// Unregister by bare trigger id -- the queue/topic name is recovered
    /// from the stored registration, since an unregister `TriggerConfig`
    /// carries only the id.
    pub async fn unregister(&self, trigger_id: &str) {
        let mut registrations = self.registrations.lock().await;
        let Some(registration) = registrations.remove(trigger_id) else {
            return;
        };
        let key = registration.subscription_key();
        let still_shared = registrations
            .values()
            .any(|r| r.spec.queue == registration.spec.queue && r.subscription_key() == key);
        if !still_shared {
            self.adapter
                .unsubscribe(&registration.spec.queue, key)
                .await;
        }
    }
}

#[async_trait]
impl TriggerHandler for QueueTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let spec: SubscriberSpec = serde_json::from_value(config.config.clone())
            .map_err(|e| Error::Handler(format!("invalid durable:subscriber config: {e}")))?;
        self.register_subscriber(RegisteredSubscriber {
            trigger_id: config.id,
            function_id: config.function_id,
            metadata: config.metadata,
            spec,
        })
        .await
        .map_err(Error::Handler)
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.unregister(&config.id).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::TopicInfo;
    use crate::store::TopicStats;
    use serde_json::json;
    use std::sync::Mutex as StdMutex;

    #[derive(Debug, Clone, PartialEq)]
    enum Call {
        Subscribe {
            topic: String,
            id: String,
            function_id: String,
            metadata: Option<Value>,
            condition_function_id: Option<String>,
            queue_config: Option<SubscriberQueueConfig>,
        },
        Unsubscribe {
            topic: String,
            id: String,
        },
    }

    #[derive(Default)]
    struct MockAdapter {
        calls: StdMutex<Vec<Call>>,
    }

    impl MockAdapter {
        fn calls(&self) -> Vec<Call> {
            self.calls.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl QueueAdapter for MockAdapter {
        async fn enqueue(
            &self,
            _topic: &str,
            _data: Value,
            _traceparent: Option<String>,
            _baggage: Option<String>,
        ) {
        }

        async fn subscribe(
            &self,
            topic: &str,
            id: &str,
            function_id: &str,
            metadata: Option<Value>,
            condition_function_id: Option<String>,
            queue_config: Option<SubscriberQueueConfig>,
        ) {
            self.calls.lock().unwrap().push(Call::Subscribe {
                topic: topic.to_string(),
                id: id.to_string(),
                function_id: function_id.to_string(),
                metadata,
                condition_function_id,
                queue_config,
            });
        }

        async fn unsubscribe(&self, topic: &str, id: &str) {
            self.calls.lock().unwrap().push(Call::Unsubscribe {
                topic: topic.to_string(),
                id: id.to_string(),
            });
        }

        async fn redrive_dlq(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn redrive_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn discard_dlq_message(
            &self,
            _topic: &str,
            _message_id: &str,
        ) -> anyhow::Result<bool> {
            Ok(false)
        }

        async fn dlq_count(&self, _topic: &str) -> anyhow::Result<u64> {
            Ok(0)
        }

        async fn list_topics(&self) -> anyhow::Result<Vec<TopicInfo>> {
            Ok(vec![])
        }

        async fn topic_stats(&self, _topic: &str) -> anyhow::Result<TopicStats> {
            Ok(TopicStats::default())
        }

        async fn shutdown(&self) {}
    }

    fn trigger_config(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
        }
    }

    fn handler_with_mock() -> (QueueTriggerHandler, Arc<MockAdapter>) {
        let mock = Arc::new(MockAdapter::default());
        let adapter: Arc<dyn QueueAdapter> = mock.clone();
        let handler = QueueTriggerHandler::new(Arc::new(SwappableAdapter::new(adapter, "mock")));
        (handler, mock)
    }

    #[tokio::test]
    async fn register_calls_adapter_subscribe_with_merged_config() {
        let (handler, mock) = handler_with_mock();

        handler
            .register_trigger(trigger_config(
                "t1",
                "backend",
                json!({"queue": "demo", "max_retries": 2, "backoff_ms": 1}),
            ))
            .await
            .unwrap();

        assert_eq!(
            mock.calls(),
            vec![Call::Subscribe {
                topic: "demo".to_string(),
                // No binding id in the metadata: the subscription is keyed by
                // its stable function id, not the per-registration trigger id.
                id: "backend".to_string(),
                function_id: "backend".to_string(),
                metadata: None,
                condition_function_id: None,
                queue_config: Some(SubscriberQueueConfig {
                    max_retries: Some(2),
                    backoff_delay_ms: Some(1),
                    ..Default::default()
                }),
            }]
        );
    }

    /// The discovery-run-4 regression: the harness's `__binding` pointer
    /// arrives as trigger metadata and MUST reach the adapter — the delivery
    /// hop cannot resolve a fire without it.
    #[tokio::test]
    async fn register_passes_trigger_metadata_to_the_adapter() {
        let (handler, mock) = handler_with_mock();
        let mut config = trigger_config("t1", "harness::trigger::deliver", json!({"queue": "q"}));
        config.metadata = Some(json!({ "__binding": "sub_abc" }));
        handler.register_trigger(config).await.unwrap();

        let Call::Subscribe { id, metadata, .. } = &mock.calls()[0] else {
            panic!("expected a Subscribe call");
        };
        assert_eq!(metadata, &Some(json!({ "__binding": "sub_abc" })));
        // The binding id is the durable subscription identity — it, not the
        // ephemeral trigger id, keys the adapter's queue.
        assert_eq!(id, "sub_abc");

        // And a hot-swap resubscribe carries it to the NEW adapter too.
        let new_mock = Arc::new(MockAdapter::default());
        let new_adapter: Arc<dyn QueueAdapter> = new_mock.clone();
        handler.adapter.replace(new_adapter, "new-mock").await;
        handler.resubscribe_all().await;
        let Call::Subscribe { id, metadata, .. } = &new_mock.calls()[0] else {
            panic!("expected a Subscribe call");
        };
        assert_eq!(metadata, &Some(json!({ "__binding": "sub_abc" })));
        assert_eq!(id, "sub_abc");
    }

    /// The SDK mints a fresh trigger id per registration, so a restarted
    /// subscriber re-registers under a new id. Its durable identity (here the
    /// function-id fallback) must not change, or it would come back to a
    /// fresh empty queue while the old one keeps collecting fanout copies
    /// forever.
    #[tokio::test]
    async fn re_registration_under_a_new_trigger_id_keeps_the_subscription_key() {
        let (handler, mock) = handler_with_mock();
        handler
            .register_trigger(trigger_config(
                "uuid-1",
                "backend",
                json!({"queue": "demo"}),
            ))
            .await
            .unwrap();
        handler.unregister("uuid-1").await;
        handler
            .register_trigger(trigger_config(
                "uuid-2",
                "backend",
                json!({"queue": "demo"}),
            ))
            .await
            .unwrap();

        let ids: Vec<_> = mock
            .calls()
            .iter()
            .map(|call| match call {
                Call::Subscribe { id, .. } => format!("subscribe:{id}"),
                Call::Unsubscribe { id, .. } => format!("unsubscribe:{id}"),
            })
            .collect();
        assert_eq!(
            ids,
            vec![
                "subscribe:backend",
                "unsubscribe:backend",
                "subscribe:backend"
            ],
            "both registrations must attach to the same durable queue"
        );
    }

    /// A hot-swap resubscribe collapses registrations sharing a subscription
    /// key to ONE subscribe against the new adapter, mirroring
    /// `register_subscriber`'s dedup — otherwise the swap would stack
    /// duplicate consumers that each deliver every message again.
    #[tokio::test]
    async fn resubscribe_all_subscribes_shared_keys_once() {
        let (handler, _old_mock) = handler_with_mock();
        handler
            .register_trigger(trigger_config(
                "uuid-1",
                "backend",
                json!({"queue": "demo"}),
            ))
            .await
            .unwrap();
        handler
            .register_trigger(trigger_config(
                "uuid-2",
                "backend",
                json!({"queue": "demo"}),
            ))
            .await
            .unwrap();

        let new_mock = Arc::new(MockAdapter::default());
        let new_adapter: Arc<dyn QueueAdapter> = new_mock.clone();
        handler.adapter.replace(new_adapter, "new-mock").await;
        handler.resubscribe_all().await;

        assert_eq!(
            new_mock.calls().len(),
            1,
            "shared-key registrations must resubscribe once"
        );
        let Call::Subscribe { id, .. } = &new_mock.calls()[0] else {
            panic!("expected a Subscribe call");
        };
        assert_eq!(id, "backend");
    }

    /// Leftover duplicate registrations for the same function (e.g. persisted
    /// triggers re-delivered across app restarts) share one subscription
    /// instead of each delivering every message again.
    #[tokio::test]
    async fn same_function_registrations_share_one_live_subscription() {
        let (handler, mock) = handler_with_mock();
        handler
            .register_trigger(trigger_config(
                "uuid-1",
                "backend",
                json!({"queue": "demo"}),
            ))
            .await
            .unwrap();
        handler
            .register_trigger(trigger_config(
                "uuid-2",
                "backend",
                json!({"queue": "demo"}),
            ))
            .await
            .unwrap();
        assert_eq!(mock.calls().len(), 1, "one shared subscription");

        // The shared subscription stays live until the LAST registration goes.
        handler.unregister("uuid-1").await;
        assert_eq!(mock.calls().len(), 1, "still consumed by uuid-2");
        handler.unregister("uuid-2").await;
        assert_eq!(
            mock.calls().last().unwrap(),
            &Call::Unsubscribe {
                topic: "demo".to_string(),
                id: "backend".to_string(),
            }
        );
    }

    #[tokio::test]
    async fn register_passes_condition_function_id_through() {
        let (handler, mock) = handler_with_mock();

        handler
            .register_trigger(trigger_config(
                "t1",
                "backend",
                json!({"queue": "demo", "condition_function_id": "condition"}),
            ))
            .await
            .unwrap();

        let Call::Subscribe {
            condition_function_id,
            ..
        } = &mock.calls()[0]
        else {
            panic!("expected a Subscribe call");
        };
        assert_eq!(condition_function_id.as_deref(), Some("condition"));
    }

    #[tokio::test]
    async fn register_rejects_empty_queue() {
        let (handler, _mock) = handler_with_mock();
        let err = handler
            .register_trigger(trigger_config("t1", "backend", json!({"queue": ""})))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("queue"));
    }

    #[tokio::test]
    async fn duplicate_trigger_id_is_rejected() {
        let (handler, mock) = handler_with_mock();
        let cfg = trigger_config("t1", "backend", json!({"queue": "demo"}));
        handler.register_trigger(cfg.clone()).await.unwrap();
        let err = handler.register_trigger(cfg).await.unwrap_err();
        assert!(err.to_string().contains("already registered"));
        assert_eq!(mock.calls().len(), 1, "adapter.subscribe called only once");
    }

    #[tokio::test]
    async fn unregister_by_id_calls_adapter_unsubscribe() {
        let (handler, mock) = handler_with_mock();
        handler
            .register_trigger(trigger_config("t1", "backend", json!({"queue": "demo"})))
            .await
            .unwrap();

        handler
            .unregister_trigger(trigger_config("t1", "", Value::Null))
            .await
            .unwrap();

        assert_eq!(
            mock.calls().last().unwrap(),
            &Call::Unsubscribe {
                topic: "demo".to_string(),
                id: "backend".to_string(),
            }
        );
        assert!(handler.registrations().await.is_empty());
    }

    #[tokio::test]
    async fn resubscribe_all_attaches_every_registration_to_the_new_adapter() {
        let (handler, old_mock) = handler_with_mock();
        handler
            .register_trigger(trigger_config(
                "t1",
                "backend-1",
                json!({"queue": "demo-1", "max_retries": 2}),
            ))
            .await
            .unwrap();
        handler
            .register_trigger(trigger_config(
                "t2",
                "backend-2",
                json!({"queue": "demo-2"}),
            ))
            .await
            .unwrap();
        assert_eq!(old_mock.calls().len(), 2, "initial registration subscribes");

        // Simulate a hot-swap: the adapter behind the handler's
        // `SwappableAdapter` is replaced with a fresh mock.
        let new_mock = Arc::new(MockAdapter::default());
        let new_adapter: Arc<dyn QueueAdapter> = new_mock.clone();
        handler.adapter.replace(new_adapter, "new-mock").await;

        handler.resubscribe_all().await;

        // The old adapter never sees another call post-swap.
        assert_eq!(old_mock.calls().len(), 2);
        // The new adapter is subscribed once per tracked registration.
        let new_calls = new_mock.calls();
        assert_eq!(new_calls.len(), 2);
        assert!(new_calls.contains(&Call::Subscribe {
            topic: "demo-1".to_string(),
            id: "backend-1".to_string(),
            function_id: "backend-1".to_string(),
            metadata: None,
            condition_function_id: None,
            queue_config: Some(SubscriberQueueConfig {
                max_retries: Some(2),
                ..Default::default()
            }),
        }));
        assert!(new_calls.contains(&Call::Subscribe {
            topic: "demo-2".to_string(),
            id: "backend-2".to_string(),
            function_id: "backend-2".to_string(),
            metadata: None,
            condition_function_id: None,
            queue_config: Some(SubscriberQueueConfig::default()),
        }));
        // The registrations map itself is untouched by the swap.
        assert_eq!(handler.registrations().await.len(), 2);
    }

    #[tokio::test]
    async fn unregister_unknown_id_is_a_noop() {
        let (handler, mock) = handler_with_mock();
        handler
            .unregister_trigger(trigger_config("missing", "", Value::Null))
            .await
            .unwrap();
        assert!(mock.calls().is_empty());
    }

    #[test]
    fn supports_builtin_topic_and_queue_config_aliases() {
        let spec: SubscriberSpec = serde_json::from_value(json!({
            "topic": "demo",
            "queue_config": {
                "maxRetries": 9,
                "backoffDelayMs": 25
            }
        }))
        .unwrap();
        assert_eq!(spec.queue, "demo");
        let merged = merged_queue_config(&spec);
        assert_eq!(merged.max_retries, Some(9));
        assert_eq!(merged.backoff_delay_ms, Some(25));
    }

    #[test]
    fn top_level_overrides_win_over_queue_config() {
        let spec = SubscriberSpec {
            queue: "demo".to_string(),
            max_retries: Some(2),
            backoff_ms: Some(1),
            condition_function_id: None,
            queue_config: Some(SubscriberQueueConfig {
                max_retries: Some(9),
                backoff_delay_ms: Some(25),
                ..Default::default()
            }),
        };
        let merged = merged_queue_config(&spec);
        assert_eq!(merged.max_retries, Some(2));
        assert_eq!(merged.backoff_delay_ms, Some(1));
    }
}
