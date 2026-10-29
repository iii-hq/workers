//! Boot sequence: builtin guard → build adapter → hub → register the
//! `publish` function, which subscribers observe.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, InvocationCtx, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::adapters::{self, Relay};
use crate::config::PubSubConfig;
use crate::hub::Hub;
use crate::PUBLISH_FUNCTION_ID;

const LIST_WORKERS_FUNCTION_ID: &str = "engine::workers::list";
const BUILTIN_III_PUBSUB_WORKER_ID: &str = "iii-pubsub";
/// Metadata key of a `publish` call that relays an event another engine's
/// pubsub worker published; its value is that worker's origin id.
pub const RELAYED_FROM: &str = "pubsub_relayed_from";

const PUBLISH_DESCRIPTION: &str = "Publish an event to a pubsub topic. Subscribers observe this \
     function with a filter on input.topic and deliver: 'input', and receive { topic, data }. \
     Returns null.";

pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;
pub type ConfigCell = Arc<tokio::sync::RwLock<PubSubConfig>>;

/// Input of the `publish` function — exact field parity with the builtin
/// (engine/src/workers/pubsub/pubsub.rs:78-84).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PubSubInput {
    /// Topic to publish to. Observers filtered on this topic receive the event.
    pub topic: String,
    /// JSON payload delivered to each observer.
    pub data: Value,
    /// Caller worker id the engine injects into every invocation payload.
    /// Read only to key deprecation warnings: never serialized, never part of
    /// the advertised request schema, never forwarded to subscribers.
    #[serde(default, rename = "_caller_worker_id", skip_serializing)]
    #[schemars(skip)]
    pub caller_worker_id: Option<String>,
}

/// The `publish` handler. Emits the rate-limited deprecation warning (never
/// for the engine's `stream.events` bridge topic, nor for an event relayed
/// from another engine), then publishes: same result (`null`) and same
/// `topic_not_set` error. iii delivers the call to the observers of `publish`
/// on this engine; only `topic` and `data` go to the backend, so
/// `_caller_worker_id` is never forwarded.
pub async fn handle_publish(
    hub: &Hub,
    input: PubSubInput,
    relayed: bool,
) -> Result<PubSubPublishResponse, Error> {
    if !relayed {
        crate::deprecation::warn_publish(&input.topic, input.caller_worker_id.as_deref());
    }
    hub.publish(&input.topic, input.data, relayed)
        .await
        .map_err(Error::Handler)?;
    // Builtin returns Success(None) — a null result.
    Ok(PubSubPublishResponse)
}

/// Whether the invocation relays an event another engine's pubsub worker
/// published (see [`RELAYED_FROM`]).
pub fn is_relayed(ctx: Option<&InvocationCtx>) -> bool {
    ctx.and_then(|ctx| ctx.metadata.as_ref())
        .is_some_and(|metadata| metadata.get(RELAYED_FROM).is_some())
}

/// Success result for `publish`.
///
/// This intentionally serializes as JSON `null` for builtin parity while still
/// giving the SDK a concrete JsonSchema response type for registry publish.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct PubSubPublishResponse;

pub struct BootHandle {
    pub hub: Arc<Hub>,
    pub relay: Arc<dyn Relay>,
    pub config: ConfigCell,
    pub apply_lock: ApplyLock,
}

impl BootHandle {
    pub async fn shutdown(&self) {
        self.hub.shutdown().await;
    }
}

/// SDK-backed [`Relay`]: a fire-and-forget local call of `publish`, marked
/// with [`RELAYED_FROM`], so iii delivers it to the observers on this engine.
struct SdkRelay {
    iii: Arc<IIIClient>,
}

#[async_trait::async_trait]
impl Relay for SdkRelay {
    fn is_observed(&self) -> bool {
        self.iii.is_observed(PUBLISH_FUNCTION_ID)
    }

    async fn deliver(&self, topic: &str, data: Value, origin: &str) {
        let request = CallRequest {
            function_id: PUBLISH_FUNCTION_ID.to_string(),
            payload: json!({ "topic": topic, "data": data }),
            action: Some(CallAction::Void),
            timeout_ms: None,
        }
        .metadata(json!({ RELAYED_FROM: origin }));
        if let Err(e) = self.iii.call(request).await {
            tracing::debug!(topic = %topic, error = %e, "pubsub relay failed");
        }
    }
}

