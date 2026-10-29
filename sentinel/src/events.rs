//! `sentinel::group-changed` — the surface siblings and the console watch —
//! and, next to each trigger type, the observable function an iii 0.25
//! consumer observes instead: `sentinel::on-group-changed` and
//! `sentinel::on-investigation-changed`.
//!
//! The event is a doorbell, not a record: a subscriber re-reads the group it
//! names rather than trusting the payload, which is what keeps a page correct
//! when two events cross. Occurrence events are coalesced per group over a
//! second, because an incident with a thousand occurrences must not become a
//! thousand deliveries — while a state change goes out immediately, since
//! that is the one an alert cares about.
//!
//! A trigger type and its observable function share one [`Table`]: a binding
//! config and an observer's args have the same shape and filter the same
//! way, and events reach observers from the same fan-out, with `obs.call`.
//! The observable functions' own handlers refuse every invocation
//! (`FORBIDDEN`), so no caller can make this worker deliver a forged event.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{observe_codes, TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::triggers::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde_json::Value;
use tokio::sync::Mutex;

use crate::{
    GroupChangedConfigV1, GroupChangedEventV1, GroupChangedOpV1, GroupStatusV1,
    InvestigationChangedEventV1,
};

pub const GROUP_CHANGED: &str = "sentinel::group-changed";
pub const INVESTIGATION_CHANGED: &str = "sentinel::investigation-changed";

/// The observable function next to [`GROUP_CHANGED`].
pub const ON_GROUP_CHANGED: &str = "sentinel::on-group-changed";
pub const ON_GROUP_CHANGED_DESC: &str =
    "Fires when an error group is created, takes a new occurrence, or changes state. \
     Observe it with args { group_id?, service_name?, ops? } (the sentinel::group-changed \
     binding config; ops narrows to created, occurrence or status); every matching observer \
     receives the GroupChangedEventV1. Occurrence events are coalesced per group over one \
     second; re-read the group rather than trusting the payload. Only the sentinel worker \
     fires it: invoking it directly is refused.";

/// The observable function next to [`INVESTIGATION_CHANGED`].
pub const ON_INVESTIGATION_CHANGED: &str = "sentinel::on-investigation-changed";
pub const ON_INVESTIGATION_CHANGED_DESC: &str =
    "Fires when an investigation is created, starts, or finishes. Observe it with args \
     { group_id? } (the sentinel::investigation-changed binding config); every observer \
     receives the InvestigationChangedEventV1. Only the sentinel worker fires it: invoking it \
     directly is refused.";

/// How long occurrence events for one group are gathered before delivery.
const COALESCE: Duration = Duration::from_secs(1);

type Bindings = Arc<RwLock<HashMap<String, TriggerConfig>>>;
type Observers = Arc<RwLock<HashMap<String, Arc<dyn EventObserver>>>>;

/// One observer of a `sentinel::on-*` function as a sink of the fan-out:
/// the SDK's [`Observer`] in production, a recorder in tests (no engine).
#[async_trait]
pub trait EventObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    /// The observer's args: the trigger type's binding config.
    fn args(&self) -> &Value;
    /// Hands one event to the observer's target (`obs.call`).
    async fn deliver(&self, payload: Value) -> Result<(), Error>;
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn args(&self) -> &Value {
        self.args_value()
    }

    async fn deliver(&self, payload: Value) -> Result<(), Error> {
        self.call::<Value>(payload).await.map(|_| ())
    }
}

/// The subscribers of one trigger type: its bindings, and the observers of
/// the observable function next to it. After a worker restart the engine
/// replays both, so the table rebuilds itself.
#[derive(Clone, Default)]
pub struct Table {
    bindings: Bindings,
    observers: Observers,
}

impl Table {
    /// Files an observer next to the bindings; a replayed id replaces it.
    pub fn add_observer(&self, observer: Arc<dyn EventObserver>) {
        self.observers
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(observer.id().to_string(), observer);
    }

    /// Takes an observer out; whether it was filed.
    pub fn remove_observer(&self, id: &str) -> bool {
        self.observers
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(id)
            .is_some()
    }

