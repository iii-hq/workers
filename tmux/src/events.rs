//! `tmux::changed` and `tmux::on-changed` — the live feed behind the console
//! page.
//!
//! A binding of the `tmux::changed` trigger type, or an observer of the
//! observable function `tmux::on-changed` (iii 0.25), receives the full
//! [`Overview`] right after it subscribes and again whenever anything in it
//! changes. Both live in one subscriber set and are fed by one poll loop,
//! which only runs while at least one subscriber exists; mutations through
//! this worker poke it so their effect shows at once instead of on the next
//! tick.
//!
//! Binding delivery mirrors the ide's trigger types: `TriggerAction::Void`,
//! one task per fire, and the binding's `metadata` and `namespace` passed
//! through — a fire that drops `__binding` is discarded by the harness
//! without an error (MOT-4719). Observers get the same payload through
//! `obs.call`, one task per delivery.
//!
//! Changes are routed in-process, never by invoking `tmux::on-changed`: its
//! handler refuses every invocation with `FORBIDDEN`, so no caller can make
//! this worker hand observers a forged overview. On an engine without
//! observe the function registers as a plain function, its hooks never run,
//! and the trigger type carries everything.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{observe_codes, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType, TriggerAction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::collector::Collector;
use crate::model::Overview;

pub const CHANGED: &str = "tmux::changed";

/// The observable function next to [`CHANGED`].
pub const ON_CHANGED: &str = "tmux::on-changed";

pub const ON_CHANGED_DESCRIPTION: &str =
    "Live tmux overview as an observable function: observe it with args {} (the tmux::changed \
     config, no options); every observer receives the full state { sessions, ports } (sessions, \
     windows, panes with agent/compose/port status, compose stacks and listening ports) right \
     after it is added and again whenever it changes. Only the tmux worker fires it: invoking it \
     directly is refused.";

/// `tmux::changed` takes no options; neither do `tmux::on-changed` observers.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangedConfig {}

/// Never returned: `tmux::on-changed` refuses every invocation with
/// `FORBIDDEN`. Its observers' targets receive the overview as their input.
#[derive(Debug, Clone, Default, Serialize, JsonSchema)]
pub struct OnChangedResponse {}

/// How long a delivery waits for an observer the engine has not marked
/// active yet: `on_observer_added` files it, and its first frame is sent,
/// before the engine acknowledges it.
const ACTIVATION_WAIT: Duration = Duration::from_secs(5);
const ACTIVATION_POLL: Duration = Duration::from_millis(50);

/// One observer of [`ON_CHANGED`] as a sink of the [`Monitor`]: the SDK's
/// [`Observer`] in production, a recorder in tests (no engine needed).
pub trait OverviewObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    /// Hands one overview to the observer's target (`obs.call`), on its own
    /// task.
    fn deliver(&self, payload: Value);
}

impl OverviewObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn deliver(&self, payload: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + ACTIVATION_WAIT;
            while !obs.is_active() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(ACTIVATION_POLL).await;
            }
            match obs.call::<Value>(payload).await {
                Ok(_) => {}
                // The observer's own filters declined this overview.
                Err(Error::Remote { code, .. }) if code == observe_codes::FILTERS_NOT_MATCHED => {}
                Err(error) => {
                    tracing::debug!(observer = %obs.id(), target = %obs.target(), %error, "tmux::on-changed delivery failed")
                }
            }
        });
    }
}

/// The subscriber-set key of an observer, apart from every binding id: the
/// engine's trigger ids and observer ids are separate spaces.
fn observer_key(id: &str) -> String {
    format!("observer:{id}")
}

/// A `tmux::changed` binding.
#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    metadata: Option<Value>,
    namespace: Option<String>,
}

impl Binding {
    fn request(&self, payload: Value) -> TriggerRequestWithMetadata {
        let mut request: TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id.clone(),
            payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(metadata) = &self.metadata {
            request = request.metadata(metadata.clone());
        }
        if let Some(namespace) = &self.namespace {
            request = request.namespace(namespace.clone());
        }
        request
    }
}

