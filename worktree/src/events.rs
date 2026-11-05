//! The six event functions this worker hosts, and the emitter that calls
//! them. Each is private (only this worker calls it), takes no args, and
//! returns the event it is called with, which is what observers observe.
//! iii delivers every call to each observer whose filters match: consumers
//! select events with filters on `input.repo_path`, `input.worktree_id` and
//! `input.session_id`. The emitter calls a function fire-and-forget
//! (`CallAction::Void`), and only while something observes it.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{CallAction, CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const ON_CREATED_FN: &str = "worktree::on-created";
pub const ON_CLAIMED_FN: &str = "worktree::on-claimed";
pub const ON_RELEASED_FN: &str = "worktree::on-released";
pub const ON_REMOVED_FN: &str = "worktree::on-removed";
pub const ON_LANDED_FN: &str = "worktree::on-landed";
pub const ON_LAND_BLOCKED_FN: &str = "worktree::on-land-blocked";

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
    /// The function that carries this event.
    pub fn function_id(&self) -> &'static str {
        match self {
            EventKind::Created => ON_CREATED_FN,
            EventKind::Claimed => ON_CLAIMED_FN,
            EventKind::Released => ON_RELEASED_FN,
            EventKind::Removed => ON_REMOVED_FN,
            EventKind::Landed => ON_LANDED_FN,
            EventKind::LandBlocked => ON_LAND_BLOCKED_FN,
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

// ---------------------------------------------------------------------------
// Event payloads (what the functions return, and observers observe)
// ---------------------------------------------------------------------------

/// `worktree::on-created` — a new managed worktree exists.
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

/// `worktree::on-claimed` — a session took ownership.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ClaimedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub previous_session_id: Option<String>,
    pub timestamp: i64,
}

/// `worktree::on-released` — a claim was released.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ReleasedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub session_id: String,
    pub timestamp: i64,
}

/// `worktree::on-removed` — a managed worktree was removed (remove or prune).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RemovedEvent {
    pub worktree_id: String,
    pub repo_path: String,
    pub branch: String,
    pub branch_deleted: bool,
    /// The session that held the worktree's claim, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub timestamp: i64,
}

/// `worktree::on-landed` — a land completed; the target branch was advanced.
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

/// `worktree::on-land-blocked` — a land ended blocked; see `reason` and `code`.
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
// Registration
// ---------------------------------------------------------------------------

const FILTERS: &str = "No args; filter on input.repo_path, input.worktree_id, input.session_id.";

/// Register the six event functions. Each handler returns the event it is
/// called with.
pub fn register_functions(iii: &IIIClient) {
    fn source<E>(iii: &IIIClient, kind: EventKind, what: &str)
    where
        E: Serialize + for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
    {
        iii.register_function(
            kind.function_id(),
            RegisterFunction::new(|event: E| Ok::<_, Error>(event))
                .description(format!("{what} {FILTERS}"))
                .private(),
        );
        tracing::info!(
            function_id = kind.function_id(),
            "registered event function"
        );
    }
    source::<CreatedEvent>(
        iii,
        EventKind::Created,
        "Called when a new managed worktree exists.",
    );
    source::<ClaimedEvent>(
        iii,
        EventKind::Claimed,
        "Called when a session took ownership of a worktree.",
    );
    source::<ReleasedEvent>(
        iii,
        EventKind::Released,
        "Called when a worktree claim was released.",
    );
    source::<RemovedEvent>(
        iii,
        EventKind::Removed,
        "Called when a managed worktree was removed.",
    );
    source::<LandedEvent>(
        iii,
        EventKind::Landed,
        "Called when a land completed; the target branch was fast-forwarded.",
    );
    source::<LandBlockedEvent>(
        iii,
        EventKind::LandBlocked,
        "Called when a land ended blocked (conflict, failed tests, dirty tree, moved target).",
    );
}

// ---------------------------------------------------------------------------
// Emission
// ---------------------------------------------------------------------------

/// How an event reaches its function. Production calls it over the bus;
/// tests record.
#[async_trait]
pub trait EventDeliverer: Send + Sync {
    /// Whether anything observes `function_id`: the emitter builds no event
    /// nobody wants.
    fn is_observed(&self, function_id: &str) -> bool;

    async fn deliver(&self, function_id: &str, payload: Value);
}

/// Fire-and-forget call of the event function, so the mutation that produced
/// the event is never blocked on observers. Failures are logged and
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
    fn is_observed(&self, function_id: &str) -> bool {
        self.iii.is_observed(function_id)
    }

    async fn deliver(&self, function_id: &str, payload: Value) {
        // On its own task so the mutation that produced the event (which
        // still holds the per-repo lock) never waits on the call.
        let iii = self.iii.clone();
        let function_id = function_id.to_string();
        tokio::spawn(async move {
            let request = CallRequest {
                function_id: function_id.clone(),
                payload,
                action: Some(CallAction::Void),
                timeout_ms: None,
            };
            if let Err(e) = iii.call(request).await {
                tracing::warn!(function_id, error = %e, "event call failed");
            }
        });
    }
}

/// Calls each event's function while it is observed.
pub struct Emitter {
    deliverer: Arc<dyn EventDeliverer>,
}

impl Emitter {
    pub fn new(deliverer: Arc<dyn EventDeliverer>) -> Self {
        Self { deliverer }
    }

    pub async fn emit<T: Serialize>(&self, kind: EventKind, payload: &T) {
        let function_id = kind.function_id();
        if !self.deliverer.is_observed(function_id) {
            return;
        }
        let payload = match serde_json::to_value(payload) {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!(error = %e, "event payload failed to serialize");
                return;
            }
        };
        self.deliverer.deliver(function_id, payload).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct RecordingDeliverer {
        observed: bool,
        seen: Mutex<Vec<(String, Value)>>,
    }

    #[async_trait]
    impl EventDeliverer for RecordingDeliverer {
        fn is_observed(&self, _function_id: &str) -> bool {
            self.observed
        }

        async fn deliver(&self, function_id: &str, payload: Value) {
            self.seen
                .lock()
                .unwrap()
                .push((function_id.to_string(), payload));
        }
    }

    fn landed() -> LandedEvent {
        LandedEvent {
            worktree_id: "wt_1".into(),
            repo_path: "/repo".into(),
            branch: "iii/wt_1".into(),
            target_branch: "main".into(),
            merged_sha: "abc".into(),
            session_id: None,
            timestamp: 1,
        }
    }

    #[test]
    fn every_kind_has_an_on_function() {
        for kind in EventKind::all() {
            assert!(kind.function_id().starts_with("worktree::on-"));
        }
        assert_eq!(
            EventKind::LandBlocked.function_id(),
            "worktree::on-land-blocked"
        );
    }

    #[tokio::test]
    async fn emit_calls_the_function_of_its_kind() {
        let deliverer = Arc::new(RecordingDeliverer {
            observed: true,
            seen: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(deliverer.clone());
        emitter.emit(EventKind::Landed, &landed()).await;
        let seen = deliverer.seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, "worktree::on-landed");
        assert_eq!(seen[0].1["merged_sha"], "abc");
    }

    #[tokio::test]
    async fn emit_skips_an_unobserved_function() {
        let deliverer = Arc::new(RecordingDeliverer {
            observed: false,
            seen: Mutex::new(Vec::new()),
        });
        let emitter = Emitter::new(deliverer.clone());
        emitter.emit(EventKind::Landed, &landed()).await;
        assert!(deliverer.seen.lock().unwrap().is_empty());
    }
}