    /// The bindings that asked for `event` (every binding when `None`).
    fn binding_targets(&self, event: Option<&GroupChangedEventV1>) -> Vec<TriggerConfig> {
        self.bindings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|binding| event.is_none_or(|event| matches(&binding.config, event)))
            .cloned()
            .collect()
    }

    /// The observers that asked for `event`, under the binding filter.
    fn observer_targets(&self, event: Option<&GroupChangedEventV1>) -> Vec<Arc<dyn EventObserver>> {
        self.observers
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .values()
            .filter(|observer| event.is_none_or(|event| matches(observer.args(), event)))
            .cloned()
            .collect()
    }

    fn len(&self) -> usize {
        self.bindings
            .read()
            .unwrap_or_else(|p| p.into_inner())
            .len()
            + self
                .observers
                .read()
                .unwrap_or_else(|p| p.into_inner())
                .len()
    }
}

#[derive(Clone, Default)]
pub struct Subscribers {
    groups: Table,
    investigations: Table,
}

impl Subscribers {
    /// Bindings plus observers, for groups and for investigations.
    pub fn counts(&self) -> (usize, usize) {
        (self.groups.len(), self.investigations.len())
    }
}

/// Remember a binding, forget it on unregister. A config that does not parse
/// is kept and simply never matches, rather than refusing the registration.
struct BindingTable(Table);

#[async_trait]
impl TriggerHandler for BindingTable {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .bindings
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .bindings
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

/// Files each observer of a `sentinel::on-*` function in the table its
/// trigger type fans out from, and takes it out when the engine removes it.
/// The engine checks the args against `args_format`; args that still do not
/// match an event simply never receive it, as with a binding.
struct ObserverTable {
    function_id: &'static str,
    table: Table,
}

#[async_trait]
impl ObserveHandler for ObserverTable {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.table.add_observer(Arc::new(obs.clone()));
        tracing::info!(
            function_id = self.function_id,
            observer = %obs.id(),
            target = %obs.target(),
            "observer added"
        );
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.table.remove_observer(obs.id());
        tracing::info!(
            function_id = self.function_id,
            observer = %obs.id(),
            "observer removed"
        );
    }
}

/// The error every direct invocation of a `sentinel::on-*` function gets:
/// events only come from this worker's own fan-out.
pub fn forbidden(function_id: &str) -> Error {
    Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args: {{ group_id? }} }}, target)); \
             only the sentinel worker fires its events"
        ),
        stacktrace: None,
    }
}

/// The body of a `sentinel::on-*` function: refuses every event handed to
/// it with [`forbidden`]. Its parameter is the event its observers receive,
/// so the published request schema documents the delivered shape (an input
/// that does not even parse as that event is refused by the SDK with a
/// deserialization error before reaching here).
pub fn refuse_invocation<Event>(function_id: &str, _event: Event) -> Result<(), Error> {
    Err(forbidden(function_id))
}

/// The handler registered for a `sentinel::on-*` function: typed on the
/// delivered `Event` (request schema) and `()` (response schema).
pub fn observable_handler<Event>(
    function_id: &'static str,
) -> impl Fn(Event) -> Result<(), Error> + Send + Sync + 'static {
    move |event: Event| refuse_invocation(function_id, event)
}

/// One observable function's wire surface.
pub struct ObservableSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    /// The observers' args (`args_format`): the trigger type's binding config.
    pub args_schema: Value,
    /// The event each observer receives, registered as the request format.
    pub event_schema: Value,
}

/// JSON Schema of `T`, generated as iii-sdk does at registration.
fn schema_value<T: JsonSchema>() -> Value {
    let schema = schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>();
    serde_json::to_value(schema).unwrap_or(Value::Null)
}

/// The observable functions, in registration order: group, then
/// investigation, each next to the trigger type of the same event.
pub fn observable_catalog() -> Vec<ObservableSpec> {
    vec![
        ObservableSpec {
            function_id: ON_GROUP_CHANGED,
            description: ON_GROUP_CHANGED_DESC,
            args_schema: schema_value::<GroupChangedConfigV1>(),
            event_schema: schema_value::<GroupChangedEventV1>(),
        },
        ObservableSpec {
            function_id: ON_INVESTIGATION_CHANGED,
            description: ON_INVESTIGATION_CHANGED_DESC,
            args_schema: schema_value::<GroupChangedConfigV1>(),
            event_schema: schema_value::<InvestigationChangedEventV1>(),
        },
    ]
}

