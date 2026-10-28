use std::{collections::BTreeMap, sync::Arc, time::Duration};

use async_trait::async_trait;
use iii_sdk::{
    protocol::{observe_codes, TriggerRequest, TriggerRequestWithMetadata},
    trigger::{TriggerConfig, TriggerHandler},
    Error, IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use serde_json::Value;
use tokio::{
    sync::{broadcast, RwLock},
    task::JoinHandle,
};

use crate::{api::*, manager::Manager};

// Accept engine transport metadata without weakening the strict public API.
#[derive(schemars::JsonSchema)]
#[schemars(transparent)]
struct EngineRequest<T>(T);

impl<'de, T: serde::Deserialize<'de>> serde::Deserialize<'de> for EngineRequest<T> {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let mut payload = serde_json::Value::deserialize(deserializer)?;
        if let Some(fields) = payload.as_object_mut() {
            fields.remove("_caller_worker_id");
        }
        T::deserialize(payload)
            .map(Self)
            .map_err(serde::de::Error::custom)
    }
}

pub const ACQUIRE: &str = "quick-tunnel::acquire";
pub const RELEASE: &str = "quick-tunnel::release";
pub const STATUS: &str = "quick-tunnel::status";
pub const CHANGED: &str = "quick-tunnel::changed";

/// The observable function next to [`CHANGED`].
pub const ON_CHANGED: &str = "quick-tunnel::on-changed";
pub const ON_CHANGED_DESCRIPTION: &str =
    "Fires when a tunnel lifecycle snapshot changes. Observe it with args { tunnel_id? } (the \
     quick-tunnel::changed binding config; omit tunnel_id for every authorized target); every \
     matching observer receives the Snapshot { tunnel_id, status, public_url, generation, error }. \
     Observe before acquire, then read quick-tunnel::status to close races. Only the \
     quick-tunnel worker fires it: invoking it directly is refused.";

/// Bindings and observers each hold at most this many subscriptions.
const SUBSCRIPTION_LIMIT: usize = 1024;
/// How long one observer delivery may take, as for a binding's trigger.
const DELIVERY_TIMEOUT: Duration = Duration::from_millis(5_000);

type ObserverEntry = (ChangedConfig, Arc<dyn SnapshotObserver>);

/// Subscriber registry shared by the `quick-tunnel::changed` trigger type
/// (bindings) and the `quick-tunnel::on-changed` observable function
/// (observers); one fan-out serves both.
#[derive(Clone)]
pub struct Changes {
    targets: Arc<Vec<String>>,
    bindings: Arc<RwLock<BTreeMap<String, (ChangedConfig, TriggerConfig)>>>,
    observers: Arc<RwLock<BTreeMap<String, ObserverEntry>>>,
}

fn selects(filter: &ChangedConfig, event: &Snapshot) -> bool {
    filter
        .tunnel_id
        .as_ref()
        .is_none_or(|id| id == &event.tunnel_id)
}

impl Changes {
    pub fn new(targets: Vec<String>) -> Self {
        Self {
            targets: Arc::new(targets),
            bindings: Arc::default(),
            observers: Arc::default(),
        }
    }

    /// A filter naming a tunnel names an authorized target.
    fn knows(&self, filter: &ChangedConfig) -> bool {
        filter
            .tunnel_id
            .as_ref()
            .is_none_or(|id| self.targets.contains(id))
    }

    async fn dispatch(&self, iii: &IIIClient, event: &Snapshot) {
        let bindings: Vec<_> = self
            .bindings
            .read()
            .await
            .values()
            .filter(|(filter, _)| selects(filter, event))
            .map(|(_, binding)| binding.clone())
            .collect();
        for binding in bindings {
            match routed_request(&binding, event) {
                Ok(request) => {
                    if let Err(error) = iii.trigger(request).await {
                        tracing::warn!(trigger_id = %binding.id, %error, "changed delivery failed; consumer must reconcile status");
                    }
                }
                Err(error) => tracing::error!(%error, "changed serialization failed"),
            }
        }
        self.notify_observers(event).await;
    }

