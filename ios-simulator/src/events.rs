//! The four trigger types this worker provides, and the observable
//! `ios-simulator::on-*` function next to each one (iii 0.25).
//!
//! - `ios-simulator::device-changed` / `ios-simulator::on-device-changed` — a
//!   simulator appeared, disappeared or changed state (booted, shut down,
//!   renamed), whoever caused it: this worker, Xcode, or `simctl` in a
//!   terminal.
//! - `ios-simulator::media-changed` / `ios-simulator::on-media-changed` — a
//!   screenshot or recording was saved or deleted.
//! - `ios-simulator::frame-event` / `ios-simulator::on-frame-event` — one
//!   live-view frame of a watched simulator, delivered straight to the
//!   bindings and observers (as `browser::frame-event` does): the live view
//!   depends on no other worker.
//! - `ios-simulator::activity` / `ios-simulator::on-activity` — a caller drove
//!   a simulator (a gesture, a button, typing, an app, a screenshot,
//!   `ios-simulator::open`), so live previews can surface it while an agent
//!   works.
//!
//! Config `{ tenant?, udid? }` narrows a binding; the same object is an
//! observer's args. A binding without `tenant` sees every tenant (the
//! operator's console); a shared deployment's `rbac-proxy`
//! trigger-registration hook stamps `tenant` on bindings made by tenants.
//! Each on-* function's [`ObserveHandler`] files observers in the subscriber
//! set its trigger type uses. Delivery is fire-and-forget: in the binding's
//! namespace for bindings, with `obs.call` for observers. Events are routed
//! in-process: invoking an on-* function is refused with FORBIDDEN, so nobody
//! can forge one.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const DEVICE_CHANGED: &str = "ios-simulator::device-changed";
pub const MEDIA_CHANGED: &str = "ios-simulator::media-changed";
pub const FRAME_EVENT: &str = "ios-simulator::frame-event";
pub const ACTIVITY: &str = "ios-simulator::activity";

pub const ON_DEVICE_CHANGED: &str = "ios-simulator::on-device-changed";
pub const ON_MEDIA_CHANGED: &str = "ios-simulator::on-media-changed";
pub const ON_FRAME_EVENT: &str = "ios-simulator::on-frame-event";
pub const ON_ACTIVITY: &str = "ios-simulator::on-activity";

const ON_DEVICE_CHANGED_DESC: &str =
    "A simulator was added, removed, booted or shut down (by this worker, Xcode or simctl). \
     Observe it with args { tenant?, udid? }; each observer receives { tenant, udid, name, \
     state, previous_state, change, preview, timestamp }. Only the ios-simulator worker emits \
     it: invoking it directly is refused.";
const ON_MEDIA_CHANGED_DESC: &str =
    "A simulator screenshot or recording was saved or deleted. Observe it with args \
     { tenant?, udid? }; each observer receives { tenant, udid, name, kind, change, timestamp }. \
     Only the ios-simulator worker emits it: invoking it directly is refused.";
const ON_FRAME_EVENT_DESC: &str =
    "A live-view frame of a simulator someone is watching (ios-simulator::watch). Observe it \
     with args { tenant?, udid? }; each observer receives { tenant, udid, data, width, height, \
     device_width, device_height, seq }. Only the ios-simulator worker emits it: invoking it \
     directly is refused.";
const ON_ACTIVITY_DESC: &str =
    "A caller drove a simulator (gesture, button, typing, app, screenshot, ios-simulator::open). \
     Observe it with args { tenant?, udid? }; each observer receives { tenant, udid, function, \
     timestamp }. Only the ios-simulator worker emits it: invoking it directly is refused.";

