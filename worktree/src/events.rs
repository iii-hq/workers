//! The six custom trigger types this worker emits, the observable
//! `worktree::on-*` function next to each one, and the fan-out behind them.
//! Old consumers bind a trigger type with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]s. New consumers
//! observe the function (iii 0.25) with the same config as args; its
//! [`ObserveHandler`] files observers in the same subscriber set. Delivery is
//! fire-and-forget and at-least-once for bindings (`TriggerAction::Void`),
//! fire-and-forget `obs.call` for observers; per-binding `config` (and
//! per-observer args) filters are evaluated here by the emitting worker, and
//! malformed configs or args are rejected at registration time. Events are
//! routed in-process: invoking an on-* function is refused with FORBIDDEN, so
//! nobody can forge one.

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

pub const CREATED: &str = "worktree::created";
pub const CLAIMED: &str = "worktree::claimed";
pub const RELEASED: &str = "worktree::released";
pub const REMOVED: &str = "worktree::removed";
pub const LANDED: &str = "worktree::landed";
pub const LAND_BLOCKED: &str = "worktree::land-blocked";

pub const ON_CREATED: &str = "worktree::on-created";
pub const ON_CLAIMED: &str = "worktree::on-claimed";
pub const ON_RELEASED: &str = "worktree::on-released";
pub const ON_REMOVED: &str = "worktree::on-removed";
pub const ON_LANDED: &str = "worktree::on-landed";
pub const ON_LAND_BLOCKED: &str = "worktree::on-land-blocked";

pub const ON_CREATED_DESC: &str =
    "A new managed worktree exists. Observe it with args { repo_path?, worktree_id?, \
     session_id? }; each observer receives { worktree_id, repo_path, path, branch, base_ref, \
     base_sha, session_id?, timestamp }. Only the worktree worker emits it: invoking it \
     directly is refused.";
pub const ON_CLAIMED_DESC: &str =
    "A session took ownership of a worktree. Observe it with args { repo_path?, worktree_id?, \
     session_id? }; each observer receives { worktree_id, repo_path, session_id, \
     previous_session_id?, timestamp }. Only the worktree worker emits it: invoking it \
     directly is refused.";
pub const ON_RELEASED_DESC: &str =
    "A worktree claim was released. Observe it with args { repo_path?, worktree_id?, \
     session_id? }; each observer receives { worktree_id, repo_path, session_id, timestamp }. \
     Only the worktree worker emits it: invoking it directly is refused.";
pub const ON_REMOVED_DESC: &str =
    "A managed worktree was removed (remove or prune). Observe it with args { repo_path?, \
     worktree_id?, session_id? }; each observer receives { worktree_id, repo_path, branch, \
     branch_deleted, timestamp }. Only the worktree worker emits it: invoking it directly is \
     refused.";
pub const ON_LANDED_DESC: &str =
    "A land completed; the target branch was fast-forwarded. Observe it with args { \
     repo_path?, worktree_id?, session_id? }; each observer receives { worktree_id, repo_path, \
     branch, target_branch, merged_sha, session_id?, timestamp }. Only the worktree worker \
     emits it: invoking it directly is refused.";
pub const ON_LAND_BLOCKED_DESC: &str =
    "A land ended blocked (conflict, failed tests, dirty tree, moved target). Observe it with \
     args { repo_path?, worktree_id?, session_id? }; each observer receives { worktree_id, \
     repo_path, target_branch, reason, code, conflict_files?, exit_code?, session_id?, \
     timestamp }. Only the worktree worker emits it: invoking it directly is refused.";

/// Codes the `worktree::on-*` functions refuse with.
pub mod codes {
    /// Invoking an on-* function directly.
    pub const FORBIDDEN: &str = "FORBIDDEN";
    /// An observer's args the trigger type would refuse as a config.
    pub const INVALID_ARGS: &str = "INVALID_ARGS";
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventKind {
    Created,
    Claimed,
    Released,
    Removed,
    Landed,
    LandBlocked,
}

impl EventKind {
    pub fn trigger_type(&self) -> &'static str {
        match self {
            EventKind::Created => CREATED,
            EventKind::Claimed => CLAIMED,
            EventKind::Released => RELEASED,
            EventKind::Removed => REMOVED,
            EventKind::Landed => LANDED,
            EventKind::LandBlocked => LAND_BLOCKED,
        }
    }