    /// Hands `event` to every observer whose args select it, with `obs.call`,
    /// under the same filter and payload a binding gets.
    async fn notify_observers(&self, event: &Snapshot) {
        let observers: Vec<_> = self
            .observers
            .read()
            .await
            .values()
            .filter(|(filter, _)| selects(filter, event))
            .map(|(_, observer)| observer.clone())
            .collect();
        for observer in observers {
            match observer.deliver(event).await {
                Ok(()) => {}
                Err(Error::Remote { code, .. }) if code == observe_codes::FILTERS_NOT_MATCHED => {}
                Err(error) => {
                    tracing::warn!(observer = %observer.id(), %error, "on-changed delivery failed; consumer must reconcile status")
                }
            }
        }
    }

    /// Files an observer under its id; its args are parsed exactly as a
    /// binding config (`null` = every authorized target, unknown fields and
    /// unknown tunnels refused).
    pub async fn add_observer(
        &self,
        observer: Arc<dyn SnapshotObserver>,
    ) -> Result<(), ObserveRejection> {
        let args = observer.args_value();
        let filter: ChangedConfig = if args.is_null() {
            ChangedConfig::default()
        } else {
            serde_json::from_value(args.clone())
                .map_err(|e| ObserveRejection::new(observe_codes::INVALID_ARGS, e.to_string()))?
        };
        if !self.knows(&filter) {
            return Err(ObserveRejection::new(
                observe_codes::INVALID_ARGS,
                "unknown tunnel_id",
            ));
        }
        let id = observer.id().to_owned();
        let mut observers = self.observers.write().await;
        if observers.len() >= SUBSCRIPTION_LIMIT && !observers.contains_key(&id) {
            return Err(ObserveRejection::new(
                observe_codes::LIMIT_EXCEEDED,
                "subscription limit reached",
            ));
        }
        observers.insert(id, (filter, observer));
        Ok(())
    }

    pub async fn remove_observer(&self, id: &str) {
        self.observers.write().await.remove(id);
    }
}

/// Preserve target metadata and resolved namespace, including the legacy default.
pub fn routed_request(
    binding: &TriggerConfig,
    event: &Snapshot,
) -> Result<TriggerRequestWithMetadata, Error> {
    let mut request: TriggerRequestWithMetadata = TriggerRequest {
        function_id: binding.function_id.clone(),
        payload: serde_json::to_value(event)?,
        action: None,
        timeout_ms: Some(5_000),
    }
    .namespace(binding.namespace.as_deref().unwrap_or("default"));
    if let Some(metadata) = &binding.metadata {
        request = request.metadata(metadata.clone());
    }
    Ok(request)
}

#[async_trait]
impl TriggerHandler for Changes {
    async fn register_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        let filter: ChangedConfig = serde_json::from_value(binding.config.clone())?;
        if !self.knows(&filter) {
            return Err(Error::Handler("unknown tunnel_id".into()));
        }
        if binding
            .namespace
            .as_deref()
            .is_some_and(|ns| ns.trim().is_empty())
        {
            return Err(Error::Handler("empty namespace".into()));
        }
        let mut bindings = self.bindings.write().await;
        if bindings.len() >= SUBSCRIPTION_LIMIT && !bindings.contains_key(&binding.id) {
            return Err(Error::Handler("subscription limit reached".into()));
        }
        bindings.insert(binding.id.clone(), (filter, binding));
        Ok(())
    }
    async fn unregister_trigger(&self, binding: TriggerConfig) -> Result<(), Error> {
        self.bindings.write().await.remove(&binding.id);
        Ok(())
    }
}

/// One observer of [`ON_CHANGED`] as a sink of [`Changes`]: the SDK's
/// [`Observer`] in production, a recorder in tests (no engine needed).
#[async_trait]
pub trait SnapshotObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    /// The observer's args, parsed as a `quick-tunnel::changed` binding config.
    fn args_value(&self) -> &Value;
    /// Hands one snapshot to the observer's target (`obs.call`).
    async fn deliver(&self, event: &Snapshot) -> Result<(), Error>;
}