/// One entry of the subscriber set: a binding or an observer.
#[derive(Clone)]
enum Subscriber {
    Binding(Binding),
    Observer(Arc<dyn OverviewObserver>),
}

/// Bindings and observers plus the poll loop that feeds them.
pub struct Monitor {
    iii: IIIClient,
    collector: Arc<Collector>,
    subscribers: Mutex<HashMap<String, Subscriber>>,
    wake: Notify,
}

/// The overview without its timestamps, so an unchanged state is not re-sent.
fn fingerprint(overview: &Overview) -> Option<Value> {
    let mut value = serde_json::to_value(overview).ok()?;
    value["sessions"]["generated_at_ms"] = Value::Null;
    value["ports"]["generated_at_ms"] = Value::Null;
    Some(value)
}

impl Monitor {
    pub fn new(iii: IIIClient, collector: Arc<Collector>) -> Arc<Self> {
        Arc::new(Self {
            iii,
            collector,
            subscribers: Mutex::new(HashMap::new()),
            wake: Notify::new(),
        })
    }

    /// Re-collect now (after a mutation) instead of on the next tick.
    pub fn poke(&self) {
        self.wake.notify_one();
    }

    fn targets(&self) -> Vec<Subscriber> {
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .cloned()
            .collect()
    }

    fn deliver(&self, targets: Vec<Subscriber>, payload: Value) {
        for subscriber in targets {
            match subscriber {
                Subscriber::Binding(binding) => {
                    let iii = self.iii.clone();
                    let request = binding.request(payload.clone());
                    tokio::spawn(async move {
                        if let Err(error) = iii.trigger(request).await {
                            tracing::debug!(function_id = %binding.function_id, %error, "tmux::changed delivery failed");
                        }
                    });
                }
                Subscriber::Observer(observer) => observer.deliver(payload.clone()),
            }
        }
    }

    /// Files `subscriber` under `key` and sends it the current overview
    /// alone, right away.
    fn subscribe(self: &Arc<Self>, key: String, subscriber: Subscriber) {
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(key, subscriber.clone());
        let monitor = self.clone();
        tokio::spawn(async move {
            let overview = monitor.collector.overview().await;
            if let Ok(payload) = serde_json::to_value(&overview) {
                monitor.deliver(vec![subscriber], payload);
            }
            monitor.poke();
        });
    }

    fn unsubscribe(&self, key: &str) {
        self.subscribers
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(key);
    }

    /// Files an observer of [`ON_CHANGED`] next to the bindings; it gets the
    /// first frame and every change a binding gets.
    pub fn add_observer(self: &Arc<Self>, observer: Arc<dyn OverviewObserver>) {
        let key = observer_key(observer.id());
        self.subscribe(key, Subscriber::Observer(observer));
    }

    pub fn remove_observer(&self, id: &str) {
        self.unsubscribe(&observer_key(id));
    }

    /// The poll loop. Sleeps while nothing is bound or observing.
    pub fn spawn(self: &Arc<Self>) {
        let monitor = self.clone();
        tokio::spawn(async move {
            let mut last: Option<Value> = None;
            loop {
                let wake = monitor.wake.notified();
                tokio::pin!(wake);
                wake.as_mut().enable();
                if monitor.targets().is_empty() {
                    last = None;
                    wake.await;
                    continue;
                }
                let overview = monitor.collector.overview().await;
                let print = fingerprint(&overview);
                if print.is_some() && print != last {
                    if let Ok(payload) = serde_json::to_value(&overview) {
                        monitor.deliver(monitor.targets(), payload);
                    }
                    last = print;
                }
                let interval = Duration::from_millis(monitor.collector.config().poll_interval_ms);
                tokio::select! {
                    _ = tokio::time::sleep(interval) => {}
                    _ = wake => {}
                }
            }
        });
    }
}

struct ChangedHandler {
    monitor: Arc<Monitor>,
}

