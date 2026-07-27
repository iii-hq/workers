//! The two custom trigger types this worker emits —
//! `approval::pending-created` / `approval::pending-resolved` — and the
//! fan-out machinery behind them (the session-manager reactivity model:
//! consumers bind handlers with the standard two-step pattern; the engine
//! routes each registration to our [`TriggerHandler`]s; delivery is
//! fire-and-forget, at-least-once, unordered; `list_pending` is the
//! reconciliation read).
//!
//! Per-binding `config` filters are evaluated here, by the emitting
//! worker: `session_id` equality and `metadata` subset-equality against
//! the record's denormalized `session_metadata` (the tenancy hook).
//! Malformed configs are rejected at registration time.

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

use crate::types::{metadata_matches, JsonMap, PendingApprovalRecord, PendingResolvedEvent};

pub const PENDING_CREATED: &str = "approval::pending-created";
pub const PENDING_CREATED_DESC: &str =
    "A function call was held for human approval and its inbox record written. \
     Payload: PendingApprovalRecord (redacted args, session context, expiry). \
     Bind notification workers here.";

pub const PENDING_RESOLVED: &str = "approval::pending-resolved";
pub const PENDING_RESOLVED_DESC: &str =
    "A pending approval left the inbox (outcome: allow | deny | timeout | aborted). \
     Emitted exactly once per record; lets UIs clear badges.";

/// Config accepted by both trigger types.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this session.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Equality match against the record's `session_metadata` (every key
    /// given here must equal the stored value — subset match).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonMap>,
}

/// `null` configs are treated as `{}`; unknown fields are rejected so a
/// misspelled filter key fails at registration, not silently.
pub fn parse_binding_config(raw: &Value) -> Result<BindingConfig, String> {
    if raw.is_null() {
        return Ok(BindingConfig::default());
    }
    serde_json::from_value(raw.clone()).map_err(|e| format!("invalid binding config: {e}"))
}

/// Pure filter evaluation: does this binding receive this event?
pub fn binding_matches(
    filter: &BindingConfig,
    session_id: &str,
    session_metadata: Option<&JsonMap>,
) -> bool {
    if let Some(want_sid) = &filter.session_id {
        if want_sid != session_id {
            return false;
        }
    }
    if let Some(want) = &filter.metadata {
        if !metadata_matches(want, session_metadata) {
            return false;
        }
    }
    true
}

#[derive(Debug, Clone)]
pub struct Binding {
    pub id: String,
    pub function_id: String,
    pub filter: BindingConfig,
}

/// Thread-safe subscriber registry for one trigger type. Cloned into both
/// the [`TriggerHandler`] (mutates on register/unregister) and the
/// [`Emitter`] (iterates read-only snapshots). After a worker restart the
/// engine replays existing registrations, so the sets rebuild themselves.
#[derive(Clone)]
pub struct SubscriberSet {
    trigger_type: &'static str,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
}

impl SubscriberSet {
    pub fn new(trigger_type: &'static str) -> Self {
        Self {
            trigger_type,
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn trigger_type(&self) -> &'static str {
        self.trigger_type
    }

    pub fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = parse_binding_config(&config.config)?;
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

    /// Snapshot so the mutex isn't held across awaits.
    pub fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[derive(Clone)]
pub struct TriggerSets {
    pub created: SubscriberSet,
    pub resolved: SubscriberSet,
}

impl TriggerSets {
    pub fn new() -> Self {
        Self {
            created: SubscriberSet::new(PENDING_CREATED),
            resolved: SubscriberSet::new(PENDING_RESOLVED),
        }
    }
}

impl Default for TriggerSets {
    fn default() -> Self {
        Self::new()
    }
}

struct ApprovalTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for ApprovalTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(
            trigger_type = self.set.trigger_type(),
            id = %id,
            function_id = %function_id,
            "trigger subscription registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = self.set.trigger_type(),
            id = %config.id,
            "trigger subscription unregistered"
        );
        self.set.remove(&config.id);
        Ok(())
    }
}

/// Register both custom trigger types with the engine. Must run **before**
/// `functions::register_all` so the handlers capture the subscriber sets
/// they fan out to.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();

    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            PENDING_CREATED,
            PENDING_CREATED_DESC,
            ApprovalTriggerHandler {
                set: sets.created.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            PENDING_RESOLVED,
            PENDING_RESOLVED_DESC,
            ApprovalTriggerHandler {
                set: sets.resolved.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );

    tracing::info!(
        trigger_types = ?[PENDING_CREATED, PENDING_RESOLVED],
        "registered trigger types"
    );
    sets
}

/// Where emissions go. Production fans out via iii; tests record.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn pending_created(&self, record: &PendingApprovalRecord);
    async fn pending_resolved(&self, event: &PendingResolvedEvent);
}

