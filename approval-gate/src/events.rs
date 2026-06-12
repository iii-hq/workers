//! The two custom trigger types this worker emits —
//! `approval::pending_created` / `approval::pending_resolved` — and the
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
use iii_sdk::{IIIError, RegisterTriggerType, TriggerConfig, TriggerHandler, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::bus::Bus;
use crate::types::{metadata_matches, JsonMap, PendingApprovalRecord, PendingResolvedEvent};

pub const PENDING_CREATED: &str = "approval::pending_created";
pub const PENDING_RESOLVED: &str = "approval::pending_resolved";

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
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
        let id = config.id.clone();
        let function_id = config.function_id.clone();
        self.set.add(config).map_err(IIIError::Handler)?;
        tracing::info!(
            trigger_type = self.set.trigger_type(),
            id = %id,
            function_id = %function_id,
            "trigger subscription registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), IIIError> {
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
pub fn register_trigger_types(iii: &Arc<III>) -> TriggerSets {
    let sets = TriggerSets::new();

    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            PENDING_CREATED,
            "A function call was held for human approval and its inbox record written. \
             Payload: PendingApprovalRecord (redacted args, session context, expiry). \
             Bind notification workers here.",
            ApprovalTriggerHandler {
                set: sets.created.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            PENDING_RESOLVED,
            "A pending approval left the inbox (outcome: allow | deny | timeout | aborted). \
             Emitted exactly once per record; lets UIs clear badges.",
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

/// Where emissions go. Production fans out over the bus; tests record.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn pending_created(&self, record: &PendingApprovalRecord);
    async fn pending_resolved(&self, event: &PendingResolvedEvent);
}

/// Filtered fire-and-forget fan-out to every matching binding.
pub struct Emitter {
    sets: TriggerSets,
    bus: Arc<dyn Bus>,
}

impl Emitter {
    pub fn new(sets: TriggerSets, bus: Arc<dyn Bus>) -> Self {
        Self { sets, bus }
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
                self.bus
                    .call_void(&binding.function_id, payload.clone())
                    .await;
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

/// Test double: records every emission.
#[derive(Default)]
pub struct RecordingSink {
    pub created: Mutex<Vec<PendingApprovalRecord>>,
    pub resolved: Mutex<Vec<PendingResolvedEvent>>,
}

impl RecordingSink {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn created_events(&self) -> Vec<PendingApprovalRecord> {
        self.created
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }

    pub fn resolved_events(&self) -> Vec<PendingResolvedEvent> {
        self.resolved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .clone()
    }
}

#[async_trait]
impl EventSink for RecordingSink {
    async fn pending_created(&self, record: &PendingApprovalRecord) {
        self.created
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(record.clone());
    }

    async fn pending_resolved(&self, event: &PendingResolvedEvent) {
        self.resolved
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push(event.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testkit::FakeBus;
    use crate::types::ResolvedOutcome;
    use serde_json::json;

    fn trigger_config(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: function_id.into(),
            config,
            metadata: None,
        }
    }

    fn record(session_id: &str, metadata: Option<Value>) -> PendingApprovalRecord {
        PendingApprovalRecord {
            session_id: session_id.into(),
            turn_id: "t_1".into(),
            function_call_id: "c_1".into(),
            function_id: "shell::run".into(),
            arguments_excerpt: Value::Null,
            pending_at: 1,
            expires_at: 2,
            session_title: None,
            session_description: None,
            session_metadata: metadata.map(|m| serde_json::from_value(m).expect("metadata map")),
            depth: 0,
            assistant_excerpt: None,
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

    #[tokio::test]
    async fn emitter_delivers_to_matching_bindings_with_status() {
        let sets = TriggerSets::new();
        sets.created
            .add(trigger_config("b1", "notify::on_pending", json!({})))
            .unwrap();
        sets.created
            .add(trigger_config(
                "b2",
                "other::on_pending",
                json!({ "session_id": "different" }),
            ))
            .unwrap();

        let bus = Arc::new(FakeBus::new());
        let emitter = Emitter::new(sets, bus.clone());
        emitter
            .pending_created(&record("s_1", Some(json!({ "owner": "u_1" }))))
            .await;

        let delivered = bus.calls_to("notify::on_pending");
        assert_eq!(delivered.len(), 1);
        assert!(delivered[0].void);
        assert_eq!(delivered[0].payload["status"], json!("pending"));
        assert_eq!(delivered[0].payload["session_id"], json!("s_1"));
        assert!(bus.calls_to("other::on_pending").is_empty());
    }

    #[tokio::test]
    async fn emitter_delivers_resolved_events() {
        let sets = TriggerSets::new();
        sets.resolved
            .add(trigger_config("b1", "notify::on_resolved", json!(null)))
            .unwrap();
        let bus = Arc::new(FakeBus::new());
        let emitter = Emitter::new(sets, bus.clone());
        emitter
            .pending_resolved(&PendingResolvedEvent {
                session_id: "s_1".into(),
                turn_id: "t_1".into(),
                function_call_id: "c_1".into(),
                function_id: "shell::run".into(),
                outcome: ResolvedOutcome::Timeout,
                reason: None,
                session_metadata: None,
                resolved_at: 5,
            })
            .await;
        let delivered = bus.calls_to("notify::on_resolved");
        assert_eq!(delivered.len(), 1);
        assert_eq!(delivered[0].payload["outcome"], json!("timeout"));
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
