//! Worker-owned change notifications for the records this worker stores.
//!
//! Runs, reconciliation snapshots and finding actions live in the worker's
//! private `state` scopes and are read through `security-scan::list`,
//! `security-scan::read`, `security-scan::reconciliation` and
//! `security-scan::action-read`. After a compare-and-set commits a change, the
//! worker fires one of the trigger types below so consumers know to re-read.
//! The payload is a small public projection with the record id and its
//! `updated_at` revision hint, never the record itself: consumers bind first,
//! do an initial read, re-read on each notification, and re-read everything
//! after a reconnect, because notifications are not replayed.
//!
//! The worker owns the binding tables: filters are validated at registration,
//! the tables are capped per trigger type and per target function, and each
//! matching binding receives one fire-and-forget (`TriggerAction::Void`) send
//! carrying the binding's namespace and metadata. Nothing is spawned per event
//! and nothing is queued in this worker.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{RunRecordV1, RunStatusV1, SecurityActionRecordV1, SecurityActionStatusV1};

pub const RUN_CHANGED: &str = "security-scan::run-changed";
pub const RECONCILIATION_CHANGED: &str = "security-scan::reconciliation-changed";
pub const ACTION_CHANGED: &str = "security-scan::action-changed";

/// Bindings accepted per trigger type.
pub const MAX_BINDINGS_PER_TYPE: usize = 256;
/// Bindings accepted per trigger type for one target function id.
pub const MAX_BINDINGS_PER_FUNCTION: usize = 16;
/// Longest accepted filter value, in bytes.
pub const MAX_FILTER_VALUE_LEN: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ChangeKind {
    Run,
    Reconciliation,
    Action,
}

impl ChangeKind {
    pub const ALL: [ChangeKind; 3] = [
        ChangeKind::Run,
        ChangeKind::Reconciliation,
        ChangeKind::Action,
    ];

    pub fn trigger_type(self) -> &'static str {
        match self {
            ChangeKind::Run => RUN_CHANGED,
            ChangeKind::Reconciliation => RECONCILIATION_CHANGED,
            ChangeKind::Action => ACTION_CHANGED,
        }
    }

    fn description(self) -> &'static str {
        match self {
            ChangeKind::Run => {
                "A security scan run was created or changed. Payload { run_id, repository, status, \
                 attempt, updated_at, completed_at }; re-read security-scan::list or \
                 security-scan::read. Optional filter { repository, run_id }."
            }
            ChangeKind::Reconciliation => {
                "A run's GitHub alert reconciliation snapshot was saved. Payload { run_id, \
                 repository }; re-read security-scan::reconciliation. Optional filter \
                 { repository, run_id }."
            }
            ChangeKind::Action => {
                "A finding action (issue or fix) was created or changed. Payload { action_id, \
                 run_id, repository, status, updated_at }; re-read security-scan::action-read. \
                 Optional filter { repository, run_id }."
            }
        }
    }
}

/// Binding `config` accepted by every `security-scan::*-changed` trigger
/// type. Both fields are optional equality filters; unknown keys are
/// rejected so a misspelled filter fails at registration instead of
/// silently receiving nothing.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeFilterV1 {
    /// Only changes for this configured repository name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repository: Option<String>,
    /// Only changes for this run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
}

impl ChangeFilterV1 {
    pub fn parse(kind: ChangeKind, raw: &Value) -> Result<Self, String> {
        let raw = if raw.is_null() {
            Value::Object(serde_json::Map::new())
        } else {
            raw.clone()
        };
        let filter: ChangeFilterV1 = serde_json::from_value(raw)
            .map_err(|error| format!("invalid {} config: {error}", kind.trigger_type()))?;
        for (field, value) in [
            ("repository", &filter.repository),
            ("run_id", &filter.run_id),
        ] {
            if let Some(value) = value {
                if value.trim().is_empty() || value.len() > MAX_FILTER_VALUE_LEN {
                    return Err(format!(
                        "invalid {} config: `{field}` must contain 1 to {MAX_FILTER_VALUE_LEN} bytes",
                        kind.trigger_type()
                    ));
                }
            }
        }
        Ok(filter)
    }

