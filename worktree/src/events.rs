//! The six custom trigger types this worker emits, and the fan-out behind
//! them. Consumers bind handlers with the standard two-step pattern; the
//! engine routes each registration to our [`TriggerHandler`]s. Delivery is
//! fire-and-forget (`TriggerAction::Void`) and at-least-once; per-binding
//! `config` filters are evaluated here by the emitting worker, and malformed
//! configs are rejected at registration time.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const CREATED: &str = "worktree::created";
pub const CLAIMED: &str = "worktree::claimed";
pub const RELEASED: &str = "worktree::released";
pub const REMOVED: &str = "worktree::removed";
pub const LANDED: &str = "worktree::landed";
pub const LAND_BLOCKED: &str = "worktree::land-blocked";

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
    pub filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type.
#[derive(Clone)]
pub struct SubscriberSet {
    kind: EventKind,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    pub fn new(kind: EventKind) -> Self {
        Self {
            kind,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Parse + insert a binding. Rejects malformed configs at registration.
    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let raw = if config.config.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            config.config.clone()
        };
        let filter: BindingConfig = serde_json::from_value(raw)
            .map_err(|e| format!("invalid {} config: {e}", self.kind.trigger_type()))?;
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

    /// Snapshot so the mutex is never held across awaits.
    pub fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
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

/// Register the six custom trigger types with the engine. Must run before
/// `functions::register_all` so handlers can capture the subscriber sets.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();
    let descriptions: [(EventKind, &str); 6] = [
        (EventKind::Created, "A new managed worktree exists."),
        (
            EventKind::Claimed,
            "A session took ownership of a worktree.",
        ),
        (EventKind::Released, "A worktree claim was released."),
        (EventKind::Removed, "A managed worktree was removed."),
        (
            EventKind::Landed,
            "A land completed; the target branch was fast-forwarded.",
        ),
        (
            EventKind::LandBlocked,
            "A land ended blocked (conflict, failed tests, dirty tree, moved target).",
        ),
    ];
    for (kind, description) in descriptions {
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
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value);
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
    async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
        // Dispatch each matching binding on its own task so the mutation that
        // produced the event (which still holds the per-repo lock) never waits
        // on a bus round-trip, and a slow binding cannot delay its siblings.
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
        let bindings = self.sets.for_kind(kind).snapshot();
        if bindings.is_empty() {
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
            if binding_matches(&binding.filter, &ctx) {
                self.deliverer
                    .deliver(kind.trigger_type(), &binding.function_id, payload.clone())
                    .await;
            }
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
        })
        .unwrap();
        set.add(TriggerConfig {
            id: "t2".into(),
            function_id: "ui::recv".into(),
            config: json!({}),
            metadata: None,
        })
        .unwrap();
        assert_eq!(set.snapshot().len(), 2);
        set.remove("t1");
        assert_eq!(set.snapshot().len(), 1);
    }

    struct RecordingDeliverer {
        seen: Mutex<Vec<(String, String, Value)>>,
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        async fn deliver(&self, trigger_type: &str, function_id: &str, payload: Value) {
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
            })
            .unwrap();
        sets.for_kind(EventKind::Landed)
            .add(TriggerConfig {
                id: "other-repo".into(),
                function_id: "recv::other".into(),
                config: json!({ "repo_path": "/other" }),
                metadata: None,
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
}
