//! The two trigger types this worker provides, and the observable function
//! next to each (iii 0.25).
//!
//! - `stories:changed` fires after every working-tree rebuild that changed
//!   at least one component, with the classified changes against the
//!   previous build. Config `{ workspace?, project?, kinds? }`.
//!   `stories::on-changed` is observed with the same config as args.
//! - `stories:build` fires on every build state transition (queued, running,
//!   done, failed) for any line. Config `{ workspace? }`.
//!   `stories::on-build` is observed with the same config as args.
//!
//! Bindings and observers live in in-process maps keyed by trigger or
//! observer id; delivery is fire-and-forget: bindings in their namespace with
//! their metadata attached, observers with `obs.call`. Both get the same
//! payload for the same config. The `on-*` functions refuse direct
//! invocation with `FORBIDDEN`: events only come from this worker.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata, observe_codes};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{Change, ChangeKind, ChangeSummary, LineInfo};

pub const CHANGED_TRIGGER: &str = "stories:changed";
pub const BUILD_TRIGGER: &str = "stories:build";

/// The observable function next to [`CHANGED_TRIGGER`].
pub const ON_CHANGED_FN: &str = "stories::on-changed";
/// The observable function next to [`BUILD_TRIGGER`].
pub const ON_BUILD_FN: &str = "stories::on-build";

pub const ON_CHANGED_DESCRIPTION: &str = "Fires after a working-tree rebuild changed components. Observe it with args { workspace?, project?, kinds? } (the stories:changed binding config); every matching observer receives the ChangedEvent { workspace, line, base, summary, changes, at } narrowed to its args, as a binding does. Only the stories worker fires it: invoking it directly is refused.";
pub const ON_BUILD_DESCRIPTION: &str = "Fires on every build state transition (queued, running, done, failed) of any line. Observe it with args { workspace? } (the stories:build binding config); every matching observer receives the BuildEvent { workspace, build_id, status, line, message?, at }. Only the stories worker fires it: invoking it directly is refused.";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangedConfig {
    /// Workspace slug. Omit for every workspace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    /// Project name. Omit for every project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<String>,
    /// Change kinds to fire for: direct, indirect, new, removed. Omit for all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kinds: Option<Vec<String>>,
    /// Metadata attached to every invocation this binding receives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct BuildConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangedEvent {
    pub workspace: String,
    pub line: LineInfo,
    /// The line the changes were classified against.
    pub base: LineInfo,
    pub summary: ChangeSummary,
    /// Only the changed components (never `unchanged`).
    pub changes: Vec<Change>,
    pub at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuildEvent {
    pub workspace: String,
    pub build_id: String,
    /// queued | running | done | failed
    pub status: String,
    pub line: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    pub at: String,
}

pub type Bindings = Arc<RwLock<HashMap<String, TriggerConfig>>>;

/// One observer of an observable function as a sink: the SDK's [`Observer`]
/// in production, a recorder in tests (no engine needed).
#[async_trait]
pub trait EventObserver: Send + Sync + 'static {
    fn id(&self) -> &str;
    /// The observer's args: the trigger config a binding would pass.
    fn args_value(&self) -> &Value;
    /// Hands one event to the observer's target (`obs.call`).
    async fn deliver(&self, payload: &Value) -> Result<(), Error>;
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn args_value(&self) -> &Value {
        Observer::args_value(self)
    }

    async fn deliver(&self, payload: &Value) -> Result<(), Error> {
        self.call::<Value>(payload).await.map(|_| ())
    }
}

pub type Observers = Arc<RwLock<HashMap<String, Arc<dyn EventObserver>>>>;

/// Subscribers of each event: trigger bindings and observers live side by
/// side, and one fan-out serves both.
#[derive(Clone, Default)]
pub struct Subscribers {
    pub changed: Bindings,
    pub build: Bindings,
    pub changed_observers: Observers,
    pub build_observers: Observers,
}

struct BindingTable(Bindings);

#[async_trait]
impl TriggerHandler for BindingTable {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .insert(config.id.clone(), config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.0
            .write()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

/// Files an observer under its id.
pub fn add_observer(observers: &Observers, observer: Arc<dyn EventObserver>) {
    observers
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .insert(observer.id().to_owned(), observer);
}

pub fn remove_observer(observers: &Observers, id: &str) {
    observers
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .remove(id);
}

/// Files each observer of an observable function next to the bindings of
/// its trigger type, and takes it out when the engine removes it.
struct ObserverTable {
    function_id: &'static str,
    observers: Observers,
}

#[async_trait]
impl ObserveHandler for ObserverTable {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Args are checked by the engine against args_format (the trigger config).
        add_observer(&self.observers, Arc::new(obs.clone()));
        tracing::info!(observer = %obs.id(), target = %obs.target(), "{} observer added", self.function_id);
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        remove_observer(&self.observers, obs.id());
        tracing::info!(observer = %obs.id(), "{} observer removed", self.function_id);
    }
}

fn forbidden(function_id: &str) -> Error {
    Error::Remote {
        code: observe_codes::FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)); \
             only the stories worker fires its events"
        ),
        stacktrace: None,
    }
}