    pub fn matches(&self, repository: &str, run_id: &str) -> bool {
        self.repository
            .as_deref()
            .is_none_or(|want| want == repository)
            && self.run_id.as_deref().is_none_or(|want| want == run_id)
    }
}

/// `security-scan::run-changed` payload: the public status projection.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunChangedEventV1 {
    pub run_id: String,
    pub repository: String,
    pub status: RunStatusV1,
    pub attempt: u32,
    /// Revision hint: set on every committed write (ms).
    pub updated_at: i64,
    pub completed_at: Option<i64>,
}

impl From<&RunRecordV1> for RunChangedEventV1 {
    fn from(run: &RunRecordV1) -> Self {
        Self {
            run_id: run.run_id.clone(),
            repository: run.repository.clone(),
            status: run.status,
            attempt: run.attempt,
            updated_at: run.updated_at,
            completed_at: run.completed_at,
        }
    }
}

/// `security-scan::reconciliation-changed` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ReconciliationChangedEventV1 {
    pub run_id: String,
    pub repository: String,
}

/// `security-scan::action-changed` payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ActionChangedEventV1 {
    pub action_id: String,
    pub run_id: String,
    pub repository: String,
    pub status: SecurityActionStatusV1,
    /// Revision hint: set on every committed write (ms).
    pub updated_at: i64,
}

impl From<&SecurityActionRecordV1> for ActionChangedEventV1 {
    fn from(action: &SecurityActionRecordV1) -> Self {
        Self {
            action_id: action.action_id.clone(),
            run_id: action.run_id.clone(),
            repository: action.repository.clone(),
            status: action.status,
            updated_at: action.updated_at,
        }
    }
}

/// One send to one binding.
#[derive(Debug, Clone, PartialEq)]
pub struct Delivery {
    pub trigger_type: &'static str,
    pub binding_id: String,
    pub function_id: String,
    pub namespace: Option<String>,
    pub metadata: Option<Value>,
    pub payload: Value,
}

/// How a matched notification reaches its binding. Production sends over
/// the engine connection; tests record.
#[async_trait]
pub trait ChangeDeliverer: Send + Sync {
    async fn deliver(&self, delivery: Delivery) -> Result<(), String>;
}

/// Fire-and-forget delivery over the worker's engine connection. `Void`
/// only enqueues the invocation on the SDK connection, so the write path is
/// never blocked on a consumer.
pub struct IiiChangeDeliverer {
    iii: Arc<IIIClient>,
}

impl IiiChangeDeliverer {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

#[async_trait]
impl ChangeDeliverer for IiiChangeDeliverer {
    async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
        let request = TriggerRequest {
            function_id: delivery.function_id,
            payload: delivery.payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        };
        let result = match (delivery.namespace, delivery.metadata) {
            (Some(namespace), Some(metadata)) => {
                self.iii
                    .trigger(request.namespace(namespace).metadata(metadata))
                    .await
            }
            (Some(namespace), None) => self.iii.trigger(request.namespace(namespace)).await,
            (None, Some(metadata)) => self.iii.trigger(request.metadata(metadata)).await,
            (None, None) => self.iii.trigger(request).await,
        };
        result.map(|_| ()).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    namespace: Option<String>,
    metadata: Option<Value>,
    filter: ChangeFilterV1,
}

type BindingTables = HashMap<ChangeKind, HashMap<String, Binding>>;

/// Binding tables for the three trigger types plus the emitter.
pub struct ChangeFeed {
    tables: Mutex<BindingTables>,
    deliverer: Arc<dyn ChangeDeliverer>,
}

impl ChangeFeed {
    pub fn new(deliverer: Arc<dyn ChangeDeliverer>) -> Self {
        Self {
            tables: Mutex::new(HashMap::new()),
            deliverer,
        }
    }