#[async_trait]
impl SnapshotObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn args_value(&self) -> &Value {
        Observer::args_value(self)
    }

    async fn deliver(&self, event: &Snapshot) -> Result<(), Error> {
        self.call_with_timeout::<Value>(event, DELIVERY_TIMEOUT)
            .await
            .map(|_| ())
    }
}

/// Files each observer of [`ON_CHANGED`] in the registry the
/// `quick-tunnel::changed` bindings live in, and takes it out when the engine
/// removes it.
struct OnChangedHandler {
    changes: Changes,
}

#[async_trait]
impl ObserveHandler for OnChangedHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format; parsed again here so a
        // config the trigger type would refuse is refused the same way.
        self.changes.add_observer(Arc::new(obs.clone())).await?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), "{ON_CHANGED} observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.changes.remove_observer(obs.id()).await;
        tracing::info!(observer = %obs.id(), "{ON_CHANGED} observer removed");
    }
}

/// The handler of [`ON_CHANGED`]: snapshots only come from this worker's own
/// fan-out, so every invocation is refused, whatever its input.
pub fn refuse_invocation(_input: Value) -> Result<(), Error> {
    Err(Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_CHANGED} is an event source: observe it \
             (iii.observe({{ function_id: '{ON_CHANGED}', args: {{ tunnel_id? }} }}, target)); \
             only the quick-tunnel worker fires its snapshots"
        ),
        stacktrace: None,
    })
}