    /// The observable function next to the trigger type.
    pub fn observable_function(&self) -> &'static str {
        match self {
            EventKind::Created => ON_CREATED,
            EventKind::Claimed => ON_CLAIMED,
            EventKind::Released => ON_RELEASED,
            EventKind::Removed => ON_REMOVED,
            EventKind::Landed => ON_LANDED,
            EventKind::LandBlocked => ON_LAND_BLOCKED,
        }
    }

    pub fn all() -> [EventKind; 6] {
        [
            EventKind::Created,
            EventKind::Claimed,
            EventKind::Released,
            EventKind::Removed,
            EventKind::Landed,
            EventKind::LandBlocked,
        ]
    }
}

/// Config accepted by every `worktree::*` trigger binding. All filters are
/// optional equality matches; unknown fields fail at registration so a
/// misspelled filter key fails loudly instead of silently receiving nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this repository path.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repo_path: Option<String>,
    /// Only deliver events for this worktree.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree_id: Option<String>,
    /// Only deliver events for worktrees claimed by this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
}

/// The event-side facts a filter is evaluated against.
#[derive(Debug, Clone, Copy)]
pub struct EventCtx<'a> {
    pub repo_path: &'a str,
    pub worktree_id: &'a str,
    pub session_id: Option<&'a str>,
}

pub fn binding_matches(filter: &BindingConfig, ctx: &EventCtx<'_>) -> bool {
    if let Some(want) = &filter.repo_path {
        if want != ctx.repo_path {
            return false;
        }
    }
    if let Some(want) = &filter.worktree_id {
        if want != ctx.worktree_id {
            return false;
        }
    }
    if let Some(want) = &filter.session_id {
        match ctx.session_id {
            Some(sid) if sid == want => {}
            _ => return false,
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Event payloads (what subscribers receive)
// ---------------------------------------------------------------------------

/// `worktree::created` — a new managed worktree exists.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CreatedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub path: String,
    pub branch: String,
    pub base_ref: String,
    pub base_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub timestamp: i64,
}

/// `worktree::claimed` — a session took ownership.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_session_id: Option<String>,
    pub timestamp: i64,
}

/// `worktree::released` — a claim was released.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReleasedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub session_id: String,
    pub timestamp: i64,
}

/// `worktree::removed` — a managed worktree was removed (remove or prune).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemovedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub branch: String,
    pub branch_deleted: bool,
    pub timestamp: i64,
}

/// `worktree::landed` — a land completed; the target branch was advanced.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LandedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub branch: String,
    pub target_branch: String,
    pub merged_sha: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub timestamp: i64,
}

/// `worktree::land-blocked` — a land ended blocked; see `reason` and `code`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LandBlockedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub target_branch: String,
    /// One of `rebase_conflict`, `test_failed`, `dirty`, `worktree_path_missing`,
    /// `rebase_in_progress`, `target_ref_not_found`, `target_moved_exhausted`,
    /// `target_checked_out_dirty`.
    pub reason: String,
    /// The stable `W###` code for the block.
    pub code: String,
    /// Unmerged paths, on `rebase_conflict`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_files: Option<Vec<String>>,
    /// Test gate exit code, on `test_failed`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub timestamp: i64,
}

// ---------------------------------------------------------------------------
// Subscriber registry + trigger type registration
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct Binding {
    pub id: String,
    pub function_id: String,
    pub namespace: Option<String>,
    pub filter: BindingConfig,
}