    /// Validates and stores a binding. Re-registering an existing id
    /// replaces it in place without counting against the caps.
    pub fn add(&self, kind: ChangeKind, config: TriggerConfig) -> Result<(), String> {
        let filter = ChangeFilterV1::parse(kind, &config.config)?;
        if config.function_id.trim().is_empty() {
            return Err(format!(
                "{} binding needs a target function id",
                kind.trigger_type()
            ));
        }
        let mut tables = self.lock();
        let table = tables.entry(kind).or_default();
        if !table.contains_key(&config.id) {
            if table.len() >= MAX_BINDINGS_PER_TYPE {
                return Err(format!(
                    "{} accepts at most {MAX_BINDINGS_PER_TYPE} bindings",
                    kind.trigger_type()
                ));
            }
            let per_function = table
                .values()
                .filter(|binding| binding.function_id == config.function_id)
                .count();
            if per_function >= MAX_BINDINGS_PER_FUNCTION {
                return Err(format!(
                    "{} accepts at most {MAX_BINDINGS_PER_FUNCTION} bindings per function",
                    kind.trigger_type()
                ));
            }
        }
        table.insert(
            config.id,
            Binding {
                function_id: config.function_id,
                namespace: config.namespace,
                metadata: config.metadata,
                filter,
            },
        );
        Ok(())
    }

    pub fn remove(&self, kind: ChangeKind, id: &str) -> bool {
        self.lock()
            .get_mut(&kind)
            .is_some_and(|table| table.remove(id).is_some())
    }

    pub fn binding_count(&self, kind: ChangeKind) -> usize {
        self.lock().get(&kind).map_or(0, HashMap::len)
    }

    /// Sends `event` to every binding of `kind` whose filter matches. Call
    /// only after the change is committed. Returns how many sends were
    /// accepted; failures are logged and never fail the caller's write.
    pub async fn emit<T: Serialize>(
        &self,
        kind: ChangeKind,
        repository: &str,
        run_id: &str,
        event: &T,
    ) -> usize {
        // Snapshot so the lock is never held across an await.
        let matched: Vec<(String, Binding)> = self
            .lock()
            .get(&kind)
            .map(|table| {
                table
                    .iter()
                    .filter(|(_, binding)| binding.filter.matches(repository, run_id))
                    .map(|(id, binding)| (id.clone(), binding.clone()))
                    .collect()
            })
            .unwrap_or_default();
        if matched.is_empty() {
            return 0;
        }
        let payload = match serde_json::to_value(event) {
            Ok(payload) => payload,
            Err(error) => {
                tracing::warn!(trigger_type = kind.trigger_type(), %error, "change payload failed to serialize");
                return 0;
            }
        };
        let mut delivered = 0;
        for (binding_id, binding) in matched {
            let delivery = Delivery {
                trigger_type: kind.trigger_type(),
                binding_id,
                function_id: binding.function_id,
                namespace: binding.namespace,
                metadata: binding.metadata,
                payload: payload.clone(),
            };
            let binding_id = delivery.binding_id.clone();
            match self.deliverer.deliver(delivery).await {
                Ok(()) => delivered += 1,
                Err(error) => tracing::warn!(
                    trigger_type = kind.trigger_type(),
                    %binding_id,
                    %error,
                    "security scan change notification failed"
                ),
            }
        }
        delivered
    }

    fn lock(&self) -> MutexGuard<'_, BindingTables> {
        self.tables
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The engine-facing handler for one trigger type.
pub struct ChangeTriggerHandler {
    kind: ChangeKind,
    feed: Arc<ChangeFeed>,
}

impl ChangeTriggerHandler {
    pub fn new(kind: ChangeKind, feed: Arc<ChangeFeed>) -> Self {
        Self { kind, feed }
    }
}

#[async_trait]
impl TriggerHandler for ChangeTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        self.feed.add(self.kind, config).map_err(Error::Handler)?;
        tracing::debug!(trigger_type = self.kind.trigger_type(), %id, "change binding registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.feed.remove(self.kind, &config.id);
        tracing::debug!(trigger_type = self.kind.trigger_type(), id = %config.id, "change binding unregistered");
        Ok(())
    }
}

/// Registers the three trigger types. Call before functions and UI so
/// consumers registering early resolve against them.
pub fn register_trigger_types(iii: &IIIClient, feed: &Arc<ChangeFeed>) {
    for kind in ChangeKind::ALL {
        let _ = iii.register_trigger_type(
            RegisterTriggerType::new(
                kind.trigger_type(),
                kind.description(),
                ChangeTriggerHandler::new(kind, feed.clone()),
            )
            .trigger_request_format::<ChangeFilterV1>(),
        );
    }
}

/// Unregisters the three trigger types; call on shutdown.
pub fn unregister_trigger_types(iii: &IIIClient) {
    for kind in ChangeKind::ALL {
        iii.unregister_trigger_type(kind.trigger_type());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Default)]
    struct Recorder {
        seen: Mutex<Vec<Delivery>>,
        fail_function: Option<String>,
    }