/// Registers one owner-delivered `sentinel::on-*` function whose observers
/// share `table` with the trigger type next to it. Its request schema
/// (derived from the handler's `Event` parameter, equal to
/// `spec.event_schema`) documents the event its observers receive. On an
/// engine without observe it registers as a plain function and its hooks
/// never run.
fn register_observable<Event>(iii: &IIIClient, spec: ObservableSpec, table: Table)
where
    Event: DeserializeOwned + JsonSchema + Send + 'static,
{
    let function_id = spec.function_id;
    iii.register_function(
        function_id,
        RegisterFunction::new(observable_handler::<Event>(function_id))
            .description(spec.description)
            .observe(
                Observe::owner()
                    .args_format_value(spec.args_schema)
                    .handler(ObserverTable { function_id, table }),
            ),
    );
}

pub fn register_trigger_types(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    // Kept until the trigger types are retired: consumers bind them on 0.24
    // and 0.25 engines.
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            GROUP_CHANGED,
            "Fires when an error group is created, takes a new occurrence, or changes state. \
             Config: { group_id?, service_name?, ops? } — ops narrows to created, occurrence or \
             status. Occurrence events are coalesced per group over one second; re-read the group \
             rather than trusting the payload.",
            BindingTable(subscribers.groups.clone()),
        )
        .trigger_request_format::<GroupChangedConfigV1>()
        .call_request_format::<GroupChangedEventV1>(),
    );
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            INVESTIGATION_CHANGED,
            "Fires when an investigation is created, starts, or finishes. \
             Config: { group_id? }.",
            BindingTable(subscribers.investigations.clone()),
        )
        .trigger_request_format::<GroupChangedConfigV1>()
        .call_request_format::<Value>(),
    );

    let mut catalog = observable_catalog().into_iter();
    if let (Some(group), Some(investigation)) = (catalog.next(), catalog.next()) {
        register_observable::<GroupChangedEventV1>(iii, group, subscribers.groups.clone());
        register_observable::<InvestigationChangedEventV1>(
            iii,
            investigation,
            subscribers.investigations.clone(),
        );
    }
}

/// Delivers events to the bindings and observers that asked for them.
pub struct Emitter {
    iii: Arc<IIIClient>,
    subscribers: Subscribers,
    /// Occurrence events waiting out their coalescing window, newest payload
    /// per group.
    pending: Arc<Mutex<HashMap<String, GroupChangedEventV1>>>,
}

impl Emitter {
    pub fn new(iii: Arc<IIIClient>, subscribers: Subscribers) -> Arc<Self> {
        let emitter = Arc::new(Self {
            iii,
            subscribers,
            pending: Arc::new(Mutex::new(HashMap::new())),
        });
        let flusher = emitter.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(COALESCE);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                ticker.tick().await;
                flusher.flush().await;
            }
        });
        emitter
    }

    /// Send a group event, coalescing the noisy kind.
    pub async fn group_changed(&self, event: GroupChangedEventV1) {
        if event.op == GroupChangedOpV1::Occurrence {
            self.pending
                .lock()
                .await
                .insert(event.group_id.clone(), event);
            return;
        }
        // A creation or a state change supersedes anything gathered for that
        // group: the later event carries the newer count anyway.
        self.pending.lock().await.remove(&event.group_id);
        self.deliver(&self.subscribers.groups, &event).await;
    }

    pub async fn investigation_changed(&self, payload: Value) {
        self.deliver_value(&self.subscribers.investigations, payload, None)
            .await;
    }

    async fn flush(&self) {
        let ready: Vec<GroupChangedEventV1> = {
            let mut pending = self.pending.lock().await;
            pending.drain().map(|(_, event)| event).collect()
        };
        for event in ready {
            self.deliver(&self.subscribers.groups, &event).await;
        }
    }

    async fn deliver(&self, table: &Table, event: &GroupChangedEventV1) {
        let Ok(payload) = serde_json::to_value(event) else {
            return;
        };
        self.deliver_value(table, payload, Some(event)).await;
    }

    async fn deliver_value(
        &self,
        table: &Table,
        payload: Value,
        event: Option<&GroupChangedEventV1>,
    ) {
        for binding in table.binding_targets(event) {
            let mut request: TriggerRequestWithMetadata = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: payload.clone(),
                // Fire and forget: a subscriber that is slow or broken must
                // not hold up the pipeline that noticed the error.
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            }
            .into();
            if let Some(metadata) = subscription_metadata(&binding) {
                request = request.metadata(metadata);
            }
            if let Some(namespace) = &binding.namespace {
                request = request.namespace(namespace.clone());
            }
            if let Err(error) = self.iii.trigger(request).await {
                tracing::warn!(
                    function_id = binding.function_id,
                    %error,
                    "sentinel event delivery failed"
                );
            }
        }

        // Observers of the observable function next to the trigger type get
        // the same payload under the same filter (their args are the binding
        // config), through `obs.call`. Each call runs on its own task, for the
        // same reason a binding is fired and forgotten.
        spawn_observer_calls(table.observer_targets(event), payload);
    }
}

