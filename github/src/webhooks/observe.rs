//! iii 0.25: `github::on-pr-event`, the owner-delivered observable function
//! next to the `github::pr::event` trigger type.
//!
//! An observer passes the binding config as args (an [`EventFilter`], same
//! validation) and is filed in the same durable subscriber set as the
//! bindings, under the id `observer:<observer id>` and a target that routes
//! back to its in-memory [`Observer`] handle. Routing, notification
//! profiles, digests and watch liveness therefore treat it exactly like a
//! binding, and each delivery a binding would receive reaches the observer
//! through `obs.call` instead of a trigger. Invoking `github::on-pr-event`
//! directly is refused with FORBIDDEN.
//!
//! Observer handles do not survive a restart while their stored copies do:
//! a delivery to an observer without a live handle is skipped (acked), and
//! maintenance drops such stored copies the way it drops bindings the engine
//! no longer lists. The engine re-adds live observers on reconnect.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use async_trait::async_trait;
use iii_sdk::{ObserveHandler, ObserveRejection, Observer};
use serde_json::Value;

use super::notifications::NotificationEvent;
use super::wiring::{parse_filter, storage_task};
use super::{Failure, Result, Service, Subscriber};
use crate::events::{is_skipped_delivery, refuse_invocation};

/// Observable function next to the `github::pr::event` trigger type.
pub const ON_PR_EVENT: &str = "github::on-pr-event";
pub(super) const ON_PR_EVENT_DESCRIPTION: &str = "Durable PR lifecycle, comment, review and \
     per-entity CI events. Observe it with args { repo?, number?, watch_id?, categories?, \
     notifications? } (the github::pr::event config, same validation); each observer receives \
     what a github::pr::event binding with that config receives. Arm before watch and read one \
     snapshot after watch. Only the github worker delivers it: invoking it directly is refused.";

/// Codes an observer is refused with.
pub mod codes {
    /// Args the binding-config validation refuses.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
    /// The observer could not be stored.
    pub const STORAGE_FAILED: &str = "STORAGE_FAILED";
}

/// Stored subscriber ids of observers start with this.
pub const OBSERVER_SUBSCRIBER_PREFIX: &str = "observer:";
/// Stored subscriber targets of observers start with this; never a function
/// the engine knows (`#` is not used in function ids).
const OBSERVER_TARGET_PREFIX: &str = "github::on-pr-event#";

/// One observer of `github::on-pr-event`, as a delivery reaches it:
/// [`Observer`] delivers with `obs.call`.
#[async_trait]
pub trait PrEventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one delivery payload to the observer's target.
    async fn deliver(&self, payload: Value) -> std::result::Result<(), iii_sdk::Error>;
}

#[async_trait]
impl PrEventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: Value) -> std::result::Result<(), iii_sdk::Error> {
        self.call::<Value>(payload).await.map(|_| ())
    }
}

type ObserverMap = HashMap<String, Arc<dyn PrEventObserver>>;

/// Live observer handles by observer id.
fn live() -> MutexGuard<'static, ObserverMap> {
    static LIVE: OnceLock<Mutex<ObserverMap>> = OnceLock::new();
    LIVE.get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

/// The stored subscriber id of an observer.
pub fn subscriber_id(observer_id: &str) -> String {
    format!("{OBSERVER_SUBSCRIBER_PREFIX}{observer_id}")
}

/// True for a stored subscriber that is an observer, not a binding.
pub fn is_observer_subscriber(subscriber_id: &str) -> bool {
    subscriber_id.starts_with(OBSERVER_SUBSCRIBER_PREFIX)
}

/// True when the stored observer subscriber still has a live handle.
pub fn is_live_subscriber(subscriber_id: &str) -> bool {
    subscriber_id
        .strip_prefix(OBSERVER_SUBSCRIBER_PREFIX)
        .is_some_and(|id| live().contains_key(id))
}

fn target_function(observer_id: &str) -> String {
    format!("{OBSERVER_TARGET_PREFIX}{observer_id}")
}

/// The observer id when `function_id` is an observer's stored target.
pub(super) fn observer_target(function_id: &str) -> Option<&str> {
    function_id.strip_prefix(OBSERVER_TARGET_PREFIX)
}

