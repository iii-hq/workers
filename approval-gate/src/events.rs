//! The two custom trigger types this worker emits —
//! `approval::pending-created` / `approval::pending-resolved` — the
//! observable functions next to them (iii 0.25) —
//! `approval::on-pending-created` / `approval::on-pending-resolved` — and the
//! fan-out machinery behind both (the session-manager reactivity model:
//! consumers bind handlers with the standard two-step pattern, or observe the
//! function on a 0.25 engine; the engine routes each registration to our
//! [`TriggerHandler`]s and each observer to our [`ObserveHandler`]s; delivery
//! is fire-and-forget, unordered; `list_pending` is the reconciliation read).
//!
//! Per-binding `config` filters (and the observers' `args`, which share the
//! same [`BindingConfig`] shape) are evaluated here, by the emitting worker:
//! `session_id` equality and `metadata` subset-equality against the record's
//! denormalized `session_metadata` (the tenancy hook). Malformed configs are
//! rejected at registration time.
//!
//! Events reach observers in-process, from the same fan-out that serves the
//! trigger bindings, with `obs.call`: the observable functions' own handlers
//! refuse every invocation (`FORBIDDEN`), so no caller can make this worker
//! deliver a forged event.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{observe_codes, TriggerRequest};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType, TriggerAction,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
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

/// The observable function next to [`PENDING_CREATED`].
pub const ON_PENDING_CREATED: &str = "approval::on-pending-created";
pub const ON_PENDING_CREATED_DESC: &str =
    "Fires when a function call is held for human approval and its inbox record is written. \
     Observe it with args { session_id?, metadata? } (the approval::pending-created binding \
     config); every matching observer receives PendingApprovalRecord & { status: \"pending\" }. \
     Only the approval-gate worker fires it: invoking it directly is refused.";

/// The observable function next to [`PENDING_RESOLVED`].
pub const ON_PENDING_RESOLVED: &str = "approval::on-pending-resolved";
pub const ON_PENDING_RESOLVED_DESC: &str =
    "Fires when a pending approval leaves the inbox (outcome: allow | deny | timeout | aborted), \
     exactly once per record. Observe it with args { session_id?, metadata? } (the \
     approval::pending-resolved binding config); every matching observer receives the \
     PendingResolvedEvent. Only the approval-gate worker fires it: invoking it directly is \
     refused.";

/// Config accepted by both trigger types, and the args of both observable
/// functions (`args_format`).
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
    pub namespace: Option<String>,
    pub filter: BindingConfig,
}

/// One observer of an `approval::on-*` function, with its args parsed as
/// the trigger type parses a binding config.
#[derive(Debug, Clone)]
pub struct ObserverBinding {
    pub observer: Arc<dyn EventObserver>,
    pub filter: BindingConfig,
}

/// One observer as a sink of a [`SubscriberSet`]: the SDK's [`Observer`] in
/// production, a recorder in tests (the fan-out runs without an engine).
#[async_trait]
pub trait EventObserver: std::fmt::Debug + Send + Sync + 'static {
    fn id(&self) -> &str;
    fn target(&self) -> &str;
    /// The observer's args, parsed as a binding config.
    fn args_value(&self) -> &Value;
    /// Hands one event to the observer's target (`obs.call`).
    async fn deliver(&self, payload: Value) -> Result<(), Error>;
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn target(&self) -> &str {
        Observer::target(self)
    }

    fn args_value(&self) -> &Value {
        Observer::args_value(self)
    }

    async fn deliver(&self, payload: Value) -> Result<(), Error> {
        self.call::<Value>(payload).await.map(|_| ())
    }
}

/// Thread-safe subscriber registry for one trigger type and the observable
/// function next to it. Cloned into the [`TriggerHandler`] (mutates bindings
/// on register/unregister), the [`ObserveHandler`] (mutates observers on
/// add/remove) and the [`Emitter`] (iterates read-only snapshots). After a
/// worker restart the engine replays existing registrations and observers,
/// so the sets rebuild themselves.
#[derive(Clone)]
pub struct SubscriberSet {
    trigger_type: &'static str,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverBinding>>>,
}