/// One observer of a `worktree::on-*` function with its parsed args.
#[derive(Clone)]
pub struct ObserverBinding {
    pub observer: Arc<dyn EventObserver>,
    pub filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type: its bindings and the
/// observers of its `worktree::on-*` function, kept apart by id.
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
            namespace: config.namespace,
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

/// The six subscriber sets, one per trigger type.
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

struct WorktreeTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for WorktreeTriggerHandler {
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

/// One observer of a `worktree::on-*` function, as the emitter sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
#[async_trait]
pub trait EventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// delivery: never blocks the mutation that produced the event.
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
                    tracing::warn!(observer = %obs.id(), function_id = %obs.function_id(), error = %e, "observer delivery failed");
                }
            }
        });
    }
}

/// Files each observer of a `worktree::on-*` function in the subscriber set
/// its trigger type uses, and takes it out again when the engine removes it.
struct WorktreeObserveHandler {
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for WorktreeObserveHandler {
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

/// The error every `worktree::on-*` handler returns: events come only from
/// this worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the worktree worker emits its events"
        ),
        stacktrace: None,
    }
}

/// Registers `kind`'s observable function, owner-delivered, with args
/// [`BindingConfig`] (the trigger config) and hooks filing observers in the
/// trigger type's subscriber set. On an engine without observe it registers
/// as a plain function and its hooks never run.
fn register_observable<E>(iii: &IIIClient, kind: EventKind, description: &str, sets: &TriggerSets)
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
                .handler(WorktreeObserveHandler {
                    set: sets.for_kind(kind).clone(),
                }),
        ),
    );
    tracing::info!(function_id, "registered observable function");
}

/// Register the six custom trigger types with the engine, and the observable
/// `worktree::on-*` function next to each one, sharing the subscriber sets.
/// Must run before `functions::register_all` so handlers can capture the
/// subscriber sets.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();
    let descriptions: [(EventKind, &str, &str); 6] = [
        (
            EventKind::Created,
            "A new managed worktree exists.",
            ON_CREATED_DESC,
        ),
        (
            EventKind::Claimed,
            "A session took ownership of a worktree.",
            ON_CLAIMED_DESC,
        ),
        (
            EventKind::Released,
            "A worktree claim was released.",
            ON_RELEASED_DESC,
        ),
        (
            EventKind::Removed,
            "A managed worktree was removed.",
            ON_REMOVED_DESC,
        ),
        (
            EventKind::Landed,
            "A land completed; the target branch was fast-forwarded.",
            ON_LANDED_DESC,
        ),
        (
            EventKind::LandBlocked,
            "A land ended blocked (conflict, failed tests, dirty tree, moved target).",
            ON_LAND_BLOCKED_DESC,
        ),
    ];
    for (kind, description, observable_description) in descriptions {
        // Kept for consumers that bind the trigger type, on 0.24 and 0.25
        // engines alike.
        #[allow(deprecated)]
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                kind.trigger_type(),
                description,
                WorktreeTriggerHandler {
                    set: sets.for_kind(kind).clone(),
                },
            )
            .trigger_request_format::<BindingConfig>(),
        );
        tracing::info!(
            trigger_type = kind.trigger_type(),
            "registered trigger type"
        );
        match kind {
            EventKind::Created => {
                register_observable::<CreatedEvent>(iii, kind, observable_description, &sets)
            }
            EventKind::Claimed => {
                register_observable::<ClaimedEvent>(iii, kind, observable_description, &sets)
            }
            EventKind::Released => {
                register_observable::<ReleasedEvent>(iii, kind, observable_description, &sets)
            }
            EventKind::Removed => {
                register_observable::<RemovedEvent>(iii, kind, observable_description, &sets)
            }
            EventKind::Landed => {
                register_observable::<LandedEvent>(iii, kind, observable_description, &sets)
            }
            EventKind::LandBlocked => {
                register_observable::<LandBlockedEvent>(iii, kind, observable_description, &sets)
            }
        }
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
    async fn deliver(
        &self,
        trigger_type: &str,
        function_id: &str,
        namespace: Option<String>,
        payload: Value,
    );
}

