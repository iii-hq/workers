//! The custom trigger types this worker emits, the observable
//! `browser::on-*` function next to each one, and the fan-out behind them.
//! Old consumers bind a trigger type with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]. New consumers
//! observe the function (iii 0.25) with the same config as args; its
//! [`ObserveHandler`] files observers in the same subscriber set. Delivery is
//! fire-and-forget (`TriggerAction::Void`) and at-least-once for bindings,
//! `obs.call` for observers; per-binding `config` (per-observer `args`)
//! filters are evaluated here by the emitting worker, and malformed configs
//! or args are rejected at registration time. Events are routed in-process:
//! invoking an on-* function is refused with FORBIDDEN, so nobody can forge
//! one.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType, TriggerAction,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const SESSION_STARTED: &str = "browser::session-started";
pub const SESSION_STOPPED: &str = "browser::session-stopped";
pub const SESSION_UPDATED: &str = "browser::session-updated";
pub const NAVIGATED: &str = "browser::navigated";
pub const CONSOLE_EVENT: &str = "browser::console-event";
pub const NETWORK_EVENT: &str = "browser::network-event";
pub const PICKED: &str = "browser::picked";
pub const HANDOFF_REQUESTED: &str = "browser::handoff-requested";
pub const HANDOFF_RESOLVED: &str = "browser::handoff-resolved";
pub const DOWNLOAD_CHANGED: &str = "browser::download-changed";
pub const FRAME_EVENT: &str = "browser::frame-event";
pub const CHROMIUM_INSTALL_PROGRESS: &str = "browser::chromium-install-progress";

// The observable function next to each trigger type (iii 0.25).
pub const ON_SESSION_STARTED: &str = "browser::on-session-started";
pub const ON_SESSION_STOPPED: &str = "browser::on-session-stopped";
pub const ON_SESSION_UPDATED: &str = "browser::on-session-updated";
pub const ON_NAVIGATED: &str = "browser::on-navigated";
pub const ON_CONSOLE_EVENT: &str = "browser::on-console-event";
pub const ON_NETWORK_EVENT: &str = "browser::on-network-event";
pub const ON_PICKED: &str = "browser::on-picked";
pub const ON_HANDOFF_REQUESTED: &str = "browser::on-handoff-requested";
pub const ON_HANDOFF_RESOLVED: &str = "browser::on-handoff-resolved";
pub const ON_DOWNLOAD_CHANGED: &str = "browser::on-download-changed";
pub const ON_FRAME_EVENT: &str = "browser::on-frame-event";
pub const ON_CHROMIUM_INSTALL_PROGRESS: &str = "browser::on-chromium-install-progress";

/// Codes the `browser::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
    /// `browser::on-crawl-item`: the shared binding and observer cap is reached.
    pub const LIMIT_REACHED: &str = "LIMIT_REACHED";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    SessionStarted,
    SessionStopped,
    SessionUpdated,
    Navigated,
    ConsoleEvent,
    NetworkEvent,
    Picked,
    HandoffRequested,
    HandoffResolved,
    DownloadChanged,
    FrameEvent,
    ChromiumInstallProgress,
}