/// Registers only local SDK handlers; never invokes cloudflared during registration.
pub fn register(iii: Arc<IIIClient>, manager: Manager, targets: Vec<String>) -> JoinHandle<()> {
    let changes = Changes::new(targets.clone());
    let mut events = manager.subscribe();
    // Kept for consumers that bind the trigger type, on 0.24 and 0.25 engines.
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(RegisterTriggerType::new(CHANGED, "Tunnel lifecycle snapshot changed; subscribe before acquire, then read status to close races", changes.clone())
        .trigger_request_format::<ChangedConfig>().call_request_format::<Snapshot>());
    let acquire = manager.clone();
    iii.register_function(ACQUIRE, RegisterFunction::new_async(move |EngineRequest(request): EngineRequest<AcquireRequest>| {
        let manager = acquire.clone();
        async move { manager.acquire(request).await.map_err(|e| Error::Handler(e.to_string())) }
    }).description("Acquire or extend an expiring durable lease for an operator-authorized local target; returns immediately, possibly starting"));
    let release = manager.clone();
    iii.register_function(
        RELEASE,
        RegisterFunction::new_async(
            move |EngineRequest(request): EngineRequest<ReleaseRequest>| {
                let manager = release.clone();
                async move {
                    manager
                        .release(request)
                        .await
                        .map_err(|e| Error::Handler(e.to_string()))
                }
            },
        )
        .description(
            "Release one lease idempotently; the last lease stops and reaps the tunnel child",
        ),
    );
    let status = manager.clone();
    iii.register_function(
        STATUS,
        RegisterFunction::new_async(
            move |EngineRequest(request): EngineRequest<StatusRequest>| {
                let manager = status.clone();
                async move {
                    manager
                        .status(request)
                        .await
                        .map_err(|e| Error::Handler(e.to_string()))
                }
            },
        )
        .description(
            "Read the latest tunnel generation, connection state, public URL and non-secret leases",
        ),
    );
    // Registered last; on an engine without observe it is a plain function
    // whose hooks never run, and the trigger type carries every snapshot.
    iii.register_function(
        ON_CHANGED,
        RegisterFunction::new(refuse_invocation)
            .description(ON_CHANGED_DESCRIPTION)
            .request_format(
                serde_json::to_value(schemars::schema_for!(Snapshot)).unwrap_or(Value::Null),
            )
            .observe(
                Observe::owner()
                    .args_format::<ChangedConfig>()
                    .handler(OnChangedHandler {
                        changes: changes.clone(),
                    }),
            ),
    );
    tokio::spawn(async move {
        loop {
            match events.recv().await {
                Ok(event) => changes.dispatch(&iii, &event).await,
                Err(broadcast::error::RecvError::Lagged(count)) => {
                    tracing::warn!(count, "changed receiver lagged; reconciling snapshots");
                    for id in &targets {
                        if let Ok(status) = manager
                            .status(StatusRequest {
                                tunnel_id: id.clone(),
                            })
                            .await
                        {
                            changes.dispatch(&iii, &status.snapshot).await;
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;
    use crate::api::Status;

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
    impl SnapshotObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn args_value(&self) -> &Value {
            &self.args
        }

        async fn deliver(&self, event: &Snapshot) -> Result<(), Error> {
            self.seen
                .lock()
                .unwrap()
                .push(serde_json::to_value(event).unwrap());
            Ok(())
        }
    }

    fn snapshot(tunnel_id: &str) -> Snapshot {
        Snapshot {
            tunnel_id: tunnel_id.into(),
            status: Status::Ready,
            public_url: Some("https://example.trycloudflare.com".into()),
            generation: "g-1".into(),
            error: None,
        }
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: "app::on-tunnel".into(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn on_changed_refuses_direct_invocation() {
        match refuse_invocation(serde_json::json!({ "tunnel_id": "web" })) {
            Err(Error::Remote { code, message, .. }) => {
                assert_eq!(code, observe_codes::FORBIDDEN);
                assert!(message.contains(ON_CHANGED));
            }
            other => panic!("expected FORBIDDEN, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn observer_receives_the_snapshot_a_binding_receives() {
        let changes = Changes::new(vec!["web".into(), "api".into()]);
        let config = serde_json::json!({ "tunnel_id": "web" });
        changes
            .register_trigger(binding("b-1", config.clone()))
            .await
            .expect("binding registers");
        let web = Recorder::new("o-web", config);
        let all = Recorder::new("o-all", Value::Null);
        changes.add_observer(web.clone()).await.unwrap();
        changes.add_observer(all.clone()).await.unwrap();

        let event = snapshot("web");
        changes.notify_observers(&event).await;
        changes.notify_observers(&snapshot("api")).await;

        // A binding with the same config is selected for the same snapshots,
        // and is triggered with the snapshot serialized as is.
        let bindings = changes.bindings.read().await;
        let (filter, stored) = bindings.get("b-1").expect("binding stored");
        assert!(selects(filter, &event));
        assert!(!selects(filter, &snapshot("api")));
        assert!(routed_request(stored, &event).is_ok());
        let binding_payload = serde_json::to_value(&event).unwrap();

        assert_eq!(web.seen(), vec![binding_payload.clone()]);
        assert_eq!(all.seen().len(), 2);
        assert_eq!(all.seen()[0], binding_payload);
        assert_eq!(all.seen()[1]["tunnel_id"], "api");
    }

    #[tokio::test]
    async fn observer_args_are_checked_like_a_binding_config() {
        let changes = Changes::new(vec!["web".into()]);
        for args in [
            serde_json::json!({ "tunnel_id": "other" }),
            serde_json::json!({ "unknown": true }),
        ] {
            let rejection = changes
                .add_observer(Recorder::new("o", args.clone()))
                .await
                .expect_err("refused like the trigger type refuses it");
            assert_eq!(rejection.code, observe_codes::INVALID_ARGS, "{args}");
            assert!(changes.register_trigger(binding("b", args)).await.is_err());
        }
        changes
            .add_observer(Recorder::new("o", serde_json::json!({})))
            .await
            .expect("empty args select every authorized target");
    }

    #[tokio::test]
    async fn removed_observer_gets_nothing_and_bindings_stay() {
        let changes = Changes::new(vec!["web".into()]);
        changes
            .register_trigger(binding("b-1", serde_json::json!({})))
            .await
            .unwrap();
        let obs = Recorder::new("o-1", Value::Null);
        changes.add_observer(obs.clone()).await.unwrap();
        changes.remove_observer("o-1").await;

        changes.notify_observers(&snapshot("web")).await;
        assert!(obs.seen().is_empty());
        assert!(changes.observers.read().await.is_empty());
        assert!(changes.bindings.read().await.contains_key("b-1"));
    }
}
