//! The async orchestration trigger types the harness emits —
//! `harness::ready` after boot, `harness::turn-started` /
//! `harness::turn-completed` at turn boundaries, `harness::message-queued`
//! when a message parks in the mid-turn queue, and
//! `harness::triggers-changed` when a session's binding set or fire count
//! changes (harness.md § Trigger types emitted). Consumers and siblings bind
//! these to react without polling.
//!
//! Delivery is fire-and-forget (`TriggerAction::Void`), at-least-once, and
//! unordered. Per-binding `config` filters (`session_id`, `parent_session_id`)
//! are evaluated here by the emitting worker. The engine replays existing
//! registrations after a restart, so the subscriber sets rebuild themselves.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::types::turn::ParentLink;

pub const TURN_STARTED: &str = "harness::turn-started";
pub const TURN_COMPLETED: &str = "harness::turn-completed";
pub const MESSAGE_QUEUED: &str = "harness::message-queued";
pub const TRIGGERS_CHANGED: &str = "harness::triggers-changed";
pub const READY: &str = "harness::ready";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadyBindingConfig {}

/// Binding config shared by both turn-event types.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TurnEventBindingConfig {
    /// Only deliver events for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Only deliver sub-agent events whose parent is this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct BindingFilter {
    pub session_id: Option<String>,
    pub parent_session_id: Option<String>,
}

impl BindingFilter {
    fn parse(raw: &Value) -> Result<Self, String> {
        let raw = if raw.is_null() {
            Value::Object(Default::default())
        } else {
            raw.clone()
        };
        let cfg: TurnEventBindingConfig =
            serde_json::from_value(raw).map_err(|e| format!("invalid turn-event config: {e}"))?;
        Ok(BindingFilter {
            session_id: cfg.session_id,
            parent_session_id: cfg.parent_session_id,
        })
    }

    fn matches(
        &self,
        session_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
    ) -> bool {
        if let Some(sid) = &self.session_id {
            if sid != session_id {
                return false;
            }
        }
        if let Some(psid) = &self.parent_session_id {
            let link_matches = matches!(parent, Some(p) if &p.session_id == psid);
            let display_matches = display_parent == Some(psid.as_str());
            if !link_matches && !display_matches {
                return false;
            }
        }
        true
    }
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    filter: BindingFilter,
    /// The trigger's registration `metadata`, forwarded to the bound function
    /// as the invocation sidecar so targets like `harness::spawn` and
    /// `harness::notify_agent` can carry per-subscription context (the spawn
    /// spec, or the subscription id/label). Stamped once at registration and
    /// delivered as stored — there is no fire-time stamping. Carries the
    /// `__subscription_id` the self-edge check in `fan_out` matches on — the
    /// engine's own registration id (`TriggerConfig.id`) is keyed on in
    /// `SubscriberSet` but otherwise unused here.
    metadata: Option<Value>,
}