pub async fn start(iii: Arc<IIIClient>, config: PubSubConfig) -> anyhow::Result<BootHandle> {
    guard_against_builtin_pubsub(&iii).await?;

    let relay: Arc<dyn Relay> = Arc::new(SdkRelay { iii: iii.clone() });
    let adapter = adapters::build_adapter(&config, relay.clone()).await?;
    let hub = Arc::new(Hub::new(adapter));

    // Service function: the bare id `publish` (see PUBLISH_FUNCTION_ID docs —
    // exact path parity with the builtin is load-bearing). Public: anyone may
    // call it, and iii delivers every call to its observers.
    let hub_for_publish = hub.clone();
    iii.register_function(
        PUBLISH_FUNCTION_ID,
        RegisterFunction::new_async(move |input: PubSubInput| {
            let hub = hub_for_publish.clone();
            async move {
                let relayed = is_relayed(iii_sdk::current_invocation().as_ref());
                handle_publish(&hub, input, relayed).await
            }
        })
        .description(PUBLISH_DESCRIPTION),
    );

    Ok(BootHandle {
        hub,
        relay,
        config: Arc::new(tokio::sync::RwLock::new(config.normalized())),
        apply_lock: Arc::new(tokio::sync::Mutex::new(())),
    })
}

async fn guard_against_builtin_pubsub(iii: &Arc<IIIClient>) -> anyhow::Result<()> {
    let workers_list = iii
        .call(CallRequest {
            function_id: LIST_WORKERS_FUNCTION_ID.to_string(),
            payload: serde_json::json!({}),
            action: None,
            timeout_ms: Some(5000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("failed to query {LIST_WORKERS_FUNCTION_ID}: {e}"))?;

    if builtin_iii_pubsub_active(&workers_list) {
        anyhow::bail!(
            "cannot start the pubsub worker: the built-in iii-pubsub worker is active and owns \
             the 'publish' function. Remove iii-pubsub from the engine config (a config.yaml that doesn't list it won't run it), then start this \
             worker."
        );
    }
    Ok(())
}

fn builtin_iii_pubsub_active(workers_list: &serde_json::Value) -> bool {
    workers_list
        .get("workers")
        .and_then(|w| w.as_array())
        .is_some_and(|workers| {
            workers.iter().any(|worker| {
                worker.get("id").and_then(|v| v.as_str()) == Some(BUILTIN_III_PUBSUB_WORKER_ID)
                    || worker.get("name").and_then(|v| v.as_str())
                        == Some(BUILTIN_III_PUBSUB_WORKER_ID)
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_builtin_by_id() {
        let v = serde_json::json!({"workers": [{"id": "iii-pubsub"}]});
        assert!(builtin_iii_pubsub_active(&v));
    }

    #[test]
    fn detects_builtin_by_name() {
        let v = serde_json::json!({"workers": [{"id": "x", "name": "iii-pubsub"}]});
        assert!(builtin_iii_pubsub_active(&v));
    }

    #[test]
    fn absent_builtin_passes() {
        let v = serde_json::json!({"workers": [{"id": "iii-http"}]});
        assert!(!builtin_iii_pubsub_active(&v));
    }

    #[test]
    fn empty_list_passes() {
        assert!(!builtin_iii_pubsub_active(
            &serde_json::json!({"workers": []})
        ));
    }

    #[test]
    fn missing_key_passes() {
        assert!(!builtin_iii_pubsub_active(&serde_json::json!({})));
    }

    #[test]
    fn publish_response_schema_is_typed_null() {
        assert!(serde_json::to_value(PubSubPublishResponse)
            .unwrap()
            .is_null());

        let schema = serde_json::to_value(schemars::schema_for!(PubSubPublishResponse)).unwrap();
        assert_eq!(schema.get("type"), Some(&serde_json::json!("null")));
    }

    #[derive(Default)]
    struct RecordingAdapter {
        published: std::sync::Mutex<Vec<(String, Value)>>,
    }

    #[async_trait::async_trait]
    impl crate::adapters::PubSubAdapter for RecordingAdapter {
        async fn publish(&self, topic: &str, data: Value) {
            self.published
                .lock()
                .unwrap()
                .push((topic.to_string(), data));
        }
        async fn shutdown(&self) {}
    }

    fn input(payload: Value) -> PubSubInput {
        serde_json::from_value(payload).unwrap()
    }

    #[test]
    fn caller_worker_id_is_read_but_never_echoed_or_advertised() {
        let parsed = input(serde_json::json!({
            "topic": "orders",
            "data": {"id": 1},
            "_caller_worker_id": "550e8400-e29b-41d4-a716-446655440000"
        }));
        assert_eq!(
            parsed.caller_worker_id.as_deref(),
            Some("550e8400-e29b-41d4-a716-446655440000")
        );
        assert_eq!(
            serde_json::to_value(&parsed).unwrap(),
            serde_json::json!({"topic": "orders", "data": {"id": 1}})
        );
        assert!(input(serde_json::json!({"topic": "t", "data": null}))
            .caller_worker_id
            .is_none());

        // Advertised request format unchanged: only `topic` and `data`.
        let schema = serde_json::to_value(schemars::schema_for!(PubSubInput)).unwrap();
        let mut props: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        props.sort();
        assert_eq!(props, ["data", "topic"]);
        assert!(!schema.to_string().contains("caller"));
    }

    #[tokio::test]
    async fn publish_response_and_forwarding_unchanged_by_deprecation() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        let cases = [
            // Normal topic, warns (first call) and again (suppressed).
            serde_json::json!({"topic": "orders", "data": {"id": 1}, "_caller_worker_id": "w-boot-1"}),
            serde_json::json!({"topic": "orders", "data": {"id": 1}, "_caller_worker_id": "w-boot-1"}),
            // No caller id.
            serde_json::json!({"topic": "orders", "data": [1, 2]}),
            // Exempt stream bridge topic.
            serde_json::json!({"topic": "stream.events", "data": {"event": "x"}, "_caller_worker_id": "bridge"}),
        ];
        for case in cases.iter() {
            let out = handle_publish(&hub, input(case.clone()), false)
                .await
                .unwrap();
            assert!(serde_json::to_value(out).unwrap().is_null());
        }

        // The backend carries exactly `topic` and `data`: no caller id.
        let published = adapter.published.lock().unwrap().clone();
        let expected: Vec<(String, Value)> = cases
            .iter()
            .map(|c| (c["topic"].as_str().unwrap().to_string(), c["data"].clone()))
            .collect();
        assert_eq!(published, expected);
    }

    #[tokio::test]
    async fn publish_empty_topic_error_unchanged() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        let err = handle_publish(
            &hub,
            input(serde_json::json!({"topic": "", "data": 1})),
            false,
        )
        .await
        .expect_err("empty topic must fail");
        assert!(err.to_string().contains("topic_not_set: Topic is not set"));
        assert!(adapter.published.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_relayed_event_is_validated_but_not_carried_again() {
        let adapter = Arc::new(RecordingAdapter::default());
        let hub = Hub::new(adapter.clone());
        let out = handle_publish(
            &hub,
            input(serde_json::json!({"topic": "orders", "data": 1})),
            true,
        )
        .await
        .unwrap();
        assert!(serde_json::to_value(out).unwrap().is_null());
        assert!(adapter.published.lock().unwrap().is_empty());
        assert!(handle_publish(
            &hub,
            input(serde_json::json!({"topic": "", "data": 1})),
            true
        )
        .await
        .is_err());
    }

    fn ctx(metadata: Option<Value>) -> InvocationCtx {
        InvocationCtx {
            function_id: PUBLISH_FUNCTION_ID.to_string(),
            invocation_id: "inv-1".to_string(),
            caller: None,
            activation: None,
            observe_depth: 0,
            metadata,
            traceparent: None,
            baggage: None,
        }
    }

    #[test]
    fn only_the_relay_marker_makes_a_call_relayed() {
        assert!(!is_relayed(None));
        assert!(!is_relayed(Some(&ctx(None))));
        assert!(!is_relayed(Some(&ctx(Some(json!({"tenant": "acme"}))))));
        assert!(is_relayed(Some(&ctx(Some(
            json!({ RELAYED_FROM: "0123456789abcdef-42" })
        )))));
    }
}