/// Codes the `ios-simulator::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only this tenant's events. Omit for every tenant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tenant: Option<String>,
    /// Only this simulator's events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub udid: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DeviceChangedEvent {
    pub tenant: String,
    pub udid: String,
    pub name: String,
    /// The state now; `Deleted` when the simulator is gone.
    pub state: String,
    /// The state before this change; empty when the simulator is new.
    pub previous_state: String,
    /// `added`, `removed` or `updated`.
    pub change: String,
    /// False when the boot came from a surface already showing the
    /// simulator (the console page), so live previews skip it.
    pub preview: bool,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct MediaChangedEvent {
    pub tenant: String,
    pub udid: String,
    /// Media name, `<udid>/<file>`.
    pub name: String,
    /// `screenshot` or `recording`.
    pub kind: String,
    /// `added` or `removed`.
    pub change: String,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FrameEvent {
    pub tenant: String,
    pub udid: String,
    /// Base64 JPEG.
    pub data: String,
    /// Frame pixels.
    pub width: u32,
    pub height: u32,
    /// Device pixels: the coordinate space of touch.
    pub device_width: u32,
    pub device_height: u32,
    /// Monotonic per bridge; drop anything not newer than what you show.
    pub seq: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ActivityEvent {
    pub tenant: String,
    pub udid: String,
    /// The function that drove it; `ios-simulator::open` asks to be shown.
    pub function: String,
    pub timestamp: i64,
}

/// One observer of an `ios-simulator::on-*` function, as the emitter sees
/// it. Production wraps the SDK's [`Observer`]; tests record.
pub trait EventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// delivery: never blocks the caller that produced the event.
    fn deliver(&self, payload: Value);
}

impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn deliver(&self, payload: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            match obs.call::<Value>(payload).await {
                Ok(_) => {}
                // The observer's own filters declined this event.
                Err(Error::Remote { code, .. }) if code == "FILTERS_NOT_MATCHED" => {}
                Err(e) => {
                    tracing::warn!(observer = %obs.id(), function_id = %obs.function_id(), error = %e, "observer delivery failed");
                }
            }
        });
    }
}

type Bindings = Arc<RwLock<HashMap<String, (TriggerConfig, BindingConfig)>>>;
type Observers = Arc<RwLock<HashMap<String, (Arc<dyn EventObserver>, BindingConfig)>>>;

/// The subscribers of one trigger type: its bindings and the observers of
/// its `ios-simulator::on-*` function, kept apart by id.
#[derive(Clone, Default)]
pub struct Subscribers {
    bindings: Bindings,
    observers: Observers,
}

/// A binding's config or an observer's args; null means no filter.
fn parse_filter(raw: Value) -> Result<BindingConfig, String> {
    let raw = if raw.is_null() {
        serde_json::json!({})
    } else {
        raw
    };
    serde_json::from_value(raw).map_err(|e| format!("invalid ios-simulator trigger config: {e}"))
}

impl Subscribers {
    /// Parse and file a binding. Malformed configs are refused.
    pub fn add_binding(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = parse_filter(config.config.clone())?;
        self.bindings
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), (config, filter));
        Ok(())
    }

    pub fn remove_binding(&self, id: &str) {
        self.bindings
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    /// Parse an observer's args (the trigger config) and file it. Malformed
    /// args reject the observer.
    pub fn add_observer(
        &self,
        observer: Arc<dyn EventObserver>,
        args: Value,
    ) -> Result<(), String> {
        let filter = parse_filter(args)?;
        let id = observer.id().to_string();
        self.observers
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(id, (observer, filter));
        Ok(())
    }

    pub fn remove_observer(&self, id: &str) {
        self.observers
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id);
    }

    /// The bindings and observers whose filter matches, snapshotted so no
    /// lock is held while delivering.
    pub fn matching(
        &self,
        tenant: &str,
        udid: &str,
    ) -> (Vec<TriggerConfig>, Vec<Arc<dyn EventObserver>>) {
        let bindings = self
            .bindings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|(_, filter)| matches(filter, tenant, udid))
            .map(|(binding, _)| binding.clone())
            .collect();
        let observers = self
            .observers
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|(_, filter)| matches(filter, tenant, udid))
            .map(|(observer, _)| observer.clone())
            .collect();
        (bindings, observers)
    }
}

struct BindingTable(Subscribers);

#[async_trait]
impl TriggerHandler for BindingTable {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0.add_binding(config).map_err(Error::Handler)
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0.remove_binding(&config.id);
        Ok(())
    }
}

/// Files each observer of an `ios-simulator::on-*` function in the
/// subscriber set its trigger type uses, and takes it out again when the
/// engine removes it.
struct ObserverTable(Subscribers);