/// Hands `payload` to each observer on its own task, in the current
/// invocation's scope. An observer whose own filters decline the event is
/// not an error.
fn spawn_observer_calls(
    observers: Vec<Arc<dyn EventObserver>>,
    payload: Value,
) -> Vec<tokio::task::JoinHandle<()>> {
    let ctx = iii_sdk::current_invocation();
    observers
        .into_iter()
        .map(|observer| {
            let payload = payload.clone();
            tokio::spawn(iii_sdk::scope(ctx.clone(), async move {
                match observer.deliver(payload).await {
                    Ok(()) => {}
                    Err(Error::Remote { code, .. })
                        if code == observe_codes::FILTERS_NOT_MATCHED => {}
                    Err(error) => tracing::warn!(
                        observer = %observer.id(),
                        %error,
                        "sentinel observer delivery failed"
                    ),
                }
            }))
        })
        .collect()
}

/// The console puts its per-tab metadata on the binding config; a worker puts
/// it on the registration. Either is passed back on delivery.
fn subscription_metadata(binding: &TriggerConfig) -> Option<Value> {
    binding
        .config
        .get("metadata")
        .cloned()
        .filter(|value| !value.is_null())
        .or_else(|| binding.metadata.clone())
}

/// Whether one binding asked for this event.
fn matches(config: &Value, event: &GroupChangedEventV1) -> bool {
    if let Some(group_id) = config.get("group_id").and_then(Value::as_str) {
        if !group_id.is_empty() && group_id != event.group_id {
            return false;
        }
    }
    if let Some(ops) = config.get("ops").and_then(Value::as_array) {
        let wanted = op_name(event.op);
        if !ops.is_empty() && !ops.iter().any(|op| op.as_str() == Some(wanted)) {
            return false;
        }
    }
    true
}

fn op_name(op: GroupChangedOpV1) -> &'static str {
    match op {
        GroupChangedOpV1::Created => "created",
        GroupChangedOpV1::Occurrence => "occurrence",
        GroupChangedOpV1::Status => "status",
    }
}