impl EventKind {
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => SESSION_STARTED,
            EventKind::SessionStopped => SESSION_STOPPED,
            EventKind::SessionUpdated => SESSION_UPDATED,
            EventKind::Navigated => NAVIGATED,
            EventKind::ConsoleEvent => CONSOLE_EVENT,
            EventKind::NetworkEvent => NETWORK_EVENT,
            EventKind::Picked => PICKED,
            EventKind::HandoffRequested => HANDOFF_REQUESTED,
            EventKind::HandoffResolved => HANDOFF_RESOLVED,
            EventKind::DownloadChanged => DOWNLOAD_CHANGED,
            EventKind::FrameEvent => FRAME_EVENT,
            EventKind::ChromiumInstallProgress => CHROMIUM_INSTALL_PROGRESS,
        }
    }

    /// The observable function next to the trigger type.
    pub fn observable_function(&self) -> &'static str {
        match self {
            EventKind::SessionStarted => ON_SESSION_STARTED,
            EventKind::SessionStopped => ON_SESSION_STOPPED,
            EventKind::SessionUpdated => ON_SESSION_UPDATED,
            EventKind::Navigated => ON_NAVIGATED,
            EventKind::ConsoleEvent => ON_CONSOLE_EVENT,
            EventKind::NetworkEvent => ON_NETWORK_EVENT,
            EventKind::Picked => ON_PICKED,
            EventKind::HandoffRequested => ON_HANDOFF_REQUESTED,
            EventKind::HandoffResolved => ON_HANDOFF_RESOLVED,
            EventKind::DownloadChanged => ON_DOWNLOAD_CHANGED,
            EventKind::FrameEvent => ON_FRAME_EVENT,
            EventKind::ChromiumInstallProgress => ON_CHROMIUM_INSTALL_PROGRESS,
        }
    }

    pub fn all() -> [EventKind; 12] {
        [
            EventKind::SessionStarted,
            EventKind::SessionStopped,
            EventKind::SessionUpdated,
            EventKind::Navigated,
            EventKind::ConsoleEvent,
            EventKind::NetworkEvent,
            EventKind::Picked,
            EventKind::HandoffRequested,
            EventKind::HandoffResolved,
            EventKind::DownloadChanged,
            EventKind::FrameEvent,
            EventKind::ChromiumInstallProgress,
        ]
    }
}

/// Config accepted by every `browser::*` trigger binding. The only filter is
/// an optional session-id equality match; unknown fields fail at registration
/// so a misspelled filter key fails loudly instead of silently receiving
/// nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this browser session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

pub fn binding_matches(filter: &BindingConfig, session_id: &str) -> bool {
    match &filter.session_id {
        Some(want) => want == session_id,
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Event payloads (what subscribers receive)
// ---------------------------------------------------------------------------

/// `browser::session-started` — a Chromium session is up and ready.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStartedEvent {
    pub session_id: String,
    pub url: String,
    pub headless: bool,
    /// False when the opener asked for no live preview (`preview: false`
    /// on `sessions::start`); consoles then show no thumbnail for it.
    pub preview: bool,
    pub timestamp: i64,
}

/// `browser::session-stopped` — a tab closed for good; see `reason`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionStoppedEvent {
    pub session_id: String,
    /// One of `stopped`, `idle`, `expired`, `crashed`.
    pub reason: String,
    pub timestamp: i64,
}

/// `browser::session-updated` — a tab woke up (`active: true`, its page is
/// open again), went to sleep (`active: false`, page closed, tab kept), or
/// its live page's title changed (`active: true`; reported by an in-page
/// title watcher the page cannot see). The tab strip re-reads
/// `browser::sessions::list` on this; URL moves arrive as
/// `browser::navigated`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SessionUpdatedEvent {
    pub session_id: String,
    pub active: bool,
    pub url: String,
    pub title: String,
    pub timestamp: i64,
}

/// `browser::navigated` — the session's page committed a navigation.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NavigatedEvent {
    pub session_id: String,
    pub url: String,
    pub timestamp: i64,
}

/// `browser::download-changed` — a download in the session started,
/// progressed, or finished. The durable record is `browser::downloads::list`;
/// this is the nudge to re-list.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DownloadChangedEvent {
    pub session_id: String,
    pub timestamp: i64,
}

/// `browser::frame-event` — one live screencast frame of a watched tab, the
/// picture the console viewport paints. Internal console-UI plumbing and
/// high-volume (up to 30 a second, base64 JPEG): bind with a `session_id`
/// filter only while a viewer is on screen. `browser::frame` reads the newest
/// frame once for the first paint.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct FrameEventPayload {
    pub session_id: String,
    /// Base64 JPEG.
    pub frame: String,
    /// Page-viewport size the frame maps to (input coordinate space).
    pub width: u32,
    pub height: u32,
    /// Monotonic per-tab cursor; a viewer ignores frames older than the last
    /// one it painted.
    pub frame_seq: u64,
    pub timestamp: i64,
}

/// `browser::console-event` — one console/log/exception entry was captured.
/// High-volume: bind with a `session_id` filter and debounce on the consumer
/// side; the durable record is the ring buffer behind `browser::console::read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConsoleEventPayload {
    pub session_id: String,
    pub entry: crate::session::ConsoleEntry,
}