#[async_trait]
impl ObserveHandler for ObserverTable {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.0
            .add_observer(Arc::new(obs.clone()), obs.args_value().clone())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), function_id = %obs.function_id(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.0.remove_observer(obs.id());
        tracing::info!(observer = %obs.id(), function_id = %obs.function_id(), "observer removed");
    }
}

/// The error every `ios-simulator::on-*` handler returns: events come only
/// from this worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the ios-simulator worker emits its events"
        ),
        stacktrace: None,
    }
}

/// Registers an observable function, owner-delivered, with args
/// [`BindingConfig`] (the trigger config) and hooks filing observers in the
/// trigger type's subscriber set. On an engine without observe it registers
/// as a plain function and its hooks never run.
fn register_observable<E>(
    iii: &IIIClient,
    function_id: &'static str,
    description: &str,
    subscribers: &Subscribers,
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
                .args_format::<BindingConfig>()
                .handler(ObserverTable(subscribers.clone())),
        ),
    );
}

pub fn matches(filter: &BindingConfig, tenant: &str, udid: &str) -> bool {
    filter.tenant.as_deref().is_none_or(|t| t == tenant)
        && filter.udid.as_deref().is_none_or(|u| u == udid)
}

pub struct Events {
    iii: Arc<IIIClient>,
    device: Subscribers,
    media: Subscribers,
    frame: Subscribers,
    activity: Subscribers,
}