/// Filtered fire-and-forget fan-out to every matching binding.
pub struct Emitter {
    sets: TriggerSets,
    iii: Arc<IIIClient>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, iii: Arc<IIIClient>) -> Self {
        Self { sets, iii }
    }

    async fn fan_out(
        &self,
        set: &SubscriberSet,
        session_id: &str,
        session_metadata: Option<&JsonMap>,
        payload: Value,
    ) {
        for binding in set.snapshot() {
            if binding_matches(&binding.filter, session_id, session_metadata) {
                let request = TriggerRequest {
                    function_id: binding.function_id.clone(),
                    payload: payload.clone(),
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                };
                let route_ns = self.iii.namespace().filter(|_| {
                    !matches!(
                        binding.function_id.split("::").next(),
                        Some(
                            "state"
                                | "stream"
                                | "queue"
                                | "pubsub"
                                | "configuration"
                                | "cron"
                                | "http"
                                | "engine"
                                | "sandbox"
                                | "log"
                                | "secret"
                                | "kv"
                                | "iii"
                        )
                    )
                });
                let res = match route_ns {
                    Some(ns) => self.iii.trigger(request.namespace(ns)).await,
                    None => self.iii.trigger(request).await,
                };
                if let Err(e) = res {
                    tracing::warn!(
                        function_id = %binding.function_id,
                        error = %e,
                        "void fan-out failed"
                    );
                }
            }
        }
    }
}

#[async_trait]
impl EventSink for Emitter {
    async fn pending_created(&self, record: &PendingApprovalRecord) {
        // Payload: PendingApprovalRecord & { status: "pending" }.
        let mut payload = serde_json::to_value(record).unwrap_or(Value::Null);
        if let Some(map) = payload.as_object_mut() {
            map.insert("status".to_string(), Value::String("pending".to_string()));
        }
        self.fan_out(
            &self.sets.created,
            &record.session_id,
            record.session_metadata.as_ref(),
            payload,
        )
        .await;
    }

    async fn pending_resolved(&self, event: &PendingResolvedEvent) {
        let payload = serde_json::to_value(event).unwrap_or(Value::Null);
        self.fan_out(
            &self.sets.resolved,
            &event.session_id,
            event.session_metadata.as_ref(),
            payload,
        )
        .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn trigger_config(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: function_id.into(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn null_config_is_match_all() {
        let filter = parse_binding_config(&Value::Null).unwrap();
        assert!(binding_matches(&filter, "s_1", None));
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        assert!(parse_binding_config(&json!({ "sesion_id": "typo" })).is_err());
    }

    #[test]
    fn session_id_filter_is_equality() {
        let filter = parse_binding_config(&json!({ "session_id": "s_1" })).unwrap();
        assert!(binding_matches(&filter, "s_1", None));
        assert!(!binding_matches(&filter, "s_2", None));
    }

    #[test]
    fn metadata_filter_is_subset_equality() {
        let filter = parse_binding_config(&json!({ "metadata": { "owner": "u_1" } })).unwrap();
        let meta: JsonMap = serde_json::from_value(json!({ "owner": "u_1", "extra": 1 })).unwrap();
        assert!(binding_matches(&filter, "s_1", Some(&meta)));
        assert!(!binding_matches(&filter, "s_1", None));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn emitter_delivers_to_matching_bindings_with_status() {
        crate::testkit::with_stack(
            crate::testkit::BootOpts::needs_approval(),
            |stack| async move {
                let out = crate::testkit::call(
                    &stack.iii,
                    "approval::gate",
                    crate::testkit::hook_input("s_1", "c_1", "shell::run"),
                )
                .await
                .unwrap();
                assert_eq!(out["decision"], json!("hold"));
                assert!(
                    crate::testkit::wait_for(3_000, || {
                        !crate::testkit::log_snapshot(&stack.created).is_empty()
                    })
                    .await
                );
                let created = crate::testkit::log_snapshot(&stack.created);
                assert_eq!(created.len(), 1);
                assert_eq!(created[0]["status"], json!("pending"));
            },
        )
        .await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn emitter_delivers_resolved_events() {
        crate::testkit::with_stack(
            crate::testkit::BootOpts::needs_approval(),
            |stack| async move {
                crate::testkit::state_set(
                    &stack.iii,
                    crate::pending::PENDING_SCOPE,
                    "s_1/c_1",
                    json!({
                        "session_id": "s_1",
                        "turn_id": "t_1",
                        "function_call_id": "c_1",
                        "function_id": "shell::run",
                        "kind": "function",
                        "arguments_excerpt": {},
                        "pending_at": 100,
                        "depth": 0,
                    }),
                )
                .await;
                crate::testkit::call(
                    &stack.iii,
                    "approval::resolve",
                    json!({
                        "session_id": "s_1",
                        "function_call_id": "c_1",
                        "decision": "allow"
                    }),
                )
                .await
                .unwrap();
                assert!(
                    crate::testkit::wait_for(3_000, || {
                        !crate::testkit::log_snapshot(&stack.resolved).is_empty()
                    })
                    .await
                );
                let resolved = crate::testkit::log_snapshot(&stack.resolved);
                assert_eq!(resolved[0]["outcome"], json!("allow"));
            },
        )
        .await;
    }

    #[test]
    fn subscriber_set_add_remove() {
        let set = SubscriberSet::new(PENDING_CREATED);
        set.add(trigger_config("b1", "f::1", json!({}))).unwrap();
        assert_eq!(set.len(), 1);
        set.remove("b1");
        assert!(set.is_empty());
        // Malformed config rejected.
        assert!(set
            .add(trigger_config("b2", "f::2", json!({ "bogus": 1 })))
            .is_err());
    }
}