/// Build the event for a group the ingest just touched.
pub fn group_event(
    op: GroupChangedOpV1,
    group_id: &str,
    status: GroupStatusV1,
    occurrence_count: u64,
    reason: Option<crate::GroupChangeReasonV1>,
) -> GroupChangedEventV1 {
    GroupChangedEventV1 {
        op,
        group_id: group_id.to_string(),
        status,
        previous_status: None,
        occurrence_count,
        reason,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn event(group_id: &str, op: GroupChangedOpV1) -> GroupChangedEventV1 {
        GroupChangedEventV1 {
            op,
            group_id: group_id.into(),
            status: GroupStatusV1::New,
            previous_status: None,
            occurrence_count: 1,
            reason: None,
        }
    }

    #[test]
    fn an_empty_config_subscribes_to_everything() {
        assert!(matches(
            &json!({}),
            &event("grp_1", GroupChangedOpV1::Status)
        ));
    }

    #[test]
    fn a_binding_can_follow_one_group() {
        let config = json!({ "group_id": "grp_1" });
        assert!(matches(&config, &event("grp_1", GroupChangedOpV1::Status)));
        assert!(!matches(&config, &event("grp_2", GroupChangedOpV1::Status)));
    }

    #[test]
    fn a_binding_can_ask_for_state_changes_only() {
        let config = json!({ "ops": ["status", "created"] });
        assert!(matches(&config, &event("grp_1", GroupChangedOpV1::Status)));
        assert!(matches(&config, &event("grp_1", GroupChangedOpV1::Created)));
        assert!(
            !matches(&config, &event("grp_1", GroupChangedOpV1::Occurrence)),
            "an alert does not want a delivery per occurrence"
        );
    }

    #[test]
    fn the_consoles_per_tab_metadata_wins_over_the_registration() {
        let binding = TriggerConfig {
            id: "b1".into(),
            function_id: "iii::console::group-changed".into(),
            config: json!({ "metadata": { "tab": "a" } }),
            metadata: Some(json!({ "tab": "b" })),
            namespace: None,
        };
        assert_eq!(subscription_metadata(&binding), Some(json!({ "tab": "a" })));

        let worker = TriggerConfig {
            id: "b2".into(),
            function_id: "alerts::on-error".into(),
            config: json!({}),
            metadata: Some(json!({ "channel": "#ops" })),
            namespace: None,
        };
        assert_eq!(
            subscription_metadata(&worker),
            Some(json!({ "channel": "#ops" }))
        );
    }

    /// Stands in for the SDK observer: records what `obs.call` would carry
    /// to its target.
    struct Recorder {
        id: String,
        args: Value,
        seen: std::sync::Mutex<Vec<Value>>,
    }

    impl Recorder {
        fn new(id: &str, args: Value) -> Arc<Self> {
            Arc::new(Self {
                id: id.into(),
                args,
                seen: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn seen(&self) -> Vec<Value> {
            self.seen.lock().unwrap().clone()
        }
    }

    #[async_trait]
    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        fn args(&self) -> &Value {
            &self.args
        }

        async fn deliver(&self, payload: Value) -> Result<(), Error> {
            self.seen.lock().unwrap().push(payload);
            Ok(())
        }
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: format!("alerts::{id}"),
            config,
            metadata: None,
            namespace: None,
        }
    }

    async fn deliver_to_observers(
        table: &Table,
        event: Option<&GroupChangedEventV1>,
        payload: Value,
    ) {
        for handle in spawn_observer_calls(table.observer_targets(event), payload) {
            handle.await.unwrap();
        }
    }

    fn assert_forbidden(function_id: &str, result: Result<(), Error>) {
        match result {
            Err(Error::Remote { code, message, .. }) => {
                assert_eq!(code, observe_codes::FORBIDDEN);
                assert_eq!(code, "FORBIDDEN");
                assert!(message.contains(function_id), "{message}");
            }
            other => panic!("expected FORBIDDEN, got {other:?}"),
        }
    }

    #[test]
    fn each_on_function_refuses_direct_invocation() {
        let group = observable_handler::<GroupChangedEventV1>(ON_GROUP_CHANGED);
        for op in [
            GroupChangedOpV1::Created,
            GroupChangedOpV1::Occurrence,
            GroupChangedOpV1::Status,
        ] {
            assert_forbidden(ON_GROUP_CHANGED, group(event("grp_1", op)));
        }
        let investigation =
            observable_handler::<InvestigationChangedEventV1>(ON_INVESTIGATION_CHANGED);
        let changed = InvestigationChangedEventV1 {
            op: crate::InvestigationChangedOpV1::Created,
            investigation_id: "inv_1".into(),
            group_id: "grp_1".into(),
            session_id: "s_1".into(),
            status: crate::InvestigationStatusV1::Running,
            error: None,
        };
        assert_forbidden(ON_INVESTIGATION_CHANGED, investigation(changed));
    }

    #[test]
    fn observable_functions_sit_next_to_their_trigger_types() {
        let catalog = observable_catalog();
        let ids: Vec<_> = catalog.iter().map(|spec| spec.function_id).collect();
        assert_eq!(
            ids,
            [
                "sentinel::on-group-changed",
                "sentinel::on-investigation-changed"
            ]
        );
        assert_eq!(GROUP_CHANGED, "sentinel::group-changed");
        assert_eq!(INVESTIGATION_CHANGED, "sentinel::investigation-changed");
        for spec in &catalog {
            // The observers' args are the binding config of the trigger type.
            assert_eq!(spec.args_schema, schema_value::<GroupChangedConfigV1>());
            assert!(spec.description.contains("refused"), "{}", spec.function_id);
        }
        assert_eq!(
            catalog[0].event_schema,
            schema_value::<GroupChangedEventV1>()
        );
        assert_eq!(
            catalog[1].event_schema,
            schema_value::<InvestigationChangedEventV1>()
        );
    }

    #[tokio::test]
    async fn observer_receives_the_change_a_binding_receives() {
        let table = Table::default();
        let bindings = BindingTable(table.clone());
        bindings
            .register_trigger(binding("b1", json!({ "group_id": "grp_1" })))
            .await
            .unwrap();
        bindings
            .register_trigger(binding("b2", json!({ "group_id": "grp_2" })))
            .await
            .unwrap();
        let matching = Recorder::new("o1", json!({ "group_id": "grp_1" }));
        let other = Recorder::new("o2", json!({ "group_id": "grp_2" }));
        table.add_observer(matching.clone());
        table.add_observer(other.clone());

        let changed = event("grp_1", GroupChangedOpV1::Status);
        let bound: Vec<String> = table
            .binding_targets(Some(&changed))
            .into_iter()
            .map(|binding| binding.id)
            .collect();
        assert_eq!(bound, ["b1"]);

        let payload = serde_json::to_value(&changed).unwrap();
        deliver_to_observers(&table, Some(&changed), payload.clone()).await;
        assert_eq!(matching.seen(), vec![payload]);
        assert!(other.seen().is_empty());
    }

    #[tokio::test]
    async fn observers_filter_ops_like_a_binding() {
        let table = Table::default();
        BindingTable(table.clone())
            .register_trigger(binding("b1", json!({ "ops": ["status"] })))
            .await
            .unwrap();
        let alert = Recorder::new("o1", json!({ "ops": ["status"] }));
        table.add_observer(alert.clone());

        let noisy = event("grp_1", GroupChangedOpV1::Occurrence);
        assert!(table.binding_targets(Some(&noisy)).is_empty());
        assert!(table.observer_targets(Some(&noisy)).is_empty());

        let status = event("grp_1", GroupChangedOpV1::Status);
        assert_eq!(table.binding_targets(Some(&status)).len(), 1);
        let payload = serde_json::to_value(&status).unwrap();
        deliver_to_observers(&table, Some(&status), payload.clone()).await;
        assert_eq!(alert.seen(), vec![payload]);
    }

    #[tokio::test]
    async fn investigation_events_reach_every_observer_and_binding() {
        let subscribers = Subscribers::default();
        let table = subscribers.investigations.clone();
        BindingTable(table.clone())
            .register_trigger(binding("b1", json!({ "group_id": "grp_9" })))
            .await
            .unwrap();
        let observer = Recorder::new("o1", json!({ "group_id": "grp_9" }));
        table.add_observer(observer.clone());
        assert_eq!(subscribers.counts(), (0, 2));

        let payload = json!({ "investigation_id": "inv_1", "group_id": "grp_1" });
        assert_eq!(table.binding_targets(None).len(), 1);
        deliver_to_observers(&table, None, payload.clone()).await;
        assert_eq!(observer.seen(), vec![payload]);
    }

    #[tokio::test]
    async fn a_removed_observer_stops_receiving_and_bindings_stay() {
        let table = Table::default();
        BindingTable(table.clone())
            .register_trigger(binding("same", json!({})))
            .await
            .unwrap();
        let observer = Recorder::new("same", json!({}));
        table.add_observer(observer.clone());
        // A replayed observer id replaces the entry instead of doubling it.
        table.add_observer(observer.clone());
        assert_eq!(table.len(), 2);

        assert!(table.remove_observer("same"));
        assert!(!table.remove_observer("same"));
        assert_eq!(table.len(), 1, "the binding of the same id stays");

        let changed = event("grp_1", GroupChangedOpV1::Created);
        assert_eq!(table.binding_targets(Some(&changed)).len(), 1);
        deliver_to_observers(&table, Some(&changed), json!({})).await;
        assert!(observer.seen().is_empty());
    }
}