/// File an observer under its args, validated as a `github::pr::event`
/// binding config, in the same subscriber set as the bindings (replacing one
/// with the same id).
pub async fn add_observer(
    service: &Arc<Service>,
    args: &Value,
    observer: Arc<dyn PrEventObserver>,
) -> std::result::Result<(), ObserveRejection> {
    let filter = parse_filter(args.clone())
        .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e.to_string()))?;
    let observer_id = observer.id().to_string();
    // Live before stored: a stored observer always has a handle until restart.
    live().insert(observer_id.clone(), observer);
    let subscriber = Subscriber {
        id: subscriber_id(&observer_id),
        function_id: target_function(&observer_id),
        filter,
        metadata: None,
        namespace: None,
    };
    let service = service.clone();
    let stored = storage_task(async move {
        if let Some(store) = &service.store {
            store.change(|d| {
                d.subscribers.insert(subscriber.id.clone(), subscriber);
                Ok(())
            })?;
        }
        Ok(())
    })
    .await;
    if let Err(e) = stored {
        live().remove(&observer_id);
        return Err(ObserveRejection::new(codes::STORAGE_FAILED, e.to_string()));
    }
    Ok(())
}

/// Remove an observer: its handle, then its stored copy, exactly as a
/// binding's unregister does (pending deliveries cancelled, watches left
/// without a listener stopped after the grace period).
pub async fn remove_observer(service: &Arc<Service>, observer_id: &str) -> Result<()> {
    live().remove(observer_id);
    let service = service.clone();
    let id = subscriber_id(observer_id);
    storage_task(async move {
        if service.store.is_some() {
            service.drop_subscribers(&[id]).await?;
        }
        Ok(())
    })
    .await
}

/// Deliver one payload to a live observer with `obs.call`. A missing handle
/// or a skipped call (filters not matched, observer gone) sent nothing and is
/// not a failure; any other error fails the delivery so the job is retried.
pub(super) async fn deliver(observer_id: &str, payload: Value) -> Result<Value> {
    let observer = live().get(observer_id).cloned();
    let Some(observer) = observer else {
        tracing::debug!(observer = %observer_id, "github::on-pr-event observer not live; delivery skipped");
        return Ok(Value::Null);
    };
    match observer.deliver(payload).await {
        Ok(()) => Ok(Value::Null),
        Err(e) if is_skipped_delivery(&e) => {
            tracing::debug!(observer = %observer_id, error = %e, "github::on-pr-event observer skipped the event");
            Ok(Value::Null)
        }
        Err(e) => Err(Failure::Engine(e)),
    }
}

/// Handler of `github::on-pr-event`: typed by the [`NotificationEvent`] its
/// observers receive, as a `github::pr::event` binding does (the published
/// request schema); any invocation is refused.
pub(super) fn refuse_pr_event(
    _event: NotificationEvent,
) -> std::result::Result<(), iii_sdk::Error> {
    Err(refuse_invocation(ON_PR_EVENT))
}

/// The observe hooks of `github::on-pr-event`.
pub struct PrEventObserveHandler {
    service: Arc<Service>,
}

impl PrEventObserveHandler {
    pub fn new(service: Arc<Service>) -> Self {
        Self { service }
    }
}