/// The handler of [`ON_CHANGED_FN`]: events only come from this worker's own
/// fan-out, so every invocation is refused, whatever its input. Typed on the
/// delivered [`ChangedEvent`] so the function publishes the event's schema.
pub fn refuse_changed(_event: ChangedEvent) -> Result<(), Error> {
    Err(forbidden(ON_CHANGED_FN))
}

/// The handler of [`ON_BUILD_FN`]: every invocation is refused. Typed on the
/// delivered [`BuildEvent`] so the function publishes the event's schema.
pub fn refuse_build(_event: BuildEvent) -> Result<(), Error> {
    Err(forbidden(ON_BUILD_FN))
}

/// Registers both trigger types and, next to each, its owner-delivered
/// observable function sharing [`Subscribers`]. On an engine without observe
/// the functions are plain functions whose hooks never run.
pub fn register_trigger_types(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    // Kept for consumers that bind the trigger types, on 0.24 and 0.25 engines.
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGED_TRIGGER,
            "Fires after a working-tree rebuild changed components, with each change classified as direct, indirect, new or removed. Config: { workspace?, project?, kinds?, metadata? }.",
            BindingTable(subscribers.changed.clone()),
        )
        .trigger_request_format::<ChangedConfig>()
        .call_request_format::<ChangedEvent>(),
    );
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            BUILD_TRIGGER,
            "Fires on every build state transition (queued, running, done, failed) of any line. Config: { workspace?, metadata? }.",
            BindingTable(subscribers.build.clone()),
        )
        .trigger_request_format::<BuildConfig>()
        .call_request_format::<BuildEvent>(),
    );
    iii.register_function(
        ON_CHANGED_FN,
        RegisterFunction::new(refuse_changed)
            .description(ON_CHANGED_DESCRIPTION)
            .observe(
                Observe::owner()
                    .args_format::<ChangedConfig>()
                    .handler(ObserverTable {
                        function_id: ON_CHANGED_FN,
                        observers: subscribers.changed_observers.clone(),
                    }),
            ),
    );
    iii.register_function(
        ON_BUILD_FN,
        RegisterFunction::new(refuse_build)
            .description(ON_BUILD_DESCRIPTION)
            .observe(
                Observe::owner()
                    .args_format::<BuildConfig>()
                    .handler(ObserverTable {
                        function_id: ON_BUILD_FN,
                        observers: subscribers.build_observers.clone(),
                    }),
            ),
    );
}

fn text<'a>(config: &'a Value, key: &str) -> &'a str {
    config
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .trim()
}

fn kind_name(kind: ChangeKind) -> &'static str {
    match kind {
        ChangeKind::Direct => "direct",
        ChangeKind::Indirect => "indirect",
        ChangeKind::New => "new",
        ChangeKind::Removed => "removed",
        ChangeKind::Unchanged => "unchanged",
    }
}

/// The event narrowed to what the binding asked for; `None` when nothing is
/// left to deliver.
pub fn narrow_changed(config: &Value, event: &ChangedEvent) -> Option<ChangedEvent> {
    let workspace = text(config, "workspace");
    if !workspace.is_empty() && workspace != event.workspace {
        return None;
    }
    let project = text(config, "project");
    let kinds: Vec<&str> = config
        .get("kinds")
        .and_then(Value::as_array)
        .map(|k| {
            k.iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|k| !k.is_empty())
                .collect()
        })
        .unwrap_or_default();
    let changes: Vec<Change> = event
        .changes
        .iter()
        .filter(|c| project.is_empty() || c.project == project)
        .filter(|c| kinds.is_empty() || kinds.contains(&kind_name(c.kind)))
        .cloned()
        .collect();
    if changes.is_empty() {
        return None;
    }
    Some(ChangedEvent {
        summary: crate::model::summarize(&changes),
        changes,
        ..event.clone()
    })
}

pub fn matches_build(config: &Value, event: &BuildEvent) -> bool {
    let workspace = text(config, "workspace");
    workspace.is_empty() || workspace == event.workspace
}

fn subscription_metadata(binding: &TriggerConfig) -> Option<Value> {
    binding
        .config
        .get("metadata")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| binding.metadata.clone())
}

/// One observer's delivery: the observer and the payload a binding with the
/// same config would receive.
pub type ObserverDelivery = (Arc<dyn EventObserver>, Value);