/// `browser::network-event` — one network request was captured (completed or
/// failed). Symmetric with `browser::console-event`; the durable record is the
/// ring buffer behind `browser::network::read`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct NetworkEventPayload {
    pub session_id: String,
    pub entry: crate::session::NetworkEntry,
}

/// `browser::picked` — the human picked an element in inspect mode
/// (`browser::pick::start`). Carries everything a chat composer needs to
/// describe the element, plus a `ref` that `browser::act` and
/// `browser::evaluate` accept directly.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickedEvent {
    pub session_id: String,
    pub element: PickedElement,
    pub timestamp: i64,
}

/// `browser::handoff-requested` — a session is paused waiting for a human to
/// complete a step (CAPTCHA, 2FA, payment). The console surfaces this beside
/// the live viewport; resolve it with `browser::handoff::confirm` (or the
/// human clicks the in-page continue control).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffRequestedEvent {
    pub session_id: String,
    pub handoff_id: String,
    /// What the human must do before the paused call continues.
    pub instructions: String,
    pub timestamp: i64,
}

/// `browser::handoff-resolved` — a paused handoff finished, however it
/// finished. The console clears its banner on this; `via` is `in_page`,
/// `confirm_call`, `timeout`, or `cancelled`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HandoffResolvedEvent {
    pub session_id: String,
    pub handoff_id: String,
    pub via: String,
    pub timestamp: i64,
}

/// `browser::chromium-install-progress` carries
/// [`crate::chromium::install::InstallProgress`]: `{ job_id, phase, version?,
/// bytes_done, bytes_total?, path?, error?, hint?, timestamp }`. It is not a
/// session event, so it reaches only bindings without a `session_id` filter.
pub use crate::chromium::install::InstallProgress as ChromiumInstallProgressEvent;

/// The element payload inside `browser::picked`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct PickedElement {
    /// Stable ref usable with `browser::act` / `browser::evaluate` until the
    /// next navigation.
    pub r#ref: String,
    /// Lowercase tag name (`button`, `input`, …).
    pub tag: String,
    /// Flattened `name="value"` attribute pairs.
    pub attributes: HashMap<String, String>,
    /// Outer HTML, truncated.
    pub outer_html: String,
    /// Trimmed innerText, truncated.
    pub text: String,
    /// Viewport-space bounding box.
    pub bounds: Bounds,
    /// Page URL at pick time.
    pub url: String,
    /// Most recent console errors at pick time (newest last).
    pub console_recent: Vec<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct Bounds {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

// ---------------------------------------------------------------------------
// Subscriber registry + trigger type registration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Binding {
    pub id: String,
    pub function_id: String,
    pub filter: BindingConfig,
}

/// One observer of a `browser::on-*` function with its parsed args.
#[derive(Clone)]
pub struct ObserverBinding {
    pub observer: Arc<dyn EventObserver>,
    pub filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type: its bindings and the
/// observers of its `browser::on-*` function, kept apart by id.
#[derive(Clone)]
pub struct SubscriberSet {
    kind: EventKind,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverBinding>>>,
}

impl SubscriberSet {
    pub fn new(kind: EventKind) -> Self {
        Self {
            kind,
            inner: Arc::new(Mutex::new(HashMap::new())),
            observers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// A binding's config or an observer's args; null means no filter.
    fn parse_filter(&self, raw: Value) -> Result<BindingConfig, String> {
        let raw = if raw.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            raw
        };
        serde_json::from_value(raw)
            .map_err(|e| format!("invalid {} config: {e}", self.kind.trigger_type()))
    }

    /// Parse + insert a binding. Rejects malformed configs at registration.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = self.parse_filter(config.config.clone())?;
        let binding = Binding {
            id: config.id.clone(),
            function_id: config.function_id,
            filter,
        };
        self.lock().insert(config.id, binding);
        Ok(())
    }

    pub fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    /// Parse an observer's args (the trigger config) and file it. Malformed
    /// args reject the observer.
    pub fn add_observer(
        &self,
        observer: Arc<dyn EventObserver>,
        args: Value,
    ) -> Result<(), String> {
        let filter = self.parse_filter(args)?;
        let id = observer.id().to_string();
        self.observers_lock()
            .insert(id, ObserverBinding { observer, filter });
        Ok(())
    }

    pub fn remove_observer(&self, id: &str) {
        self.observers_lock().remove(id);
    }