#[async_trait]
impl TriggerHandler for ChangedHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let binding = Binding {
            function_id: config.function_id.clone(),
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
        };
        tracing::debug!(trigger_type = CHANGED, id = %config.id, "live feed bound");
        self.monitor
            .subscribe(config.id.clone(), Subscriber::Binding(binding));
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::debug!(trigger_type = CHANGED, id = %config.id, "live feed unbound");
        self.monitor.unsubscribe(&config.id);
        Ok(())
    }
}

/// Files each observer of [`ON_CHANGED`] in the monitor's subscriber set,
/// and takes it out when the engine removes it.
struct OnChangedHandler {
    monitor: Arc<Monitor>,
}

#[async_trait]
impl ObserveHandler for OnChangedHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format; parsed as a binding
        // config so anything the trigger type would refuse is refused.
        let ChangedConfig {} = obs.args()?;
        self.monitor.add_observer(Arc::new(obs.clone()));
        tracing::debug!(observer = %obs.id(), target = %obs.target(), "{ON_CHANGED} observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.monitor.remove_observer(obs.id());
        tracing::debug!(observer = %obs.id(), "{ON_CHANGED} observer removed");
    }
}

/// The handler of [`ON_CHANGED`]: overviews only come from the poll loop,
/// so every invocation is refused, whatever its input.
pub fn refuse_invocation(_input: Value) -> Result<(), Error> {
    Err(Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_CHANGED} is an event source: observe it \
             (iii.observe({{ function_id: '{ON_CHANGED}', args: {{}} }}, target)); \
             only the tmux worker fires its overviews"
        ),
        stacktrace: None,
    })
}

/// Registers `tmux::on-changed`, owner-delivered, with [`OnChangedHandler`]
/// as its hooks over the monitor's subscriber set.
pub fn register_on_changed(iii: &IIIClient, monitor: Arc<Monitor>) {
    let schema =
        |root: schemars::schema::RootSchema| serde_json::to_value(root).unwrap_or(Value::Null);
    let settings = schemars::r#gen::SchemaSettings::draft07();
    iii.register_function(
        ON_CHANGED,
        RegisterFunction::new(refuse_invocation)
            .description(ON_CHANGED_DESCRIPTION)
            .request_format(schema(
                settings
                    .clone()
                    .into_generator()
                    .into_root_schema_for::<Overview>(),
            ))
            .response_format(schema(
                settings
                    .into_generator()
                    .into_root_schema_for::<OnChangedResponse>(),
            ))
            .observe(
                Observe::owner()
                    .args_format::<ChangedConfig>()
                    .handler(OnChangedHandler { monitor }),
            ),
    );
}