/// Everything one event reaches: bindings of the trigger type and observers
/// of the observable function, each with its payload.
pub type Deliveries = (Vec<(TriggerConfig, Value)>, Vec<ObserverDelivery>);

fn deliver(iii: Arc<IIIClient>, (bindings, observers): Deliveries) {
    if bindings.is_empty() && observers.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for (binding, payload) in bindings {
            let mut request: TriggerRequestWithMetadata = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload,
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
            if let Err(error) = iii.trigger(request).await {
                tracing::warn!(function_id = binding.function_id, error = %error, "stories trigger delivery failed");
            }
        }
        for (observer, payload) in observers {
            match observer.deliver(&payload).await {
                Ok(()) => {}
                Err(Error::Remote { code, .. }) if code == observe_codes::FILTERS_NOT_MATCHED => {}
                Err(error) => {
                    tracing::warn!(observer = %observer.id(), error = %error, "stories observer delivery failed");
                }
            }
        }
    });
}

/// The bindings and observers a `stories:changed` event reaches, each with
/// the event narrowed to its config (a binding's config, an observer's args).
pub fn changed_deliveries(subscribers: &Subscribers, event: &ChangedEvent) -> Deliveries {
    let bindings = subscribers
        .changed
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter_map(|binding| {
            let narrowed = narrow_changed(&binding.config, event)?;
            serde_json::to_value(narrowed)
                .ok()
                .map(|payload| (binding.clone(), payload))
        })
        .collect();
    let observers = subscribers
        .changed_observers
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter_map(|observer| {
            let narrowed = narrow_changed(observer.args_value(), event)?;
            serde_json::to_value(narrowed)
                .ok()
                .map(|payload| (observer.clone(), payload))
        })
        .collect();
    (bindings, observers)
}

/// The bindings and observers a `stories:build` event reaches.
pub fn build_deliveries(subscribers: &Subscribers, event: &BuildEvent) -> Deliveries {
    let Ok(payload) = serde_json::to_value(event) else {
        return Deliveries::default();
    };
    let bindings = subscribers
        .build
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|binding| matches_build(&binding.config, event))
        .map(|binding| (binding.clone(), payload.clone()))
        .collect();
    let observers = subscribers
        .build_observers
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|observer| matches_build(observer.args_value(), event))
        .map(|observer| (observer.clone(), payload.clone()))
        .collect();
    (bindings, observers)
}

pub fn emit_changed(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &ChangedEvent) {
    deliver(iii.clone(), changed_deliveries(subscribers, event));
}