    /// Snapshot so the mutex is never held across awaits.
    pub fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    /// Snapshot of the observers, same rule.
    pub fn observer_snapshot(&self) -> Vec<ObserverBinding> {
        self.observers_lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn observers_lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ObserverBinding>> {
        self.observers.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// The subscriber sets, one per trigger type.
#[derive(Clone)]
pub struct TriggerSets {
    inner: HashMap<&'static str, SubscriberSet>,
}

impl TriggerSets {
    pub fn new() -> Self {
        let mut inner = HashMap::new();
        for kind in EventKind::all() {
            inner.insert(kind.trigger_type(), SubscriberSet::new(kind));
        }
        Self { inner }
    }

    pub fn for_kind(&self, kind: EventKind) -> &SubscriberSet {
        self.inner
            .get(kind.trigger_type())
            .expect("all kinds are populated at construction")
    }
}

impl Default for TriggerSets {
    fn default() -> Self {
        Self::new()
    }
}

struct BrowserTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for BrowserTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(id = %id, function_id = %function_id, "trigger subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(id = %config.id, "trigger subscription unregistered");
        self.set.remove(&config.id);
        Ok(())
    }
}

/// One observer of a `browser::on-*` function, as the emitter sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
#[async_trait]
pub trait EventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event to the observer, fire-and-forget like a binding's
    /// delivery: never blocks the pump that produced the event.
    async fn deliver(&self, payload: Value);
    /// Delivery that returns once the observer's target took the event (or
    /// after 5 s), for producers that want back-pressure (the frame pump).
    async fn deliver_awaited(&self, payload: Value) {
        self.deliver(payload).await;
    }
}

/// How long an awaited observer delivery may take, as for bindings.
const AWAITED_DELIVERY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

fn log_observer_failure(obs: &Observer, error: Error) {
    match error {
        // The observer's own filters declined this event.
        Error::Remote { code, .. } if code == "FILTERS_NOT_MATCHED" => {}
        e => tracing::debug!(
            observer = %obs.id(),
            function_id = %obs.function_id(),
            error = %e,
            "observer delivery failed"
        ),
    }
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, payload: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            if let Err(e) = obs.call::<Value>(payload).await {
                log_observer_failure(&obs, e);
            }
        });
    }

    async fn deliver_awaited(&self, payload: Value) {
        if let Err(e) = self
            .call_with_timeout::<Value>(payload, AWAITED_DELIVERY_TIMEOUT)
            .await
        {
            log_observer_failure(self, e);
        }
    }
}

/// Files each observer of a `browser::on-*` function in the subscriber set
/// its trigger type uses, and takes it out again when the engine removes it.
struct BrowserObserveHandler {
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for BrowserObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.set
            .add_observer(Arc::new(obs.clone()), obs.args_value().clone())
            .map_err(|e| ObserveRejection::new(codes::INVALID_ARGS, e))?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), function_id = %obs.function_id(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
        tracing::info!(observer = %obs.id(), function_id = %obs.function_id(), "observer removed");
    }
}

/// The error every `browser::on-*` handler returns: events come only from
/// this worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the browser worker emits its events"
        ),
        stacktrace: None,
    }
}

/// The description of `kind`'s observable function, from its trigger type's.
pub fn observable_description(kind: EventKind, trigger_description: &str) -> String {
    let args = match kind {
        EventKind::ChromiumInstallProgress => "{} (not a session event: pass no session_id)",
        _ => "{ session_id? }",
    };
    format!(
        "{trigger_description} Observe it with args {args}; each observer receives the payload a \
         {} binding receives. Only the browser worker emits it: invoking it directly is refused.",
        kind.trigger_type()
    )
}

/// Registers `kind`'s observable function, owner-delivered, with args
/// [`BindingConfig`] (the trigger config) and hooks filing observers in the
/// trigger type's subscriber set. On an engine without observe it registers
/// as a plain function and its hooks never run.
fn register_observable<E>(iii: &IIIClient, kind: EventKind, description: String, sets: &TriggerSets)
where
    E: DeserializeOwned + JsonSchema + Send + 'static,
{
    let function_id = kind.observable_function();
    iii.register_function(
        function_id,
        RegisterFunction::new(move |_event: E| -> Result<(), Error> {
            Err(refuse_invocation(function_id))
        })
        .description(description)
        .observe(
            Observe::owner()
                .args_format::<BindingConfig>()
                .handler(BrowserObserveHandler {
                    set: sets.for_kind(kind).clone(),
                }),
        ),
    );
    tracing::info!(function_id, "registered observable function");
}