/// Fire-and-forget bus delivery so the mutation that produced the event is
/// never blocked on subscriber latency. Failures are logged and swallowed.
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
    async fn deliver(
        &self,
        trigger_type: &str,
        function_id: &str,
        namespace: Option<String>,
        payload: Value,
    ) {
        // Dispatch each matching binding on its own task so the mutation that
        // produced the event (which still holds the per-repo lock) never waits
        // on a bus round-trip, and a slow binding cannot delay its siblings.
        let iii = self.iii.clone();
        let trigger_type = trigger_type.to_string();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let request = TriggerRequest {
                function_id: function_id.clone(),
                payload,
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let namespace = namespace.as_deref().unwrap_or("default");
            let res = iii.trigger(request.namespace(namespace)).await;
            if let Err(e) = res {
                tracing::warn!(trigger_type, function_id, error = %e, "event fan-out failed");
            }
        });
    }
}

/// Evaluates each binding's filter against each event and delivers the
/// payload to every match.
pub struct Emitter {
    sets: TriggerSets,
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { sets, deliverer }
    }

    pub fn sets(&self) -> &TriggerSets {
        &self.sets
    }

    pub async fn emit<T: Serialize>(&self, kind: EventKind, ctx: EventCtx<'_>, payload: &T) {
        let set = self.sets.for_kind(kind);
        let bindings: Vec<Binding> = set
            .snapshot()
            .into_iter()
            .filter(|b| binding_matches(&b.filter, &ctx))
            .collect();
        let observers: Vec<ObserverBinding> = set
            .observer_snapshot()
            .into_iter()
            .filter(|o| binding_matches(&o.filter, &ctx))
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
            self.deliverer
                .deliver(
                    kind.trigger_type(),
                    &binding.function_id,
                    binding.namespace.clone(),
                    payload.clone(),
                )
                .await;
        }
        for observer in observers {
            observer.observer.deliver(payload.clone()).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn empty_filter_matches_everything() {
        let f = BindingConfig::default();
        let ctx = EventCtx {
            repo_path: "/repo",
            worktree_id: "wt_1",
            session_id: Some("s_1"),
        };
        assert!(binding_matches(&f, &ctx));
    }

    #[test]
    fn each_filter_field_is_an_equality_match() {
        let ctx = EventCtx {
            repo_path: "/repo",
            worktree_id: "wt_1",
            session_id: None,
        };
        let by_repo = BindingConfig {
            repo_path: Some("/repo".into()),
            ..Default::default()
        };
        assert!(binding_matches(&by_repo, &ctx));
        let wrong_repo = BindingConfig {
            repo_path: Some("/other".into()),
            ..Default::default()
        };
        assert!(!binding_matches(&wrong_repo, &ctx));
        let by_session = BindingConfig {
            session_id: Some("s_1".into()),
            ..Default::default()
        };
        // Event without a session never matches a session filter.
        assert!(!binding_matches(&by_session, &ctx));
    }

    #[test]
    fn subscriber_set_rejects_unknown_fields() {
        let set = SubscriberSet::new(EventKind::Created);
        let err = set
            .add(TriggerConfig {
                id: "t1".into(),
                function_id: "ui::recv".into(),
                config: json!({ "sesion_id": "typo" }),
                metadata: None,
                namespace: None,
            })
            .unwrap_err();
        assert!(err.contains("invalid worktree::created config"));
        assert!(set.snapshot().is_empty());
    }

    #[test]
    fn subscriber_set_accepts_null_and_empty_configs() {
        let set = SubscriberSet::new(EventKind::Landed);
        set.add(TriggerConfig {
            id: "t1".into(),
            function_id: "ui::recv".into(),
            config: Value::Null,
            metadata: None,
            namespace: Some("project-a".into()),
        })
        .unwrap();
        set.add(TriggerConfig {
            id: "t2".into(),
            function_id: "ui::recv".into(),
            config: json!({}),
            metadata: None,
            namespace: None,
        })
        .unwrap();
        let snapshot = set.snapshot();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(
            snapshot
                .iter()
                .find(|binding| binding.id == "t1")
                .and_then(|binding| binding.namespace.as_deref()),
            Some("project-a")
        );
        set.remove("t1");
        assert_eq!(set.snapshot().len(), 1);
    }

    struct RecordingDeliverer {
        seen: Mutex<Vec<(String, String, Value)>>,
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        async fn deliver(
            &self,
            trigger_type: &str,
            function_id: &str,
            _namespace: Option<String>,
            payload: Value,
        ) {
            self.seen.lock().unwrap().push((
                trigger_type.to_string(),
                function_id.to_string(),
                payload,
            ));
        }
    }

    #[tokio::test]
    async fn emit_delivers_only_to_matching_bindings() {
        let sets = TriggerSets::new();
        sets.for_kind(EventKind::Landed)
            .add(TriggerConfig {
                id: "all".into(),
                function_id: "recv::all".into(),
                config: json!({}),
                metadata: None,
                namespace: None,
            })
            .unwrap();
        sets.for_kind(EventKind::Landed)
            .add(TriggerConfig {
                id: "other-repo".into(),
                function_id: "recv::other".into(),
                config: json!({ "repo_path": "/other" }),
                metadata: None,
                namespace: None,
            })
            .unwrap();
        let deliverer = Arc::new(RecordingDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(sets, deliverer.clone());
        let event = LandedEvent {
            worktree_id: "wt_1".into(),
            repo_path: "/repo".into(),
            branch: "iii/wt_1".into(),
            target_branch: "main".into(),
            merged_sha: "abc".into(),
            session_id: None,
            timestamp: 1,
        };
        emitter
            .emit(
                EventKind::Landed,
                EventCtx {
                    repo_path: "/repo",
                    worktree_id: "wt_1",
                    session_id: None,
                },
                &event,
            )
            .await;
        let seen = deliverer.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].1, "recv::all");
        assert_eq!(seen[0].2["merged_sha"], "abc");
    }

    /// Records what an observer of a `worktree::on-*` function receives.
    struct RecordingObserver {
        id: String,
        seen: Mutex<Vec<Value>>,
    }

    impl RecordingObserver {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                seen: Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EventObserver for RecordingObserver {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, payload: Value) {
            self.seen.lock().unwrap().push(payload);
        }
    }

    fn landed_event() -> LandedEvent {
        LandedEvent {
            worktree_id: "wt_1".into(),
            repo_path: "/repo".into(),
            branch: "iii/wt_1".into(),
            target_branch: "main".into(),
            merged_sha: "abc".into(),
            session_id: Some("s_1".into()),
            timestamp: 7,
        }
    }

    fn landed_ctx() -> EventCtx<'static> {
        EventCtx {
            repo_path: "/repo",
            worktree_id: "wt_1",
            session_id: Some("s_1"),
        }
    }

    #[test]
    fn each_kind_has_its_observable_function_next_to_its_trigger_type() {
        let pairs: Vec<(&str, &str)> = EventKind::all()
            .iter()
            .map(|k| (k.trigger_type(), k.observable_function()))
            .collect();
        assert_eq!(
            pairs,
            vec![
                (CREATED, ON_CREATED),
                (CLAIMED, ON_CLAIMED),
                (RELEASED, ON_RELEASED),
                (REMOVED, ON_REMOVED),
                (LANDED, ON_LANDED),
                (LAND_BLOCKED, ON_LAND_BLOCKED),
            ]
        );
        for (trigger_type, function_id) in pairs {
            let suffix = trigger_type.strip_prefix("worktree::").unwrap();
            assert_eq!(function_id, format!("worktree::on-{suffix}"));
        }
    }

    #[test]
    fn every_on_function_refuses_direct_invocation_with_forbidden() {
        for kind in EventKind::all() {
            let function_id = kind.observable_function();
            match refuse_invocation(function_id) {
                Error::Remote { code, message, .. } => {
                    assert_eq!(code, codes::FORBIDDEN, "{function_id}");
                    assert!(message.contains(function_id), "{message}");
                    assert!(message.contains("observe"), "{message}");
                }
                other => panic!("{function_id}: expected Error::Remote FORBIDDEN, got {other:?}"),
            }
        }
    }

    #[test]
    fn observer_args_are_parsed_like_a_binding_config() {
        let set = SubscriberSet::new(EventKind::Claimed);
        let err = set
            .add_observer(RecordingObserver::new("o1"), json!({ "sesion_id": "typo" }))
            .unwrap_err();
        assert!(err.contains("invalid worktree::claimed config"), "{err}");
        assert!(set.observer_snapshot().is_empty());

        set.add_observer(RecordingObserver::new("o2"), Value::Null)
            .unwrap();
        set.add_observer(
            RecordingObserver::new("o3"),
            json!({ "repo_path": "/repo", "session_id": "s_1" }),
        )
        .unwrap();
        let snapshot = set.observer_snapshot();
        assert_eq!(snapshot.len(), 2);
        let o3 = snapshot
            .iter()
            .find(|o| o.observer.id() == "o3")
            .expect("o3 filed");
        assert_eq!(o3.filter.repo_path.as_deref(), Some("/repo"));
        assert_eq!(o3.filter.session_id.as_deref(), Some("s_1"));
        // Observers and bindings are kept apart.
        assert!(set.snapshot().is_empty());
        set.remove_observer("o2");
        assert_eq!(set.observer_snapshot().len(), 1);
    }

    #[tokio::test]
    async fn observer_receives_the_event_a_binding_receives() {
        let sets = TriggerSets::new();
        let config = json!({ "repo_path": "/repo", "session_id": "s_1" });
        sets.for_kind(EventKind::Landed)
            .add(TriggerConfig {
                id: "binding".into(),
                function_id: "recv::binding".into(),
                config: config.clone(),
                metadata: None,
                namespace: None,
            })
            .unwrap();
        let observer = RecordingObserver::new("observer");
        sets.for_kind(EventKind::Landed)
            .add_observer(observer.clone(), config)
            .unwrap();
        let deliverer = Arc::new(RecordingDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(sets, deliverer.clone());
        let event = landed_event();
        emitter.emit(EventKind::Landed, landed_ctx(), &event).await;

        let bound = deliverer.seen.lock().unwrap().clone();
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].0, LANDED);
        let observed = observer.seen();
        assert_eq!(observed.len(), 1);
        assert_eq!(
            observed[0], bound[0].2,
            "observer and binding get the same event"
        );
        assert_eq!(observed[0], serde_json::to_value(&event).unwrap());
    }

    #[tokio::test]
    async fn emit_delivers_only_to_matching_observers_of_the_kind() {
        let sets = TriggerSets::new();
        let all = RecordingObserver::new("all");
        let other_repo = RecordingObserver::new("other-repo");
        let other_session = RecordingObserver::new("other-session");
        let other_kind = RecordingObserver::new("other-kind");
        let removed = RecordingObserver::new("removed");
        let landed = sets.for_kind(EventKind::Landed);
        landed.add_observer(all.clone(), json!({})).unwrap();
        landed
            .add_observer(other_repo.clone(), json!({ "repo_path": "/other" }))
            .unwrap();
        landed
            .add_observer(other_session.clone(), json!({ "session_id": "s_2" }))
            .unwrap();
        landed.add_observer(removed.clone(), Value::Null).unwrap();
        landed.remove_observer("removed");
        sets.for_kind(EventKind::LandBlocked)
            .add_observer(other_kind.clone(), json!({}))
            .unwrap();
        let deliverer = Arc::new(RecordingDeliverer {
            seen: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(sets, deliverer.clone());
        emitter
            .emit(EventKind::Landed, landed_ctx(), &landed_event())
            .await;

        assert_eq!(all.seen().len(), 1);
        assert_eq!(all.seen()[0]["merged_sha"], "abc");
        assert!(other_repo.seen().is_empty());
        assert!(other_session.seen().is_empty());
        assert!(other_kind.seen().is_empty());
        assert!(removed.seen().is_empty());
        // No bindings: nothing went through the trigger deliverer.
        assert!(deliverer.seen.lock().unwrap().is_empty());
    }
}