#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = BindingFilter::parse(&config.config)?;
        self.lock().insert(
            config.id,
            Binding {
                function_id: config.function_id,
                filter,
                metadata: config.metadata,
            },
        );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    fn add_unfiltered(&self, config: TriggerConfig) -> Binding {
        let binding = Binding {
            function_id: config.function_id,
            filter: BindingFilter::default(),
            metadata: config.metadata,
        };
        self.lock().insert(config.id, binding.clone());
        binding
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct ReadyTriggerHandler {
    set: SubscriberSet,
    ready: Arc<AtomicBool>,
    iii: Arc<IIIClient>,
}

#[async_trait]
impl TriggerHandler for ReadyTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let raw = if config.config.is_null() {
            serde_json::json!({})
        } else {
            config.config.clone()
        };
        serde_json::from_value::<ReadyBindingConfig>(raw)
            .map_err(|error| Error::Handler(format!("invalid ready-event config: {error}")))?;
        let binding = self.set.add_unfiltered(config);
        if self.ready.load(Ordering::Acquire) {
            deliver_ready(&self.iii, &binding).await;
        }
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

struct TurnEventTriggerHandler {
    type_id: &'static str,
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for TurnEventTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        // Trigger delivery never creates an agent — refuse the binding loudly
        // at registration instead of letting it no-op (or worse) at fire
        // time. This is the engine-direct path; agent registrations are
        // already rejected wholesale by the subscribe interceptor.
        if function_id == crate::functions::SPAWN_ID {
            return Err(Error::Handler(
                "harness::spawn is not a trigger target: trigger delivery never creates an \
                 agent. Spawn children directly and watch what they write."
                    .to_string(),
            ));
        }
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = self.type_id, %id, %function_id, "turn-event subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// The harness's emitted turn-event subscriber sets + the engine handle for
/// fan-out. Cloned into [`crate::deps::Deps`].
#[derive(Clone)]
pub struct TurnEvents {
    iii: Arc<IIIClient>,
    ready: SubscriberSet,
    ready_emitted: Arc<AtomicBool>,
    started: SubscriberSet,
    completed: SubscriberSet,
    queued: SubscriberSet,
    triggers_changed: SubscriberSet,
}

impl TurnEvents {
    /// Register the trigger types and return the emitter. Must run before
    /// function registration so the handlers capture the subscriber sets.
    pub fn register(iii: &Arc<IIIClient>) -> Self {
        let ready = SubscriberSet::default();
        let ready_emitted = Arc::new(AtomicBool::new(false));
        let started = SubscriberSet::default();
        let completed = SubscriberSet::default();
        let queued = SubscriberSet::default();
        let triggers_changed = SubscriberSet::default();

        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                READY,
                "The harness completed boot and can accept turns.",
                ReadyTriggerHandler {
                    set: ready.clone(),
                    ready: ready_emitted.clone(),
                    iii: iii.clone(),
                },
            )
            .trigger_request_format::<ReadyBindingConfig>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TURN_STARTED,
                "A harness turn began executing (first loop step).",
                TurnEventTriggerHandler {
                    type_id: TURN_STARTED,
                    set: started.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TURN_COMPLETED,
                "A harness turn reached a terminal status (completed/cancelled/failed).",
                TurnEventTriggerHandler {
                    type_id: TURN_COMPLETED,
                    set: completed.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                MESSAGE_QUEUED,
                "A message parked in a session's server-side queue while its turn streams.",
                TurnEventTriggerHandler {
                    type_id: MESSAGE_QUEUED,
                    set: queued.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                TRIGGERS_CHANGED,
                "A session's trigger-binding set or fire count changed — refetch harness::triggers::list.",
                TurnEventTriggerHandler {
                    type_id: TRIGGERS_CHANGED,
                    set: triggers_changed.clone(),
                },
            )
            .trigger_request_format::<TurnEventBindingConfig>(),
        );
        tracing::info!(
            "registered harness::ready / harness::turn-started / harness::turn-completed / harness::message-queued / harness::triggers-changed trigger types"
        );

        Self {
            iii: iii.clone(),
            ready,
            ready_emitted,
            started,
            completed,
            queued,
            triggers_changed,
        }
    }

    /// Announce readiness after all turn dependencies and lifecycle bindings
    /// have been installed. Subscribers that bind later receive the current
    /// ready state from [`ReadyTriggerHandler`].
    pub async fn emit_ready(&self) {
        self.ready_emitted.store(true, Ordering::Release);
        for binding in self.ready.snapshot() {
            deliver_ready(&self.iii, &binding).await;
        }
    }

    /// A message parked in the session's `harness_queue` (send's queue path).
    /// The payload is a pointer, not the message — consumers refetch
    /// `harness::status` → `queued` (idempotent under at-least-once delivery).
    pub async fn emit_queued(&self, session_id: &str, entry_id: &str, queued_at: i64) {
        tracing::info!(session_id, entry_id, "message queued");
        let payload = serde_json::json!({
            "session_id": session_id,
            "entry_id": entry_id,
            "queued_at": queued_at,
            "timestamp": now_ms(),
        });
        self.fan_out(
            &self.queued,
            MESSAGE_QUEUED,
            session_id,
            None,
            None,
            payload,
        )
        .await;
    }

    /// Doorbell only — a session's binding set or fires count changed;
    /// consumers refetch `harness::triggers::list`. No per-event logging:
    /// a single fire rings twice (claim + retirement).
    pub async fn emit_triggers_changed(&self, session_id: &str) {
        let payload = serde_json::json!({
            "session_id": session_id,
            "timestamp": now_ms(),
        });
        self.fan_out(
            &self.triggers_changed,
            TRIGGERS_CHANGED,
            session_id,
            None,
            None,
            payload,
        )
        .await;
    }

    pub async fn emit_started(
        &self,
        session_id: &str,
        turn_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
    ) {
        tracing::info!(session_id, turn_id, "turn started");
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "timestamp": now_ms(),
        });
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        if let Some(dp) = display_parent {
            payload["parent_session_id"] = Value::String(dp.to_string());
        }
        self.fan_out(
            &self.started,
            TURN_STARTED,
            session_id,
            parent,
            display_parent,
            payload,
        )
        .await;
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn emit_completed(
        &self,
        session_id: &str,
        turn_id: &str,
        status: &str,
        result: Option<&Value>,
        result_error: Option<&str>,
        reason: Option<&str>,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        terminal: bool,
        context: Option<&crate::context_snapshot::ContextSnapshotV1>,
    ) {
        tracing::info!(
            session_id,
            turn_id,
            status,
            result_error,
            terminal,
            "turn completed"
        );
        // `terminal: false` marks a turn whose session still owns an armed
        // wake (a one-shot notify) — the run continues and a later turn in the
        // same session carries the real outcome. Consumers should treat
        // non-terminal completions as progress and finalize only on
        // `terminal: true`.
        let mut payload = serde_json::json!({
            "session_id": session_id,
            "turn_id": turn_id,
            "status": status,
            "terminal": terminal,
            "timestamp": now_ms(),
        });
        if let Some(r) = result {
            payload["result"] = r.clone();
        }
        if let Some(e) = result_error {
            payload["result_error"] = Value::String(e.to_string());
        }
        if let Some(r) = reason {
            payload["reason"] = Value::String(r.to_string());
        }
        if let Some(p) = parent {
            payload["parent"] = serde_json::to_value(p).unwrap_or(Value::Null);
        }
        if let Some(dp) = display_parent {
            payload["parent_session_id"] = Value::String(dp.to_string());
        }
        // The latest generation's context accounting (categories, budget,
        // usage) so live consumers never re-walk the transcript for it.
        if let Some(snapshot) = context {
            payload["context"] = serde_json::to_value(snapshot).unwrap_or(Value::Null);
        }
        self.fan_out(
            &self.completed,
            TURN_COMPLETED,
            session_id,
            parent,
            display_parent,
            payload,
        )
        .await;
    }

    async fn fan_out(
        &self,
        set: &SubscriberSet,
        trigger_type: &str,
        session_id: &str,
        parent: Option<&ParentLink>,
        display_parent: Option<&str>,
        payload: Value,
    ) {
        for binding in set.snapshot() {
            if !binding.filter.matches(session_id, parent, display_parent) {
                continue;
            }
            // The registration metadata is forwarded as stored; there is no
            // fire-time stamping.
            let request = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: payload.clone(),
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let res = match binding.metadata.clone() {
                Some(m) => self.iii.trigger(request.metadata(m)).await,
                None => self.iii.trigger(request).await,
            };
            if let Err(e) = res {
                tracing::warn!(trigger_type, function_id = %binding.function_id, error = %e, "turn-event fan-out failed");
            }
        }
    }
}

async fn deliver_ready(iii: &IIIClient, binding: &Binding) {
    let request = TriggerRequest {
        function_id: binding.function_id.clone(),
        payload: serde_json::json!({
            "status": "ready",
            "timestamp": now_ms(),
        }),
        action: Some(TriggerAction::Void),
        timeout_ms: None,
    };
    let result = match &binding.metadata {
        Some(metadata) => iii.trigger(request.metadata(metadata.clone())).await,
        None => iii.trigger(request).await,
    };
    if let Err(error) = result {
        tracing::warn!(
            trigger_type = READY,
            function_id = %binding.function_id,
            %error,
            "ready-event fan-out failed"
        );
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filter_matches_session_and_parent() {
        let f = BindingFilter {
            session_id: Some("s_1".into()),
            parent_session_id: None,
        };
        assert!(f.matches("s_1", None, None));
        assert!(!f.matches("s_2", None, None));

        let pf = BindingFilter {
            session_id: None,
            parent_session_id: Some("p_1".into()),
        };
        let parent = ParentLink {
            session_id: "p_1".into(),
            turn_id: "t".into(),
            function_call_id: "fc".into(),
        };
        assert!(pf.matches("child", Some(&parent), None));
        assert!(!pf.matches("child", None, None));
    }

    #[test]
    fn filter_matches_display_parent_for_trigger_fired_children() {
        let pf = BindingFilter {
            session_id: None,
            parent_session_id: Some("root_1".into()),
        };
        // Trigger-spawned child: no ParentLink, display parent only.
        assert!(pf.matches("child", None, Some("root_1")));
        assert!(!pf.matches("child", None, Some("other_root")));
    }

    #[test]
    fn parse_rejects_unknown_fields() {
        assert!(BindingFilter::parse(&json!({ "sesion_id": "x" })).is_err());
        assert_eq!(
            BindingFilter::parse(&Value::Null).unwrap(),
            BindingFilter::default()
        );
    }
}