impl SubscriberSet {
    pub fn new(trigger_type: &'static str) -> Self {
        Self {
            trigger_type,
            inner: Arc::new(Mutex::new(HashMap::new())),
            observers: Arc::new(Mutex::new(HashMap::new())),
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
            namespace: config.namespace,
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

    /// Trigger bindings only; see [`observer_count`](Self::observer_count).
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.lock().is_empty()
    }

    /// Files an observer under its id; its args are parsed exactly as a
    /// binding config (`null` = match all, unknown fields rejected).
    pub fn add_observer(&self, observer: Arc<dyn EventObserver>) -> Result<(), String> {
        let filter = parse_binding_config(observer.args_value())?;
        let id = observer.id().to_string();
        self.lock_observers()
            .insert(id, ObserverBinding { observer, filter });
        Ok(())
    }

    pub fn remove_observer(&self, id: &str) {
        self.lock_observers().remove(id);
    }

    /// Snapshot so the mutex isn't held across awaits.
    pub fn observer_snapshot(&self) -> Vec<ObserverBinding> {
        self.lock_observers().values().cloned().collect()
    }

    pub fn observer_count(&self) -> usize {
        self.lock_observers().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_observers(&self) -> std::sync::MutexGuard<'_, HashMap<String, ObserverBinding>> {
        self.observers
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

/// Files each observer of an `approval::on-*` function in the subscriber set
/// its trigger type fans out from, and takes it out when the engine removes
/// it.
struct ApprovalObserveHandler {
    function_id: &'static str,
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for ApprovalObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format; parsed again here so a
        // config the trigger type would refuse is refused the same way.
        self.set
            .add_observer(Arc::new(obs.clone()))
            .map_err(|e| ObserveRejection::new(observe_codes::INVALID_ARGS, e))?;
        tracing::info!(
            function_id = self.function_id,
            observer = %obs.id(),
            target = %obs.target(),
            "observer added"
        );
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
        tracing::info!(
            function_id = self.function_id,
            observer = %obs.id(),
            "observer removed"
        );
    }
}

/// The error every direct invocation of an `approval::on-*` function gets:
/// events only come from this worker's own fan-out.
pub fn forbidden(function_id: &str) -> Error {
    Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args: {{ session_id?, metadata? }} }}, \
             target)); only the approval-gate worker fires its events"
        ),
        stacktrace: None,
    }
}

/// The body of an `approval::on-*` function: refuses every event handed to
/// it with [`forbidden`]. Its parameter is the event its observers receive,
/// so the published request schema documents the delivered shape (an input
/// that does not even parse as that event is refused by the SDK with a
/// deserialization error before reaching here).
pub fn refuse_invocation<Event>(function_id: &str, _event: Event) -> Result<(), Error> {
    Err(forbidden(function_id))
}

/// The handler registered for an `approval::on-*` function: typed on the
/// delivered `Event` (request schema) and `()` (response schema).
pub fn observable_handler<Event>(
    function_id: &'static str,
) -> impl Fn(Event) -> Result<(), Error> + Send + Sync + 'static {
    move |event: Event| refuse_invocation(function_id, event)
}

/// Registers one owner-delivered `approval::on-*` function whose observers
/// share `set` with the trigger type next to it. Its request schema (derived
/// from the handler's `Event` parameter) documents the event its observers
/// receive; on an engine without observe it registers as a plain function
/// and its hooks never run.
fn register_observable<Event>(
    iii: &IIIClient,
    function_id: &'static str,
    description: &'static str,
    set: SubscriberSet,
) where
    Event: DeserializeOwned + JsonSchema + Send + 'static,
{
    iii.register_function(
        function_id,
        RegisterFunction::new(observable_handler::<Event>(function_id))
            .description(description)
            .observe(
                Observe::owner()
                    .args_format::<BindingConfig>()
                    .handler(ApprovalObserveHandler { function_id, set }),
            ),
    );
}

/// Register both custom trigger types and the observable function next to
/// each with the engine. Must run **before** `functions::register_all` so
/// the handlers capture the subscriber sets they fan out to.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets::new();

    // Kept until the trigger types are retired; old consumers bind them on
    // 0.24 and 0.25 engines.
    #[allow(deprecated)]
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
    #[allow(deprecated)]
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

    register_observable::<PendingApprovalRecord>(
        iii,
        ON_PENDING_CREATED,
        ON_PENDING_CREATED_DESC,
        sets.created.clone(),
    );
    register_observable::<PendingResolvedEvent>(
        iii,
        ON_PENDING_RESOLVED,
        ON_PENDING_RESOLVED_DESC,
        sets.resolved.clone(),
    );

    tracing::info!(
        trigger_types = ?[PENDING_CREATED, PENDING_RESOLVED],
        observable_functions = ?[ON_PENDING_CREATED, ON_PENDING_RESOLVED],
        "registered trigger types and observable functions"
    );
    sets
}

/// Where emissions go. Production fans out via iii; tests record.
#[async_trait]
pub trait EventSink: Send + Sync {
    async fn pending_created(&self, record: &PendingApprovalRecord);
    async fn pending_resolved(&self, event: &PendingResolvedEvent);
}