    #[async_trait]
    impl ChangeDeliverer for Recorder {
        async fn deliver(&self, delivery: Delivery) -> Result<(), String> {
            if self.fail_function.as_deref() == Some(delivery.function_id.as_str()) {
                return Err("not connected".into());
            }
            self.seen.lock().unwrap().push(delivery);
            Ok(())
        }
    }

    fn binding(id: &str, function_id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: function_id.into(),
            config,
            metadata: None,
            namespace: None,
        }
    }

    fn event() -> ReconciliationChangedEventV1 {
        ReconciliationChangedEventV1 {
            run_id: "sec_1".into(),
            repository: "iii".into(),
        }
    }

    #[test]
    fn filter_accepts_null_and_empty_and_rejects_bad_values() {
        let kind = ChangeKind::Run;
        assert_eq!(
            ChangeFilterV1::parse(kind, &Value::Null).unwrap(),
            ChangeFilterV1::default()
        );
        assert_eq!(
            ChangeFilterV1::parse(kind, &json!({})).unwrap(),
            ChangeFilterV1::default()
        );
        for bad in [
            json!({ "repo": "iii" }),
            json!({ "stream_name": "security-scan:runs" }),
            json!({ "repository": 7 }),
            json!({ "repository": "  " }),
            json!({ "run_id": "" }),
            json!({ "run_id": "x".repeat(MAX_FILTER_VALUE_LEN + 1) }),
            json!("iii"),
        ] {
            let error = ChangeFilterV1::parse(kind, &bad).unwrap_err();
            assert!(error.contains(RUN_CHANGED), "{error}");
        }
    }

    #[test]
    fn filter_is_equality_on_every_present_field() {
        let both = ChangeFilterV1 {
            repository: Some("iii".into()),
            run_id: Some("sec_1".into()),
        };
        assert!(both.matches("iii", "sec_1"));
        assert!(!both.matches("iii", "sec_2"));
        assert!(!both.matches("other", "sec_1"));
        assert!(ChangeFilterV1::default().matches("any", "thing"));
    }

    #[tokio::test]
    async fn emits_only_to_matching_bindings_with_namespace_and_metadata() {
        let recorder = Arc::new(Recorder::default());
        let feed = ChangeFeed::new(recorder.clone());
        let mut scoped = binding("b-scoped", "ui::scoped", json!({ "repository": "iii" }));
        scoped.namespace = Some("tenant-a".into());
        scoped.metadata = Some(json!({ "tab": 1 }));
        feed.add(ChangeKind::Reconciliation, scoped).unwrap();
        feed.add(
            ChangeKind::Reconciliation,
            binding("b-other", "ui::other", json!({ "repository": "other" })),
        )
        .unwrap();
        feed.add(ChangeKind::Run, binding("b-run", "ui::run", json!({})))
            .unwrap();

        let sent = feed
            .emit(ChangeKind::Reconciliation, "iii", "sec_1", &event())
            .await;

        assert_eq!(sent, 1);
        let seen = recorder.seen.lock().unwrap().clone();
        assert_eq!(
            seen,
            vec![Delivery {
                trigger_type: RECONCILIATION_CHANGED,
                binding_id: "b-scoped".into(),
                function_id: "ui::scoped".into(),
                namespace: Some("tenant-a".into()),
                metadata: Some(json!({ "tab": 1 })),
                payload: json!({ "run_id": "sec_1", "repository": "iii" }),
            }]
        );
    }

    #[tokio::test]
    async fn unregister_stops_delivery() {
        let recorder = Arc::new(Recorder::default());
        let feed = ChangeFeed::new(recorder.clone());
        feed.add(
            ChangeKind::Action,
            binding("b1", "ui::actions", Value::Null),
        )
        .unwrap();
        assert_eq!(
            feed.emit(ChangeKind::Action, "iii", "sec_1", &event())
                .await,
            1
        );
        assert!(feed.remove(ChangeKind::Action, "b1"));
        assert!(!feed.remove(ChangeKind::Action, "b1"));
        assert_eq!(
            feed.emit(ChangeKind::Action, "iii", "sec_1", &event())
                .await,
            0
        );
        assert_eq!(recorder.seen.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_failed_send_does_not_stop_other_bindings() {
        let recorder = Arc::new(Recorder {
            seen: Mutex::new(Vec::new()),
            fail_function: Some("ui::gone".into()),
        });
        let feed = ChangeFeed::new(recorder.clone());
        feed.add(ChangeKind::Run, binding("b-gone", "ui::gone", json!({})))
            .unwrap();
        feed.add(ChangeKind::Run, binding("b-live", "ui::live", json!({})))
            .unwrap();
        assert_eq!(
            feed.emit(ChangeKind::Run, "iii", "sec_1", &event()).await,
            1
        );
        let seen = recorder.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].function_id, "ui::live");
    }

    #[test]
    fn caps_bindings_per_function_and_per_type_and_reregisters_in_place() {
        let feed = ChangeFeed::new(Arc::new(Recorder::default()));
        for index in 0..MAX_BINDINGS_PER_FUNCTION {
            feed.add(
                ChangeKind::Run,
                binding(&format!("same-{index}"), "ui::same", json!({})),
            )
            .unwrap();
        }
        let error = feed
            .add(ChangeKind::Run, binding("same-over", "ui::same", json!({})))
            .unwrap_err();
        assert!(error.contains("per function"), "{error}");
        // Re-registration of a known id (engine reconnect) replaces in place.
        feed.add(
            ChangeKind::Run,
            binding("same-0", "ui::same", json!({ "run_id": "sec_1" })),
        )
        .unwrap();
        assert_eq!(
            feed.binding_count(ChangeKind::Run),
            MAX_BINDINGS_PER_FUNCTION
        );

        for index in feed.binding_count(ChangeKind::Run)..MAX_BINDINGS_PER_TYPE {
            feed.add(
                ChangeKind::Run,
                binding(&format!("b-{index}"), &format!("ui::f{index}"), json!({})),
            )
            .unwrap();
        }
        let error = feed
            .add(ChangeKind::Run, binding("b-over", "ui::fresh", json!({})))
            .unwrap_err();
        assert!(error.contains("at most 256 bindings"), "{error}");
        // Caps are per trigger type.
        feed.add(
            ChangeKind::Action,
            binding("b-over", "ui::fresh", json!({})),
        )
        .unwrap();
    }

    #[test]
    fn rejects_a_binding_without_a_target_function() {
        let feed = ChangeFeed::new(Arc::new(Recorder::default()));
        assert!(feed
            .add(ChangeKind::Run, binding("b1", " ", json!({})))
            .is_err());
        assert_eq!(feed.binding_count(ChangeKind::Run), 0);
    }

    #[tokio::test]
    async fn handler_routes_engine_callbacks_into_the_feed() {
        let recorder = Arc::new(Recorder::default());
        let feed = Arc::new(ChangeFeed::new(recorder.clone()));
        let handler = ChangeTriggerHandler::new(ChangeKind::Run, feed.clone());
        let error = handler
            .register_trigger(binding("b1", "ui::runs", json!({ "bogus": true })))
            .await
            .unwrap_err();
        assert!(error.to_string().contains(RUN_CHANGED), "{error}");
        handler
            .register_trigger(binding("b1", "ui::runs", json!({})))
            .await
            .unwrap();
        assert_eq!(feed.binding_count(ChangeKind::Run), 1);
        handler
            .unregister_trigger(binding("b1", "ui::runs", json!({})))
            .await
            .unwrap();
        assert_eq!(feed.binding_count(ChangeKind::Run), 0);
    }

    #[test]
    fn trigger_type_ids_are_owned_by_the_worker() {
        assert_eq!(
            ChangeKind::ALL.map(ChangeKind::trigger_type),
            [
                "security-scan::run-changed",
                "security-scan::reconciliation-changed",
                "security-scan::action-changed",
            ]
        );
    }
}