/// [`register_observable`] with `kind`'s event payload as the request format.
fn register_observable_for(
    iii: &IIIClient,
    kind: EventKind,
    description: String,
    sets: &TriggerSets,
) {
    match kind {
        EventKind::SessionStarted => {
            register_observable::<SessionStartedEvent>(iii, kind, description, sets)
        }
        EventKind::SessionStopped => {
            register_observable::<SessionStoppedEvent>(iii, kind, description, sets)
        }
        EventKind::SessionUpdated => {
            register_observable::<SessionUpdatedEvent>(iii, kind, description, sets)
        }
        EventKind::Navigated => register_observable::<NavigatedEvent>(iii, kind, description, sets),
        EventKind::ConsoleEvent => {
            register_observable::<ConsoleEventPayload>(iii, kind, description, sets)
        }
        EventKind::NetworkEvent => {
            register_observable::<NetworkEventPayload>(iii, kind, description, sets)
        }
        EventKind::Picked => register_observable::<PickedEvent>(iii, kind, description, sets),
        EventKind::HandoffRequested => {
            register_observable::<HandoffRequestedEvent>(iii, kind, description, sets)
        }
        EventKind::HandoffResolved => {
            register_observable::<HandoffResolvedEvent>(iii, kind, description, sets)
        }
        EventKind::DownloadChanged => {
            register_observable::<DownloadChangedEvent>(iii, kind, description, sets)
        }
        EventKind::FrameEvent => {
            register_observable::<FrameEventPayload>(iii, kind, description, sets)
        }
        EventKind::ChromiumInstallProgress => {
            register_observable::<ChromiumInstallProgressEvent>(iii, kind, description, sets)
        }
    }
}

/// Register the custom trigger types with the engine, and the observable
/// `browser::on-*` function next to each one. Must run before
/// `functions::register_all` so handlers can capture the subscriber sets.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();
    let descriptions: [(EventKind, &str); 12] = [
        (
            EventKind::SessionStarted,
            "A browser tab opened and is ready.",
        ),
        (
            EventKind::SessionStopped,
            "A browser tab closed for good (stopped, idle, expired, or crashed).",
        ),
        (
            EventKind::SessionUpdated,
            "A browser tab woke up (page open again), went to sleep (page closed, tab kept), or its \
             page's title changed.",
        ),
        (
            EventKind::Navigated,
            "The session's page committed a navigation.",
        ),
        (
            EventKind::ConsoleEvent,
            "A console/log/exception entry was captured on a session's page.",
        ),
        (
            EventKind::NetworkEvent,
            "A network request was captured (completed or failed) on a session's page.",
        ),
        (
            EventKind::Picked,
            "The human picked an element in inspect mode.",
        ),
        (
            EventKind::HandoffRequested,
            "A session is paused waiting for a human to complete a step (CAPTCHA, 2FA, payment).",
        ),
        (
            EventKind::HandoffResolved,
            "A paused handoff finished (confirmed in page, by call, or timed out).",
        ),
        (
            EventKind::DownloadChanged,
            "A download started, progressed, or finished in a session.",
        ),
        (
            EventKind::FrameEvent,
            "Internal: a live screencast frame of a watched tab (console viewport plumbing, high volume).",
        ),
        (
            EventKind::ChromiumInstallProgress,
            "A browser::chromium::install job moved: a new phase (resolving, downloading, \
             extracting, verifying, done, failed) or about 1% more bytes. Not a session event: \
             bind without a session_id filter.",
        ),
    ];
    for (kind, description) in descriptions {
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                kind.trigger_type(),
                description,
                BrowserTriggerHandler {
                    set: sets.for_kind(kind).clone(),
                },
            )
            .trigger_request_format::<BindingConfig>(),
        );
        tracing::info!(
            trigger_type = kind.trigger_type(),
            "registered trigger type"
        );
        register_observable_for(iii, kind, observable_description(kind, description), &sets);
    }
    sets
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// How a matched event reaches a subscriber. Production delivers over the
/// bus; tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    /// Fire-and-forget delivery.
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value);
    /// Delivery that returns once the bus accepted the event, for producers
    /// that want back-pressure (the frame pump). Defaults to `deliver`.
    async fn deliver_awaited(&self, trigger_type: &str, function_id: &str, payload: Value) {
        self.deliver(trigger_type, function_id, payload).await;
    }
}