#[async_trait]
impl ObserveHandler for PrEventObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> std::result::Result<(), ObserveRejection> {
        add_observer(&self.service, obs.args_value(), Arc::new(obs.clone()))
            .await
            .inspect_err(|rejection| {
                tracing::warn!(function_id = ON_PR_EVENT, observer = %obs.id(), message = %rejection.message, "observer rejected");
            })?;
        tracing::info!(function_id = ON_PR_EVENT, observer = %obs.id(), target = %obs.target(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        if let Err(e) = remove_observer(&self.service, obs.id()).await {
            tracing::warn!(function_id = ON_PR_EVENT, observer = %obs.id(), error = %e, "observer removal not stored");
        } else {
            tracing::info!(function_id = ON_PR_EVENT, observer = %obs.id(), "observer removed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::webhooks::tests::service;
    use serde_json::json;

    /// An observer of `github::on-pr-event` that records what it is handed.
    struct Recording {
        id: String,
        fail_with: Option<&'static str>,
        received: Mutex<Vec<Value>>,
    }

    impl Recording {
        fn new(id: &str, fail_with: Option<&'static str>) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                fail_with,
                received: Mutex::new(Vec::new()),
            })
        }

        fn received(&self) -> Vec<Value> {
            self.received.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl PrEventObserver for Recording {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) -> std::result::Result<(), iii_sdk::Error> {
            if let Some(code) = self.fail_with {
                return Err(iii_sdk::Error::Remote {
                    code: code.to_string(),
                    message: "not sent".to_string(),
                    stacktrace: None,
                });
            }
            self.received.lock().unwrap().push(payload);
            Ok(())
        }
    }

    #[test]
    fn on_pr_event_refuses_direct_invocation() {
        assert_eq!(ON_PR_EVENT, "github::on-pr-event");
        let error = refuse_pr_event(compact_event()).unwrap_err();
        assert!(
            matches!(&error, iii_sdk::Error::Remote { code, message, .. }
                if code == crate::events::codes::FORBIDDEN && message.contains(ON_PR_EVENT)),
            "{error:?}"
        );
    }

    /// A compact delivery, as an observer with a compact policy receives it.
    fn compact_event() -> NotificationEvent {
        serde_json::from_value(json!({
            "event_id": "e1",
            "watch_id": "w1",
            "repo": "owner/repo",
            "number": 1,
            "head_sha": "abc",
            "category": "ci",
            "kind": "ci.failed",
            "final_event": false,
            "body_truncated": false
        }))
        .unwrap()
    }

    #[test]
    fn on_pr_event_publishes_typed_schemas() {
        use crate::events::tests::{assert_typed_schema, handler_schemas};
        let (request, response) = handler_schemas(&refuse_pr_event);
        assert_typed_schema(ON_PR_EVENT, "request", &request);
        assert_typed_schema(ON_PR_EVENT, "response", &response);
        // Same schema a github::pr::event binding's call request publishes.
        let expected = serde_json::to_value(
            schemars::r#gen::SchemaSettings::draft07()
                .into_generator()
                .into_root_schema_for::<NotificationEvent>(),
        )
        .unwrap();
        assert_eq!(request, expected);
    }

    #[tokio::test]
    async fn an_observer_is_filed_like_a_binding_and_receives_its_deliveries() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(service(dir.path()).await);
        let args = json!({ "repo": "owner/repo", "number": 1 });
        let observer = Recording::new("pr-obs-filed", None);
        add_observer(&service, &args, observer.clone())
            .await
            .unwrap();

        // Stored in the bindings' subscriber set with the filter a binding
        // with the same config gets.
        let stored = service.store().unwrap().read().unwrap();
        let subscriber = &stored.subscribers[&subscriber_id("pr-obs-filed")];
        assert_eq!(subscriber.function_id, "github::on-pr-event#pr-obs-filed");
        assert_eq!(
            serde_json::to_value(&subscriber.filter).unwrap(),
            serde_json::to_value(parse_filter(args.clone()).unwrap()).unwrap()
        );
        assert_eq!(subscriber.filter.repo.as_deref(), Some("owner/repo"));
        assert_eq!(subscriber.filter.number, Some(1));
        assert!(is_observer_subscriber(&subscriber.id));
        assert!(is_live_subscriber(&subscriber.id));
        assert!(!is_observer_subscriber("binding-1"));

        // A delivery to its stored target reaches it through obs.call, verbatim.
        let payload = json!({ "repo": "owner/repo", "number": 1, "category": "lifecycle" });
        let out = service
            .invoke_target(&subscriber.function_id, payload.clone(), None, None)
            .await
            .unwrap();
        assert_eq!(out, Value::Null);
        assert_eq!(observer.received(), vec![payload]);

        remove_observer(&service, "pr-obs-filed").await.unwrap();
        assert!(!is_live_subscriber(&subscriber_id("pr-obs-filed")));
        let stored = service.store().unwrap().read().unwrap();
        assert!(!stored
            .subscribers
            .contains_key(&subscriber_id("pr-obs-filed")));
    }

    #[tokio::test]
    async fn observer_args_follow_the_binding_config_validation() {
        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(service(dir.path()).await);
        for bad in [json!({ "repo": "not a repo" }), json!({ "number": "one" })] {
            let rejection = add_observer(&service, &bad, Recording::new("pr-obs-bad", None))
                .await
                .unwrap_err();
            assert_eq!(rejection.code, codes::INVALID_ARGS, "{bad}");
        }
        assert!(!is_live_subscriber(&subscriber_id("pr-obs-bad")));
    }

    #[tokio::test]
    async fn skipped_missing_and_failing_deliveries() {
        // No live handle (restart): skipped and acked.
        assert_eq!(
            deliver("pr-obs-none", json!({})).await.unwrap(),
            Value::Null
        );

        let dir = tempfile::tempdir().unwrap();
        let service = Arc::new(service(dir.path()).await);
        let skipped = Recording::new(
            "pr-obs-skipped",
            Some(crate::events::codes::FILTERS_NOT_MATCHED),
        );
        add_observer(&service, &json!({}), skipped).await.unwrap();
        assert_eq!(
            deliver("pr-obs-skipped", json!({})).await.unwrap(),
            Value::Null
        );

        let broken = Recording::new("pr-obs-broken", Some("TARGET_FAILED"));
        add_observer(&service, &json!({}), broken).await.unwrap();
        assert!(
            deliver("pr-obs-broken", json!({})).await.is_err(),
            "retried"
        );
    }
}