/// Filtered fire-and-forget fan-out to every matching binding and observer.
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
                let namespace = binding.namespace.as_deref().unwrap_or("default");
                let res = self.iii.trigger(request.namespace(namespace)).await;
                if let Err(e) = res {
                    tracing::warn!(
                        function_id = %binding.function_id,
                        error = %e,
                        "void fan-out failed"
                    );
                }
            }
        }
        // Observers get the same payload under the same filter, through
        // `obs.call` (the SDK applies their own filters too). Each call runs
        // on its own task, in this invocation's context, so a slow target
        // holds up neither the caller nor the other sinks.
        let ctx = iii_sdk::current_invocation();
        for entry in set.observer_snapshot() {
            if !binding_matches(&entry.filter, session_id, session_metadata) {
                continue;
            }
            let payload = payload.clone();
            tokio::spawn(iii_sdk::scope(ctx.clone(), async move {
                match entry.observer.deliver(payload).await {
                    Ok(()) => {}
                    Err(Error::Remote { code, .. })
                        if code == observe_codes::FILTERS_NOT_MATCHED => {}
                    Err(e) => tracing::warn!(
                        observer = %entry.observer.id(),
                        target = %entry.observer.target(),
                        error = %e,
                        "observer delivery failed"
                    ),
                }
            }));
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
        let mut config = trigger_config("b1", "f::1", json!({}));
        config.namespace = Some("project-a".into());
        set.add(config).unwrap();
        assert_eq!(set.len(), 1);
        assert_eq!(set.snapshot()[0].namespace.as_deref(), Some("project-a"));
        set.remove("b1");
        assert!(set.is_empty());
        // Malformed config rejected.
        assert!(set
            .add(trigger_config("b2", "f::2", json!({ "bogus": 1 })))
            .is_err());
    }

    /// An `approval::on-*` observer that records what it is handed.
    #[derive(Debug)]
    struct Recorder {
        id: String,
        args: Value,
        events: tokio::sync::mpsc::UnboundedSender<Value>,
    }

    #[async_trait]
    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn target(&self) -> &str {
            "test::target"
        }

        fn args_value(&self) -> &Value {
            &self.args
        }

        async fn deliver(&self, payload: Value) -> Result<(), Error> {
            let _ = self.events.send(payload);
            Ok(())
        }
    }

    fn recorder(
        id: &str,
        args: Value,
    ) -> (
        Arc<dyn EventObserver>,
        tokio::sync::mpsc::UnboundedReceiver<Value>,
    ) {
        let (events, received) = tokio::sync::mpsc::unbounded_channel();
        let observer = Arc::new(Recorder {
            id: id.to_string(),
            args,
            events,
        });
        (observer, received)
    }

    fn offline_emitter(sets: TriggerSets) -> Emitter {
        // Never connects; observers are served in-process.
        Emitter::new(
            sets,
            Arc::new(iii_sdk::register_worker(
                "ws://127.0.0.1:9",
                iii_sdk::InitOptions::default(),
            )),
        )
    }

    async fn next_event(received: &mut tokio::sync::mpsc::UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), received.recv())
            .await
            .expect("the observer is handed the event")
            .expect("one event")
    }

    fn pending_record(session_id: &str) -> PendingApprovalRecord {
        serde_json::from_value(json!({
            "session_id": session_id,
            "turn_id": "t_1",
            "function_call_id": "c_1",
            "function_id": "shell::run",
            "kind": "function",
            "arguments_excerpt": {},
            "pending_at": 100,
            "depth": 0,
        }))
        .unwrap()
    }

    fn resolved_event() -> PendingResolvedEvent {
        serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "function_call_id": "c_1",
            "function_id": "shell::run",
            "outcome": "allow",
            "resolved_at": 200,
        }))
        .unwrap()
    }

    fn assert_forbidden(function_id: &str, result: Result<(), Error>) {
        let err = result.unwrap_err();
        assert!(
            matches!(&err, Error::Remote { code, message, .. }
                if code == "FORBIDDEN" && message.contains(function_id)),
            "{err:?}"
        );
    }

    #[test]
    fn on_functions_refuse_direct_invocation() {
        let created = observable_handler::<PendingApprovalRecord>(ON_PENDING_CREATED);
        assert_forbidden(ON_PENDING_CREATED, created(pending_record("s_1")));
        let resolved = observable_handler::<PendingResolvedEvent>(ON_PENDING_RESOLVED);
        assert_forbidden(ON_PENDING_RESOLVED, resolved(resolved_event()));
        assert_eq!(ON_PENDING_CREATED, "approval::on-pending-created");
        assert_eq!(ON_PENDING_RESOLVED, "approval::on-pending-resolved");
    }

    /// Request / response schemas of a handler, as iii-sdk extracts them
    /// from its parameter and `Ok` types at registration.
    fn handler_schemas<F, T, R>(_handler: &F) -> (Value, Value)
    where
        F: Fn(T) -> Result<R, Error>,
        T: JsonSchema,
        R: JsonSchema,
    {
        fn schema<S: JsonSchema>() -> Value {
            serde_json::to_value(
                schemars::r#gen::SchemaSettings::draft07()
                    .into_generator()
                    .into_root_schema_for::<S>(),
            )
            .unwrap()
        }
        (schema::<T>(), schema::<R>())
    }

    fn assert_typed(label: &str, schema: &Value) {
        const DEFINING: [&str; 7] = [
            "type",
            "properties",
            "$ref",
            "allOf",
            "anyOf",
            "oneOf",
            "enum",
        ];
        let obj = schema.as_object().expect("schema object");
        assert!(
            DEFINING.iter().any(|k| obj.contains_key(*k)),
            "{label}: untyped (AnyValue/empty) schema: {schema}"
        );
        assert_ne!(obj.get("title"), Some(&json!("AnyValue")), "{label}");
    }

    #[test]
    fn on_function_handlers_publish_typed_schemas() {
        let created = observable_handler::<PendingApprovalRecord>(ON_PENDING_CREATED);
        let resolved = observable_handler::<PendingResolvedEvent>(ON_PENDING_RESOLVED);
        let (created_req, created_res) = handler_schemas(&created);
        let (resolved_req, resolved_res) = handler_schemas(&resolved);
        for (function_id, request, response) in [
            (ON_PENDING_CREATED, created_req, created_res),
            (ON_PENDING_RESOLVED, resolved_req, resolved_res),
        ] {
            assert_typed(&format!("{function_id} request_schema"), &request);
            assert_typed(&format!("{function_id} response_schema"), &response);
            assert!(
                request["properties"]["session_id"].is_object(),
                "{function_id}: request schema documents the delivered event"
            );
        }
    }

    #[test]
    fn observer_args_are_parsed_as_a_binding_config() {
        let set = SubscriberSet::new(PENDING_CREATED);
        let (all, _rx) = recorder("o_all", Value::Null);
        set.add_observer(all).unwrap();
        let (bad, _rx) = recorder("o_bad", json!({ "bogus": 1 }));
        assert!(
            set.add_observer(bad).is_err(),
            "a config a binding would refuse"
        );
        assert_eq!(set.observer_count(), 1);
        assert!(set.is_empty(), "observers are not bindings");
        set.remove_observer("o_all");
        assert_eq!(set.observer_count(), 0);
    }

    #[tokio::test]
    async fn an_observer_receives_the_created_event_a_binding_receives() {
        let sets = TriggerSets::new();
        let (matching, mut matching_rx) = recorder("o_1", json!({ "session_id": "s_1" }));
        let (other, mut other_rx) = recorder("o_2", json!({ "session_id": "s_2" }));
        sets.created.add_observer(matching).unwrap();
        sets.created.add_observer(other).unwrap();
        let emitter = offline_emitter(sets);

        let record = pending_record("s_1");
        emitter.pending_created(&record).await;

        // The payload bindings get: PendingApprovalRecord & { status: "pending" }.
        let mut expected = serde_json::to_value(&record).unwrap();
        expected["status"] = json!("pending");
        assert_eq!(next_event(&mut matching_rx).await, expected);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(other_rx.try_recv().is_err(), "filtered like a binding");
    }

    #[tokio::test]
    async fn an_observer_receives_the_resolved_event_a_binding_receives() {
        let sets = TriggerSets::new();
        let (observer, mut received) = recorder("o_1", json!({}));
        sets.resolved.add_observer(observer).unwrap();
        let (created_only, mut created_rx) = recorder("o_c", json!({}));
        sets.created.add_observer(created_only).unwrap();
        let emitter = offline_emitter(sets);

        let event: PendingResolvedEvent = serde_json::from_value(json!({
            "session_id": "s_1",
            "turn_id": "t_1",
            "function_call_id": "c_1",
            "function_id": "shell::run",
            "outcome": "allow",
            "resolved_at": 200,
        }))
        .unwrap();
        emitter.pending_resolved(&event).await;

        assert_eq!(
            next_event(&mut received).await,
            serde_json::to_value(&event).unwrap()
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        assert!(
            created_rx.try_recv().is_err(),
            "each function has its own set"
        );
    }
}