/// Fire-and-forget bus delivery so the page-event pump that produced the
/// event is never blocked on subscriber latency. Failures are logged and
/// swallowed.
pub struct IiiDeliverer {
    iii: Arc<IIIClient>,
}

impl IiiDeliverer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl EventDeliverer for IiiDeliverer {
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
        let iii = self.iii.clone();
        let trigger_type = trigger_type.to_string();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let res = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload,
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await;
            if let Err(e) = res {
                tracing::warn!(trigger_type, function_id, error = %e, "event fan-out failed");
            }
        });
    }

    async fn deliver_awaited(&self, trigger_type: &str, function_id: &str, payload: Value) {
        let res = self
            .iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: Some(TriggerAction::Void),
                timeout_ms: Some(5_000),
            })
            .await;
        if let Err(e) = res {
            tracing::debug!(trigger_type, function_id, error = %e, "event delivery failed");
        }
    }
}

/// Evaluates each binding's session filter against each event and delivers
/// the payload to every match.
pub struct Emitter {
    sets: TriggerSets,
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { sets, deliverer }
    }

    pub async fn emit<T: Serialize>(&self, kind: EventKind, session_id: &str, payload: &T) {
        self.emit_inner(kind, session_id, payload, false).await;
    }

    /// `emit`, returning once every matching binding accepted the event, so
    /// the caller is paced by the bus instead of flooding it.
    pub async fn emit_awaited<T: Serialize>(&self, kind: EventKind, session_id: &str, payload: &T) {
        self.emit_inner(kind, session_id, payload, true).await;
    }

    /// Whether anyone is bound to `kind` for this session — lets a producer
    /// skip work (a JPEG decode, say) nobody would see.
    pub fn has_subscribers(&self, kind: EventKind, session_id: &str) -> bool {
        let set = self.sets.for_kind(kind);
        set.snapshot()
            .iter()
            .any(|b| binding_matches(&b.filter, session_id))
            || set
                .observer_snapshot()
                .iter()
                .any(|o| binding_matches(&o.filter, session_id))
    }

    async fn emit_inner<T: Serialize>(
        &self,
        kind: EventKind,
        session_id: &str,
        payload: &T,
        awaited: bool,
    ) {
        let set = self.sets.for_kind(kind);
        // Filter before serializing: on the high-volume console-event pump,
        // a session with no matching binding or observer must not pay the
        // payload serialization cost (a console panel open on session A means
        // every session-B log line would otherwise serialize for nothing).
        let bindings: Vec<Binding> = set
            .snapshot()
            .into_iter()
            .filter(|b| binding_matches(&b.filter, session_id))
            .collect();
        let observers: Vec<ObserverBinding> = set
            .observer_snapshot()
            .into_iter()
            .filter(|o| binding_matches(&o.filter, session_id))
            .collect();
        if bindings.is_empty() && observers.is_empty() {
            return;
        }
        let payload = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event payload failed to serialize");
                return;
            }
        };
        for binding in bindings {
            if awaited {
                self.deliverer
                    .deliver_awaited(kind.trigger_type(), &binding.function_id, payload.clone())
                    .await;
            } else {
                self.deliverer
                    .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                    .await;
            }
        }
        for observer in observers {
            if awaited {
                observer.observer.deliver_awaited(payload.clone()).await;
            } else {
                observer.observer.deliver(payload.clone()).await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trigger_config(config: Value) -> TriggerConfig {
        TriggerConfig {
            id: "t1".to_string(),
            function_id: "console::on-event".to_string(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn empty_filter_matches_everything() {
        assert!(binding_matches(&BindingConfig::default(), "b1"));
    }

    #[test]
    fn session_filter_is_equality() {
        let filter = BindingConfig {
            session_id: Some("b1".to_string()),
        };
        assert!(binding_matches(&filter, "b1"));
        assert!(!binding_matches(&filter, "b2"));
    }

    #[test]
    fn add_rejects_unknown_filter_keys() {
        let set = SubscriberSet::new(EventKind::ConsoleEvent);
        let err = set
            .add(trigger_config(json!({ "sesion_id": "b1" })))
            .unwrap_err();
        assert!(err.contains("browser::console-event"), "{err}");
    }

    #[test]
    fn add_accepts_null_config() {
        let set = SubscriberSet::new(EventKind::Picked);
        set.add(trigger_config(Value::Null)).unwrap();
        assert_eq!(set.snapshot().len(), 1);
    }

    #[tokio::test]
    async fn emit_delivers_only_to_matching_bindings() {
        struct Recorder(Mutex<Vec<(String, String)>>);

        #[async_trait]
        impl EventDeliverer for Recorder {
            async fn deliver(&self, trigger_type: &str, function_id: &str, _payload: Value) {
                self.0
                    .lock()
                    .unwrap()
                    .push((trigger_type.to_string(), function_id.to_string()));
            }
        }

        let sets = TriggerSets::new();
        let set = sets.for_kind(EventKind::Navigated);
        set.add(TriggerConfig {
            id: "match".to_string(),
            function_id: "fn::match".to_string(),
            config: json!({ "session_id": "b1" }),
            metadata: None,
            namespace: None,
        })
        .unwrap();
        set.add(TriggerConfig {
            id: "other".to_string(),
            function_id: "fn::other".to_string(),
            config: json!({ "session_id": "b2" }),
            metadata: None,
            namespace: None,
        })
        .unwrap();

        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets, recorder.clone());
        emitter
            .emit(
                EventKind::Navigated,
                "b1",
                &NavigatedEvent {
                    session_id: "b1".to_string(),
                    url: "http://localhost:3000".to_string(),
                    timestamp: 0,
                },
            )
            .await;

        let seen = recorder.0.lock().unwrap().clone();
        assert_eq!(seen, vec![(NAVIGATED.to_string(), "fn::match".to_string())]);
    }

    /// A bus deliverer that records `(trigger_type, function_id, payload)`.
    struct PayloadRecorder(Mutex<Vec<(String, String, Value)>>);

    #[async_trait]
    impl EventDeliverer for PayloadRecorder {
        async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
            self.0.lock().unwrap().push((
                trigger_type.to_string(),
                function_id.to_string(),
                payload,
            ));
        }
    }

    /// An observer of a `browser::on-*` function that records what it is
    /// handed, and through which path.
    struct Recording {
        id: String,
        received: Mutex<Vec<(bool, Value)>>,
    }

    impl Recording {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                received: Mutex::new(Vec::new()),
            })
        }

        fn payloads(&self) -> Vec<Value> {
            self.received
                .lock()
                .unwrap()
                .iter()
                .map(|(_, p)| p.clone())
                .collect()
        }

        fn awaited(&self) -> Vec<bool> {
            self.received
                .lock()
                .unwrap()
                .iter()
                .map(|(a, _)| *a)
                .collect()
        }
    }

    #[async_trait]
    impl EventObserver for Recording {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) {
            self.received.lock().unwrap().push((false, payload));
        }

        async fn deliver_awaited(&self, payload: Value) {
            self.received.lock().unwrap().push((true, payload));
        }
    }

    #[test]
    fn every_on_function_refuses_direct_invocation() {
        let mut ids = std::collections::HashSet::new();
        for kind in EventKind::all() {
            let function_id = kind.observable_function();
            let suffix = kind.trigger_type().trim_start_matches("browser::");
            assert_eq!(function_id, format!("browser::on-{suffix}"));
            assert!(ids.insert(function_id), "duplicate {function_id}");
            let error = refuse_invocation(function_id);
            assert!(
                matches!(&error, Error::Remote { code, message, .. }
                    if code == codes::FORBIDDEN && message.contains(function_id)),
                "{error:?}"
            );
            let description = observable_description(kind, "desc.");
            assert!(description.starts_with("desc."), "{description}");
            assert!(description.contains(kind.trigger_type()), "{description}");
        }
        assert_eq!(ids.len(), 12);
        assert_eq!(
            EventKind::SessionStarted.observable_function(),
            "browser::on-session-started"
        );
        assert_eq!(
            EventKind::ChromiumInstallProgress.observable_function(),
            "browser::on-chromium-install-progress"
        );
    }

    #[test]
    fn observer_args_are_the_binding_config() {
        let set = SubscriberSet::new(EventKind::ConsoleEvent);
        let err = set
            .add_observer(Recording::new("o"), json!({ "sesion_id": "b1" }))
            .unwrap_err();
        assert!(err.contains("browser::console-event"), "{err}");
        assert!(set.observer_snapshot().is_empty());
        set.add_observer(Recording::new("o"), Value::Null).unwrap();
        // Re-adding an id replaces it; bindings are a separate table.
        set.add_observer(Recording::new("o"), json!({ "session_id": "b1" }))
            .unwrap();
        assert_eq!(set.observer_snapshot().len(), 1);
        assert_eq!(
            set.observer_snapshot()[0].filter.session_id.as_deref(),
            Some("b1")
        );
        assert!(set.snapshot().is_empty());
        set.remove_observer("o");
        assert!(set.observer_snapshot().is_empty());
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        let sets = TriggerSets::new();
        let set = sets.for_kind(EventKind::Navigated);
        set.add(TriggerConfig {
            id: "b-1".to_string(),
            function_id: "fn::match".to_string(),
            config: json!({ "session_id": "b1" }),
            metadata: None,
            namespace: None,
        })
        .unwrap();
        let same = Recording::new("o-same");
        let other = Recording::new("o-other");
        let all = Recording::new("o-all");
        set.add_observer(same.clone(), json!({ "session_id": "b1" }))
            .unwrap();
        set.add_observer(other.clone(), json!({ "session_id": "b2" }))
            .unwrap();
        set.add_observer(all.clone(), json!({})).unwrap();
        // Another kind's observer sees none of it.
        let picked = Recording::new("o-picked");
        sets.for_kind(EventKind::Picked)
            .add_observer(picked.clone(), json!({}))
            .unwrap();

        let recorder = Arc::new(PayloadRecorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets.clone(), recorder.clone());
        let event = NavigatedEvent {
            session_id: "b1".to_string(),
            url: "http://localhost:3000".to_string(),
            timestamp: 7,
        };
        emitter.emit(EventKind::Navigated, "b1", &event).await;

        let bound = recorder.0.lock().unwrap().clone();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].0, NAVIGATED);
        assert_eq!(bound[0].1, "fn::match");
        assert_eq!(same.payloads(), vec![bound[0].2.clone()]);
        assert_eq!(all.payloads(), vec![bound[0].2.clone()]);
        assert_eq!(bound[0].2, serde_json::to_value(&event).unwrap());
        assert!(other.payloads().is_empty(), "filtered like its binding");
        assert!(picked.payloads().is_empty());
        assert_eq!(same.awaited(), vec![false]);

        // The awaited path (frame pump) reaches observers through theirs.
        emitter
            .emit_awaited(EventKind::Navigated, "b1", &event)
            .await;
        assert_eq!(same.awaited(), vec![false, true]);

        // Removing the observer stops delivery; the binding stays.
        set.remove_observer("o-same");
        emitter.emit(EventKind::Navigated, "b1", &event).await;
        assert_eq!(same.payloads().len(), 2);
        assert_eq!(recorder.0.lock().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn an_observer_alone_counts_as_a_subscriber() {
        let sets = TriggerSets::new();
        let recorder = Arc::new(PayloadRecorder(Mutex::new(Vec::new())));
        let emitter = Emitter::new(sets.clone(), recorder.clone());
        assert!(!emitter.has_subscribers(EventKind::FrameEvent, "b1"));
        let observer = Recording::new("o");
        sets.for_kind(EventKind::FrameEvent)
            .add_observer(observer.clone(), json!({ "session_id": "b1" }))
            .unwrap();
        assert!(emitter.has_subscribers(EventKind::FrameEvent, "b1"));
        assert!(!emitter.has_subscribers(EventKind::FrameEvent, "b2"));

        emitter
            .emit(EventKind::Picked, "b1", &json!({ "session_id": "b1" }))
            .await;
        assert!(observer.payloads().is_empty());
        assert!(recorder.0.lock().unwrap().is_empty());
    }
}