pub fn emit_build(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &BuildEvent) {
    deliver(iii.clone(), build_deliveries(subscribers, event));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{LineKind, summarize};
    use serde_json::json;

    fn change(project: &str, kind: ChangeKind) -> Change {
        Change {
            id: "x".into(),
            project: project.into(),
            title: "X".into(),
            file: "x".into(),
            kind,
            files: vec![],
            a_version: None,
            b_version: None,
            states: vec![],
        }
    }

    fn event() -> ChangedEvent {
        let line = LineInfo {
            key: "worktree".into(),
            kind: LineKind::Worktree,
            label: "working tree".into(),
            sha: None,
            dirty: None,
            built_at: "t".into(),
        };
        let changes = vec![
            change("app", ChangeKind::Direct),
            change("admin", ChangeKind::Indirect),
        ];
        ChangedEvent {
            workspace: "ws".into(),
            line: line.clone(),
            base: line,
            summary: summarize(&changes),
            changes,
            at: "t".into(),
        }
    }

    #[test]
    fn narrowing_filters_workspace_project_and_kinds() {
        assert!(narrow_changed(&json!({ "workspace": "other" }), &event()).is_none());
        let narrowed = narrow_changed(&json!({ "project": "admin" }), &event()).unwrap();
        assert_eq!(narrowed.changes.len(), 1);
        assert_eq!(narrowed.summary.indirect, 1);
        assert!(narrow_changed(&json!({ "kinds": ["new"] }), &event()).is_none());
        assert_eq!(
            narrow_changed(&json!({}), &event()).unwrap().changes.len(),
            2
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

        fn args_value(&self) -> &Value {
            &self.args
        }

        async fn deliver(&self, payload: &Value) -> Result<(), Error> {
            self.seen.lock().unwrap().push(payload.clone());
            Ok(())
        }
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: format!("ui::{id}"),
            config,
            metadata: None,
            namespace: None,
        }
    }

    async fn bind(bindings: &Bindings, id: &str, config: Value) {
        BindingTable(bindings.clone())
            .register_trigger(binding(id, config))
            .await
            .unwrap();
    }

    async fn hand_to_observers(deliveries: Vec<ObserverDelivery>) {
        for (observer, payload) in deliveries {
            observer.deliver(&payload).await.unwrap();
        }
    }

    fn build_event(workspace: &str) -> BuildEvent {
        BuildEvent {
            workspace: workspace.into(),
            build_id: "b_1".into(),
            status: "done".into(),
            line: "worktree".into(),
            message: None,
            at: "t".into(),
        }
    }

    /// The structs the on-* functions are typed on are exactly what the
    /// fan-out delivers to observers.
    #[test]
    fn delivered_payloads_are_the_typed_events() {
        let subscribers = Subscribers::default();
        add_observer(
            &subscribers.changed_observers,
            Recorder::new("o1", json!({})),
        );
        add_observer(&subscribers.build_observers, Recorder::new("o2", json!({})));
        let (_, changed) = changed_deliveries(&subscribers, &event());
        let (_, build) = build_deliveries(&subscribers, &build_event("ws"));
        for (_, payload) in changed {
            let typed: ChangedEvent = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), payload);
        }
        for (_, payload) in build {
            let typed: BuildEvent = serde_json::from_value(payload.clone()).unwrap();
            assert_eq!(serde_json::to_value(typed).unwrap(), payload);
        }
    }

    #[test]
    fn each_on_function_refuses_direct_invocation() {
        for (id, refusal) in [
            (ON_CHANGED_FN, refuse_changed(event())),
            (ON_BUILD_FN, refuse_build(build_event("ws"))),
        ] {
            match refusal {
                Err(Error::Remote { code, message, .. }) => {
                    assert_eq!(code, observe_codes::FORBIDDEN);
                    assert_eq!(code, "FORBIDDEN");
                    assert!(message.contains(id), "{message}");
                }
                other => panic!("{id}: expected FORBIDDEN, got {other:?}"),
            }
        }
    }

    #[tokio::test]
    async fn changed_observer_receives_the_event_a_binding_receives() {
        let subscribers = Subscribers::default();
        let args = json!({ "workspace": "ws", "project": "admin" });
        bind(&subscribers.changed, "b1", args.clone()).await;
        let matching = Recorder::new("o1", args);
        let other = Recorder::new("o2", json!({ "workspace": "other" }));
        add_observer(&subscribers.changed_observers, matching.clone());
        add_observer(&subscribers.changed_observers, other.clone());

        let (bindings, observers) = changed_deliveries(&subscribers, &event());
        assert_eq!(bindings.len(), 1);
        assert_eq!(observers.len(), 1, "only the observer whose args match");
        let bound = bindings[0].1.clone();
        // Narrowed to the config exactly as for the binding.
        assert_eq!(bound["changes"].as_array().unwrap().len(), 1);
        assert_eq!(bound["summary"]["indirect"], 1);

        hand_to_observers(observers).await;
        assert_eq!(matching.seen(), vec![bound]);
        assert!(other.seen().is_empty());
    }

    #[tokio::test]
    async fn build_observer_receives_the_event_a_binding_receives() {
        let subscribers = Subscribers::default();
        bind(&subscribers.build, "b1", json!({ "workspace": "ws" })).await;
        let matching = Recorder::new("o1", json!({ "workspace": "ws" }));
        let every = Recorder::new("o2", json!({}));
        let other = Recorder::new("o3", json!({ "workspace": "other" }));
        for observer in [&matching, &every, &other] {
            add_observer(&subscribers.build_observers, (*observer).clone());
        }

        let (bindings, observers) = build_deliveries(&subscribers, &build_event("ws"));
        assert_eq!(bindings.len(), 1);
        assert_eq!(observers.len(), 2);
        let bound = bindings[0].1.clone();
        assert_eq!(bound, serde_json::to_value(build_event("ws")).unwrap());

        hand_to_observers(observers).await;
        assert_eq!(matching.seen(), vec![bound.clone()]);
        assert_eq!(every.seen(), vec![bound]);
        assert!(other.seen().is_empty());
    }

    #[tokio::test]
    async fn observers_of_one_function_never_see_the_other_event() {
        let subscribers = Subscribers::default();
        let changed = Recorder::new("o1", json!({}));
        add_observer(&subscribers.changed_observers, changed.clone());
        let (bindings, observers) = build_deliveries(&subscribers, &build_event("ws"));
        assert!(bindings.is_empty() && observers.is_empty());

        // Removal stops deliveries; bindings are untouched by it.
        bind(&subscribers.changed, "o1", json!({})).await;
        remove_observer(&subscribers.changed_observers, "o1");
        let (bindings, observers) = changed_deliveries(&subscribers, &event());
        assert_eq!(bindings.len(), 1);
        assert!(observers.is_empty());
    }
}