impl Events {
    /// Register the trigger types and the observable function next to each
    /// one. Run before any function can emit.
    pub fn register(iii: &Arc<IIIClient>) -> Arc<Self> {
        let events = Arc::new(Self {
            iii: iii.clone(),
            device: Subscribers::default(),
            media: Subscribers::default(),
            frame: Subscribers::default(),
            activity: Subscribers::default(),
        });
        // The trigger types stay for consumers that bind them, on 0.24 and
        // 0.25 engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                DEVICE_CHANGED,
                "A simulator was added, removed, booted or shut down (by this worker, Xcode or simctl). Config: { tenant?, udid? }.",
                BindingTable(events.device.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<DeviceChangedEvent>(),
        );
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                MEDIA_CHANGED,
                "A simulator screenshot or recording was saved or deleted. Config: { tenant?, udid? }.",
                BindingTable(events.media.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<MediaChangedEvent>(),
        );
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                FRAME_EVENT,
                "A live-view frame of a simulator someone is watching (ios-simulator::watch). Config: { tenant?, udid? }.",
                BindingTable(events.frame.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<FrameEvent>(),
        );
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                ACTIVITY,
                "A caller drove a simulator (gesture, button, typing, app, screenshot, ios-simulator::open). Config: { tenant?, udid? }.",
                BindingTable(events.activity.clone()),
            )
            .trigger_request_format::<BindingConfig>()
            .call_request_format::<ActivityEvent>(),
        );
        register_observable::<DeviceChangedEvent>(
            iii,
            ON_DEVICE_CHANGED,
            ON_DEVICE_CHANGED_DESC,
            &events.device,
        );
        register_observable::<MediaChangedEvent>(
            iii,
            ON_MEDIA_CHANGED,
            ON_MEDIA_CHANGED_DESC,
            &events.media,
        );
        register_observable::<FrameEvent>(iii, ON_FRAME_EVENT, ON_FRAME_EVENT_DESC, &events.frame);
        register_observable::<ActivityEvent>(iii, ON_ACTIVITY, ON_ACTIVITY_DESC, &events.activity);
        events
    }

    pub fn device(&self, event: DeviceChangedEvent) {
        self.deliver(&self.device, &event.tenant, &event.udid, &event);
    }

    pub fn media(&self, event: MediaChangedEvent) {
        self.deliver(&self.media, &event.tenant, &event.udid, &event);
    }

    pub fn frame(&self, event: FrameEvent) {
        self.deliver(&self.frame, &event.tenant, &event.udid, &event);
    }

    pub fn activity(&self, event: ActivityEvent) {
        self.deliver(&self.activity, &event.tenant, &event.udid, &event);
    }

    fn deliver<T: Serialize>(
        &self,
        subscribers: &Subscribers,
        tenant: &str,
        udid: &str,
        event: &T,
    ) {
        let (targets, observers) = subscribers.matching(tenant, udid);
        if targets.is_empty() && observers.is_empty() {
            return;
        }
        // After the subscriber check: a frame nobody watches is never encoded.
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        for observer in observers {
            observer.deliver(payload.clone());
        }
        if targets.is_empty() {
            return;
        }
        let iii = self.iii.clone();
        tokio::spawn(async move {
            for binding in targets {
                let mut request: TriggerRequestWithMetadata = TriggerRequest {
                    function_id: binding.function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                }
                .into();
                if let Some(namespace) = &binding.namespace {
                    request = request.namespace(namespace.clone());
                }
                if let Err(e) = iii.trigger(request).await {
                    tracing::warn!(function_id = %binding.function_id, error = %e, "event delivery failed");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_narrow_by_tenant_and_udid() {
        let any = BindingConfig::default();
        assert!(matches(&any, "acme", "U1"));
        let acme = BindingConfig {
            tenant: Some("acme".into()),
            udid: None,
        };
        assert!(matches(&acme, "acme", "U1"));
        assert!(!matches(&acme, "globex", "U1"));
        let one = BindingConfig {
            tenant: Some("acme".into()),
            udid: Some("U1".into()),
        };
        assert!(!matches(&one, "acme", "U2"));
    }

    #[test]
    fn misspelled_filter_keys_are_refused() {
        assert!(
            serde_json::from_value::<BindingConfig>(serde_json::json!({ "tenat": "x" })).is_err()
        );
    }
}

#[cfg(test)]
mod observable_tests {
    use super::*;
    use serde_json::json;
    use std::sync::Mutex;

    /// An observer of an `ios-simulator::on-*` function that records its
    /// events.
    struct Recorder {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn deliver(&self, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    fn recorder(id: &str) -> Arc<Recorder> {
        Arc::new(Recorder {
            id: id.to_string(),
            seen: Mutex::new(Vec::new()),
        })
    }

    fn binding(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    /// Events over unconnected subscriber sets: binding deliveries are void
    /// triggers queued on a client that never connects.
    fn events() -> Events {
        Events {
            iii: Arc::new(IIIClient::new("ws://127.0.0.1:9")),
            device: Subscribers::default(),
            media: Subscribers::default(),
            frame: Subscribers::default(),
            activity: Subscribers::default(),
        }
    }

    const PAIRS: [(&str, &str); 4] = [
        (DEVICE_CHANGED, ON_DEVICE_CHANGED),
        (MEDIA_CHANGED, ON_MEDIA_CHANGED),
        (FRAME_EVENT, ON_FRAME_EVENT),
        (ACTIVITY, ON_ACTIVITY),
    ];

    #[test]
    fn each_on_function_refuses_direct_invocation() {
        for (trigger_type, function_id) in PAIRS {
            // Each on-* sits next to its trigger type.
            assert_eq!(
                function_id,
                trigger_type.replacen("ios-simulator::", "ios-simulator::on-", 1)
            );
            let err = refuse_invocation(function_id);
            assert!(
                matches!(&err, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{err:?}"
            );
        }
    }

    #[test]
    fn observer_args_follow_the_binding_config_rules() {
        let set = Subscribers::default();
        set.add_observer(recorder("o_null"), Value::Null).unwrap();
        set.add_observer(recorder("o_udid"), json!({ "udid": "U1" }))
            .unwrap();
        let err = set
            .add_observer(recorder("o_bad"), json!({ "tenat": "acme" }))
            .unwrap_err();
        assert!(
            err.contains("invalid ios-simulator trigger config"),
            "{err}"
        );
        let (bindings, observers) = set.matching("acme", "U1");
        assert!(bindings.is_empty(), "observers are not bindings");
        assert_eq!(observers.len(), 2);
        let (_, observers) = set.matching("acme", "U2");
        assert_eq!(observers.len(), 1);
        set.remove_observer("o_null");
        assert!(set.matching("acme", "U2").1.is_empty());
    }

    #[test]
    fn an_observer_matches_exactly_where_a_binding_matches() {
        let set = Subscribers::default();
        for (id, config) in [
            ("all", json!({})),
            ("acme", json!({ "tenant": "acme" })),
            ("u1", json!({ "tenant": "acme", "udid": "U1" })),
            ("globex", json!({ "tenant": "globex" })),
        ] {
            set.add_binding(binding(id, &format!("fn::{id}"), config.clone()))
                .unwrap();
            set.add_observer(recorder(id), config).unwrap();
        }
        for (tenant, udid) in [("acme", "U1"), ("acme", "U2"), ("globex", "U1")] {
            let (bindings, observers) = set.matching(tenant, udid);
            let mut bound: Vec<String> = bindings.iter().map(|b| b.id.clone()).collect();
            let mut observed: Vec<String> = observers.iter().map(|o| o.id().to_string()).collect();
            bound.sort();
            observed.sort();
            assert_eq!(bound, observed, "{tenant}/{udid}");
        }
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        let events = events();
        events
            .device
            .add_binding(binding("b1", "console::on-device", json!({ "udid": "U1" })))
            .unwrap();
        let matching = recorder("o1");
        let elsewhere = recorder("o2");
        let other_type = recorder("o3");
        events
            .device
            .add_observer(matching.clone(), json!({ "udid": "U1" }))
            .unwrap();
        events
            .device
            .add_observer(elsewhere.clone(), json!({ "udid": "U2" }))
            .unwrap();
        events
            .media
            .add_observer(other_type.clone(), json!({}))
            .unwrap();

        let event = DeviceChangedEvent {
            tenant: "acme".into(),
            udid: "U1".into(),
            name: "iPhone".into(),
            state: "Booted".into(),
            previous_state: "Shutdown".into(),
            change: "updated".into(),
            preview: true,
            timestamp: 7,
        };
        // The binding the event goes to, and the payload it is handed.
        let (bindings, _) = events.device.matching("acme", "U1");
        assert_eq!(bindings.len(), 1);
        assert_eq!(bindings[0].function_id, "console::on-device");
        let payload = serde_json::to_value(&event).unwrap();

        events.device(event);
        assert_eq!(*matching.seen.lock().unwrap(), vec![payload]);
        assert!(elsewhere.seen.lock().unwrap().is_empty());
        assert!(other_type.seen.lock().unwrap().is_empty());

        events.device.remove_observer("o1");
        events.device(DeviceChangedEvent {
            tenant: "acme".into(),
            udid: "U1".into(),
            name: "iPhone".into(),
            state: "Shutdown".into(),
            previous_state: "Booted".into(),
            change: "updated".into(),
            preview: true,
            timestamp: 8,
        });
        assert_eq!(matching.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn each_trigger_type_delivers_to_its_own_observers() {
        let events = events();
        let media = recorder("m");
        let frame = recorder("f");
        let activity = recorder("a");
        events.media.add_observer(media.clone(), json!({})).unwrap();
        events.frame.add_observer(frame.clone(), json!({})).unwrap();
        events
            .activity
            .add_observer(activity.clone(), json!({ "tenant": "acme" }))
            .unwrap();

        events.media(MediaChangedEvent {
            tenant: "acme".into(),
            udid: "U1".into(),
            name: "U1/shot.png".into(),
            kind: "screenshot".into(),
            change: "added".into(),
            timestamp: 1,
        });
        events.frame(FrameEvent {
            tenant: "acme".into(),
            udid: "U1".into(),
            data: "AAAA".into(),
            width: 10,
            height: 20,
            device_width: 100,
            device_height: 200,
            seq: 3,
        });
        events.activity(ActivityEvent {
            tenant: "acme".into(),
            udid: "U1".into(),
            function: "ios-simulator::open".into(),
            timestamp: 2,
        });
        events.activity(ActivityEvent {
            tenant: "globex".into(),
            udid: "U1".into(),
            function: "ios-simulator::tap".into(),
            timestamp: 3,
        });

        assert_eq!(media.seen.lock().unwrap()[0]["kind"], json!("screenshot"));
        assert_eq!(media.seen.lock().unwrap().len(), 1);
        assert_eq!(frame.seen.lock().unwrap()[0]["seq"], json!(3));
        assert_eq!(frame.seen.lock().unwrap().len(), 1);
        let seen = activity.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0]["function"], json!("ios-simulator::open"));
    }
}