pub fn register_changed_trigger(iii: &IIIClient, monitor: Arc<Monitor>) {
    // `register_trigger_type` is deprecated in iii-sdk 0.25. Kept until the
    // trigger type is retired; old consumers bind it on 0.24 and 0.25 engines.
    #[allow(deprecated)]
    let _handle = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGED,
            "Live tmux overview: fires the full state (sessions, windows, panes with agent/compose/port \
             status, compose stacks and listening ports) when bound and again whenever it changes. \
             Config takes no options.",
            ChangedHandler { monitor },
        )
        .trigger_request_format::<ChangedConfig>()
        .call_request_format::<Overview>(),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{PortsSnapshot, SessionsSnapshot, TmuxInfo};

    fn overview(at: u64) -> Overview {
        Overview {
            sessions: SessionsSnapshot {
                tmux: TmuxInfo::default(),
                sessions: vec![],
                generated_at_ms: at,
            },
            ports: PortsSnapshot {
                supported: true,
                error: None,
                composes: vec![],
                listeners: vec![],
                generated_at_ms: at,
            },
        }
    }

    #[test]
    fn timestamps_alone_are_not_a_change() {
        assert_eq!(fingerprint(&overview(1)), fingerprint(&overview(2)));
        let mut changed = overview(1);
        changed.ports.supported = false;
        assert_ne!(fingerprint(&overview(1)), fingerprint(&changed));
    }

    /// Stands in for the SDK observer: records what `obs.call` would carry
    /// to its target.
    struct Recorder {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    impl Recorder {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                seen: Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    impl OverviewObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn deliver(&self, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    /// A monitor whose engine connection and tmux server never exist.
    fn monitor() -> Arc<Monitor> {
        let config = crate::config::WorkerConfig {
            socket_name: Some(format!("iii-tmux-unit-absent-{}", std::process::id())),
            ..crate::config::WorkerConfig::default()
        }
        .into_shared();
        Monitor::new(
            IIIClient::new("ws://stub-not-connected:0"),
            Arc::new(Collector::new(
                config,
                crate::collector::ConsoleClients::default(),
            )),
        )
    }

    fn binding(function_id: &str) -> Subscriber {
        Subscriber::Binding(Binding {
            function_id: function_id.into(),
            metadata: Some(serde_json::json!({ "tab": "a" })),
            namespace: None,
        })
    }

    /// Files subscribers without the first-frame task, so the test sees only
    /// the deliveries it makes.
    fn file(monitor: &Monitor, key: &str, subscriber: Subscriber) {
        monitor
            .subscribers
            .lock()
            .unwrap()
            .insert(key.to_string(), subscriber);
    }

    #[test]
    fn on_changed_refuses_direct_invocation() {
        for input in [
            Value::Null,
            serde_json::json!({}),
            serde_json::to_value(overview(1)).unwrap(),
        ] {
            match refuse_invocation(input) {
                Err(Error::Remote { code, message, .. }) => {
                    assert_eq!(code, observe_codes::FORBIDDEN);
                    assert_eq!(code, "FORBIDDEN");
                    assert!(message.contains(ON_CHANGED), "{message}");
                }
                other => panic!("expected FORBIDDEN, got {other:?}"),
            }
        }
    }

    #[test]
    fn on_changed_sits_next_to_the_trigger_type() {
        assert_eq!(CHANGED, "tmux::changed");
        assert_eq!(ON_CHANGED, "tmux::on-changed");
        // Observers take the binding config, which has no options.
        let ChangedConfig {} = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(ON_CHANGED_DESCRIPTION.contains("refused"));
    }

    #[tokio::test]
    async fn observer_receives_the_change_a_binding_receives() {
        let monitor = monitor();
        file(&monitor, "b1", binding("console::tmux-changed"));
        let observer = Recorder::new("o1");
        file(
            &monitor,
            &observer_key("o1"),
            Subscriber::Observer(observer.clone()),
        );

        let targets = monitor.targets();
        assert_eq!(targets.len(), 2, "the binding and the observer");
        let bound: Vec<String> = targets
            .iter()
            .filter_map(|subscriber| match subscriber {
                Subscriber::Binding(binding) => Some(binding.function_id.clone()),
                Subscriber::Observer(_) => None,
            })
            .collect();
        assert_eq!(bound, ["console::tmux-changed"]);

        // The poll loop hands one payload to every target: the binding is
        // fired with it and the observer receives it unchanged.
        let payload = serde_json::to_value(overview(7)).unwrap();
        monitor.deliver(targets, payload.clone());
        assert_eq!(observer.seen(), vec![payload.clone()]);

        let second = serde_json::to_value(overview(8)).unwrap();
        monitor.deliver(monitor.targets(), second.clone());
        assert_eq!(observer.seen(), vec![payload, second]);
    }

    #[tokio::test]
    async fn observer_keys_never_collide_with_binding_ids() {
        let monitor = monitor();
        file(&monitor, "same", binding("console::tmux-changed"));
        let observer = Recorder::new("same");
        file(
            &monitor,
            &observer_key("same"),
            Subscriber::Observer(observer.clone()),
        );
        assert_eq!(monitor.targets().len(), 2);

        // Removing the observer leaves the binding of the same id bound.
        monitor.remove_observer("same");
        let targets = monitor.targets();
        assert_eq!(targets.len(), 1);
        assert!(matches!(targets[0], Subscriber::Binding(_)));

        monitor.deliver(targets, serde_json::to_value(overview(1)).unwrap());
        assert!(
            observer.seen().is_empty(),
            "a removed observer gets nothing"
        );

        // And unbinding leaves nothing behind for the poll loop.
        monitor.unsubscribe("same");
        assert!(monitor.targets().is_empty());
    }
}
