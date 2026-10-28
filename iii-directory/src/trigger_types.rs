//! Custom trigger types this worker publishes.
//!
//! Four trigger types exist:
//!
//! - `directory::skills::on-change`  — fires after every successful
//!   `directory::skills::download` that wrote at least one skill
//!   markdown file, or a `directory::skills::update`, `create`, or
//!   `delete`. External edits under the read-only `agents_skills_folder`
//!   also fire it via the fs watcher (doorbell only — the worker never
//!   writes there).
//! - `directory::system-prompts::on-change` — fires after every
//!   successful `directory::skills::download` that wrote at least one
//!   system prompt, a `directory::system-prompts::update`, or a
//!   `directory::system-prompts::create` / `delete`.
//! - `directory::agents::on-change` — fires after every successful
//!   download or direct create, update, delete, or external edit of an
//!   agent profile.
//! - `directory::kits::on-change` — fires when a kit plan is created or
//!   discarded, while an apply runs (step progress) and when it lands, and
//!   when an update check changes the set of available kit updates.
//!
//! The `mcp` worker (and any other interested subscriber) registers a
//! trigger instance of these types via
//! `iii.register_trigger(RegisterTriggerInput::new("directory::skills::on-change", ...))`.
//! The engine routes that registration through our
//! [`SkillsTriggerHandler`] which stashes the subscriber in
//! [`SubscriberSet`]. When a download lands, the `functions::download`
//! module reads the active subscribers and invokes each one via
//! `iii.trigger` — a simple in-process fanout.
//!
//! Using a named custom trigger keeps the coupling one-way: mcp knows
//! the directory worker publishes `directory::skills::on-change`; the
//! directory worker never has to know mcp exists.
//!
//! On an iii 0.25 engine each trigger type also has an owner-delivered
//! observable function next to it — `directory::on-skills-change`,
//! `directory::on-system-prompts-change`, `directory::on-agents-change` and
//! `directory::on-kits-change`. Their observers are filed in the trigger
//! type's own [`SubscriberSet`], next to its bindings, so [`dispatch`] hands
//! every observer (with [`Observer::call`]) exactly the event a binding
//! receives. The functions refuse direct invocation with `FORBIDDEN`: only
//! this worker delivers their events.

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
use serde::Deserialize;
use serde_json::Value;

pub const SKILLS_ON_CHANGE: &str = "directory::skills::on-change";
pub const SYSTEM_PROMPTS_ON_CHANGE: &str = "directory::system-prompts::on-change";
pub const AGENTS_ON_CHANGE: &str = "directory::agents::on-change";
pub const KITS_ON_CHANGE: &str = "directory::kits::on-change";

/// The observable functions next to the trigger types above (iii 0.25).
pub const ON_SKILLS_CHANGE_FN: &str = "directory::on-skills-change";
pub const ON_SYSTEM_PROMPTS_CHANGE_FN: &str = "directory::on-system-prompts-change";
pub const ON_AGENTS_CHANGE_FN: &str = "directory::on-agents-change";
pub const ON_KITS_CHANGE_FN: &str = "directory::on-kits-change";

/// The code a direct invocation of an observable function is refused with.
pub const FORBIDDEN: &str = "FORBIDDEN";

/// Args of an `directory::on-*-change` observer, the same (empty) config a
/// trigger binding takes: every observer receives every change.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct OnChangeSpec {}

/// One observer as a sink of a [`SubscriberSet`]. Abstracted so the fan-out
/// is unit-testable without an engine.
#[async_trait]
pub trait EventObserver: Send + Sync + 'static {
    /// The observer's id (stable across replays).
    fn id(&self) -> &str;

    /// Hands one event to the observer's target.
    async fn deliver(&self, event: Value) -> Result<(), String>;
}

#[async_trait]
impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    async fn deliver(&self, event: Value) -> Result<(), String> {
        self.call::<Value>(event)
            .await
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

/// The error an observable function's handler answers every invocation with.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args: {{}} }}, target)); \
             only the iii-directory worker delivers its events"
        ),
        stacktrace: None,
    }
}

/// One entry of a trigger type's registry: a binding of the trigger type or
/// an observer of its observable function.
enum Sink {
    Binding(TriggerConfig),
    Observer(Arc<dyn EventObserver>),
}

/// The registry key of an observer, apart from every trigger id.
fn observer_key(id: &str) -> String {
    format!("observer:{id}")
}

/// Thread-safe subscriber registry keyed by trigger-instance id (bindings)
/// or `observer:<id>` (observers of the matching `directory::on-*-change`
/// function). Cloned into both the `TriggerHandler` / `ObserveHandler`
/// (which mutate on register / unregister) and the fan-out path in the
/// function modules (which iterates read-only). Binding entries are
/// `TriggerConfig` so the fan-out can see the subscriber's `function_id`.
#[derive(Clone, Default)]
pub struct SubscriberSet {
    inner: Arc<Mutex<HashMap<String, Sink>>>,
}

impl SubscriberSet {
    pub fn new() -> Self {
        Self::default()
    }

    fn insert(&self, config: TriggerConfig) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.insert(config.id.clone(), Sink::Binding(config));
    }

    fn remove(&self, id: &str) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(id);
    }

    /// Files `observer` next to the bindings: every later [`dispatch`]
    /// reaches it.
    pub fn add_observer(&self, observer: Arc<dyn EventObserver>) {
        let key = observer_key(observer.id());
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.insert(key, Sink::Observer(observer));
    }

    pub fn remove_observer(&self, id: &str) {
        let mut map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.remove(&observer_key(id));
    }

    /// Snapshot of the current binding subscribers and their target
    /// namespaces. Returns a snapshot so the mutex isn't held across awaits.
    pub fn targets(&self) -> Vec<(String, Option<String>)> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.values()
            .filter_map(|sink| match sink {
                Sink::Binding(c) => Some((c.function_id.clone(), c.namespace.clone())),
                Sink::Observer(_) => None,
            })
            .collect()
    }

    /// Snapshot of the current observers.
    pub fn observers(&self) -> Vec<Arc<dyn EventObserver>> {
        let map = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        map.values()
            .filter_map(|sink| match sink {
                Sink::Observer(o) => Some(o.clone()),
                Sink::Binding(_) => None,
            })
            .collect()
    }

    /// The hooks of the observable function next to this set's trigger type:
    /// its observers join this set.
    pub fn observe_handler(&self, function_id: &'static str) -> OnChangeObserveHandler {
        OnChangeObserveHandler {
            function_id,
            subscribers: self.clone(),
        }
    }
}

/// Fire `payload` to every subscriber using `TriggerAction::Void`
/// (fire-and-forget so the mutation that produced the change isn't
/// blocked on downstream latency), then to every observer. Failures are
/// logged and swallowed because a slow / misbehaving subscriber must not
/// break the write path.
pub async fn dispatch(iii: &IIIClient, subscribers: &SubscriberSet, payload: Value) {
    let targets = subscribers.targets();
    for (function_id, namespace) in targets {
        let fid = function_id.clone();
        let payload_copy = payload.clone();
        let request = TriggerRequest {
            function_id: fid,
            payload: payload_copy,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        };
        let namespace = namespace.as_deref().unwrap_or("default");
        let res = iii.trigger(request.namespace(namespace)).await;
        if let Err(e) = res {
            tracing::warn!(
                function_id = %function_id,
                error = %e,
                "on-change fan-out failed"
            );
        }
    }
    deliver_to_observers(subscribers, payload);
}

/// Hand `payload` to every observer in `subscribers` with
/// [`EventObserver::deliver`] (`Observer::call`), each on its own task so a
/// slow observer never blocks the write path. Failures are logged.
pub fn deliver_to_observers(subscribers: &SubscriberSet, payload: Value) {
    for observer in subscribers.observers() {
        let event = payload.clone();
        tokio::spawn(async move {
            if let Err(error) = observer.deliver(event).await {
                tracing::warn!(
                    observer = %observer.id(),
                    error = %error,
                    "on-change observer delivery failed"
                );
            }
        });
    }
}

pub struct RegisteredTriggerTypes {
    pub skills: SubscriberSet,
    pub system_prompts: SubscriberSet,
    pub agents: SubscriberSet,
    pub kits: SubscriberSet,
}

pub fn register_all(iii: &Arc<IIIClient>) -> RegisteredTriggerTypes {
    let skills = register_on_change(
        iii,
        SKILLS_ON_CHANGE,
        ON_SKILLS_CHANGE_FN,
        "Fires after a directory::skills::download that wrote at least one skill markdown file, \
         or a directory::skills::update, create, or delete. Also fires with { op: \"external\" } \
         when a watched skills root changes on disk outside this worker — including the \
         read-only agents skills root, when that root exists at startup.",
    );
    let system_prompts = register_on_change(
        iii,
        SYSTEM_PROMPTS_ON_CHANGE,
        ON_SYSTEM_PROMPTS_CHANGE_FN,
        "Fires after a directory::skills::download that wrote at least one system prompt, or a \
         directory::system-prompts::update, create, or delete. Also fires with \
         { op: \"external\" } when a watched system-prompts root changes on disk outside \
         this worker.",
    );
    let agents = register_on_change(
        iii,
        AGENTS_ON_CHANGE,
        ON_AGENTS_CHANGE_FN,
        "Fires after a directory::skills::download that wrote at least one agent profile, \
         or a directory::agents::update, create, or delete. Also fires with \
         { op: \"external\" } when a watched agents file changes on disk outside this \
         worker.",
    );
    let kits = register_on_change(
        iii,
        KITS_ON_CHANGE,
        ON_KITS_CHANGE_FN,
        "Fires when kit state changes: a plan is created or discarded \
         ({ op: \"plan\" | \"discard\", kit, plan_id }), an apply makes progress or \
         finishes ({ op: \"progress\", plan_id, steps } / { op: \"apply\", kit, plan_id }), \
         or an update check finds a different set of available updates \
         ({ op: \"updates\" }).",
    );

    RegisteredTriggerTypes {
        skills,
        system_prompts,
        agents,
        kits,
    }
}

/// Register `trigger_type` and the observable `function_id` next to it over
/// one fresh [`SubscriberSet`], and return that set for the fan-out.
fn register_on_change(
    iii: &Arc<IIIClient>,
    trigger_type: &'static str,
    function_id: &'static str,
    description: &str,
) -> SubscriberSet {
    let subscribers = SubscriberSet::new();
    // Deprecated in 0.25 and still served by 0.24 and 0.25 engines: kept for
    // consumers that bind the trigger type.
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(RegisterTriggerType::new(
        trigger_type.to_string(),
        description.to_string(),
        SkillsTriggerHandler::new(trigger_type, subscribers.clone()),
    ));
    tracing::info!(trigger_type, "registered trigger type");

    iii.register_function(
        function_id,
        RegisterFunction::new(move |_event: Value| -> Result<(), Error> {
            Err(refuse_invocation(function_id))
        })
        .description(on_change_description(trigger_type, description))
        .observe(
            Observe::owner()
                .args_format::<OnChangeSpec>()
                .handler(subscribers.observe_handler(function_id)),
        ),
    );
    tracing::info!(function_id, "registered observable function");

    subscribers
}

/// The description of the observable function next to `trigger_type`.
fn on_change_description(trigger_type: &str, description: &str) -> String {
    format!(
        "{description} Observe it with args {{}}; every observer receives the event \
         {trigger_type} bindings receive. Only the iii-directory worker fires it: invoking it \
         directly is refused (FORBIDDEN)."
    )
}

struct SkillsTriggerHandler {
    name: String,
    subscribers: SubscriberSet,
}

impl SkillsTriggerHandler {
    fn new(name: &str, subscribers: SubscriberSet) -> Self {
        Self {
            name: name.into(),
            subscribers,
        }
    }
}

#[async_trait]
impl TriggerHandler for SkillsTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            function_id = %config.function_id,
            "trigger subscription registered"
        );
        self.subscribers.insert(config);
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(
            trigger_type = %self.name,
            id = %config.id,
            "trigger subscription unregistered"
        );
        self.subscribers.remove(&config.id);
        Ok(())
    }
}

/// Observers of a `directory::on-*-change` function, filed in the
/// [`SubscriberSet`] the matching trigger type's bindings live in: every
/// change reaches both.
#[derive(Clone)]
pub struct OnChangeObserveHandler {
    function_id: &'static str,
    subscribers: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for OnChangeObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format.
        let _: OnChangeSpec = obs.args()?;
        self.subscribers.add_observer(Arc::new(obs.clone()));
        tracing::info!(
            function_id = %self.function_id,
            observer = %obs.id(),
            target = %obs.target(),
            "observer added"
        );
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.subscribers.remove_observer(obs.id());
        tracing::info!(
            function_id = %self.function_id,
            observer = %obs.id(),
            "observer removed"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn make_config(id: &str, function_id: &str) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: function_id.to_string(),
            config: json!({}),
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn subscriber_set_insert_and_remove() {
        let set = SubscriberSet::new();
        assert!(set.targets().is_empty());
        let mut namespaced = make_config("sub-1", "mcp::__on_skills_changed");
        namespaced.namespace = Some("project-a".into());
        set.insert(namespaced);
        set.insert(make_config("sub-2", "other::receiver"));
        let mut targets = set.targets();
        targets.sort();
        assert_eq!(
            targets,
            vec![
                (
                    "mcp::__on_skills_changed".to_string(),
                    Some("project-a".to_string())
                ),
                ("other::receiver".to_string(), None)
            ]
        );
        set.remove("sub-1");
        assert_eq!(set.targets(), vec![("other::receiver".to_string(), None)]);
    }

    #[test]
    fn subscriber_set_duplicate_id_overwrites() {
        let set = SubscriberSet::new();
        set.insert(make_config("sub-1", "a"));
        set.insert(make_config("sub-1", "b"));
        assert_eq!(set.targets(), vec![("b".to_string(), None)]);
    }

    /// An observer of a `directory::on-*-change` function that forwards
    /// every event it is handed.
    struct RecordingObserver {
        id: String,
        tx: tokio::sync::mpsc::UnboundedSender<Value>,
    }

    #[async_trait]
    impl EventObserver for RecordingObserver {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, event: Value) -> Result<(), String> {
            self.tx.send(event).map_err(|e| e.to_string())
        }
    }

    fn recording_observer(
        id: &str,
    ) -> (
        Arc<RecordingObserver>,
        tokio::sync::mpsc::UnboundedReceiver<Value>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        (
            Arc::new(RecordingObserver {
                id: id.to_string(),
                tx,
            }),
            rx,
        )
    }

    async fn next_event(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Value>) -> Value {
        tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
            .await
            .expect("observer delivery timed out")
            .expect("observer channel closed")
    }

    const ON_CHANGE_FUNCTIONS: [(&str, &str); 4] = [
        (SKILLS_ON_CHANGE, ON_SKILLS_CHANGE_FN),
        (SYSTEM_PROMPTS_ON_CHANGE, ON_SYSTEM_PROMPTS_CHANGE_FN),
        (AGENTS_ON_CHANGE, ON_AGENTS_CHANGE_FN),
        (KITS_ON_CHANGE, ON_KITS_CHANGE_FN),
    ];

    #[test]
    fn each_on_change_function_refuses_direct_invocation() {
        assert_eq!(
            ON_CHANGE_FUNCTIONS.map(|(_, function_id)| function_id),
            [
                "directory::on-skills-change",
                "directory::on-system-prompts-change",
                "directory::on-agents-change",
                "directory::on-kits-change",
            ]
        );
        for (trigger_type, function_id) in ON_CHANGE_FUNCTIONS {
            let err = refuse_invocation(function_id);
            assert!(
                matches!(&err, Error::Remote { code, message, .. }
                    if code == FORBIDDEN && message.contains(function_id)),
                "{err:?}"
            );
            // The description tells callers the call is refused and which
            // trigger type's events observers receive.
            let description = on_change_description(trigger_type, "Fires.");
            assert!(description.contains("FORBIDDEN"), "{description}");
            assert!(description.contains(trigger_type), "{description}");
        }
    }

    #[test]
    fn observer_args_are_the_empty_binding_config() {
        assert!(serde_json::from_value::<OnChangeSpec>(json!({})).is_ok());
        let schema = serde_json::to_value(schemars::schema_for!(OnChangeSpec)).unwrap();
        assert_eq!(schema["type"], json!("object"), "{schema}");
    }

    #[test]
    fn observers_share_the_registry_but_are_not_bindings() {
        let set = SubscriberSet::new();
        set.insert(make_config("sub-1", "mcp::__on_skills_changed"));
        let (observer, _rx) = recording_observer("sub-1");
        set.add_observer(observer);
        // Same id as a binding: both stay, keyed apart.
        assert_eq!(
            set.targets(),
            vec![("mcp::__on_skills_changed".to_string(), None)]
        );
        assert_eq!(set.observers().len(), 1);
        assert_eq!(set.observers()[0].id(), "sub-1");

        // Removing the observer leaves the binding, and vice versa.
        set.remove_observer("sub-1");
        assert!(set.observers().is_empty());
        assert_eq!(set.targets().len(), 1);
        let (observer, _rx) = recording_observer("sub-1");
        set.add_observer(observer);
        set.remove("sub-1");
        assert!(set.targets().is_empty());
        assert_eq!(set.observers().len(), 1);
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        // Never connected: the binding's void trigger is queued and dropped,
        // the observer is handed the very same payload.
        let iii = IIIClient::new("ws://127.0.0.1:9");
        let set = SubscriberSet::new();
        set.insert(make_config("sub-1", "console::on-skills-changed"));
        let (observer, mut rx) = recording_observer("o1");
        set.add_observer(observer);
        // Another trigger type's set never sees this change.
        let other = SubscriberSet::new();
        let (other_observer, mut other_rx) = recording_observer("o2");
        other.add_observer(other_observer);

        let payload = json!({ "op": "update", "id": "skills/a.md" });
        assert_eq!(
            set.targets(),
            vec![("console::on-skills-changed".to_string(), None)],
            "the binding is a dispatch target"
        );
        dispatch(&iii, &set, payload.clone()).await;
        assert_eq!(next_event(&mut rx).await, payload);
        assert!(other_rx.try_recv().is_err());

        // Removed: no further events.
        set.remove_observer("o1");
        dispatch(&iii, &set, json!({ "op": "delete" })).await;
        tokio::task::yield_now().await;
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn every_observer_gets_each_event_without_bindings() {
        let set = SubscriberSet::new();
        let (a, mut a_rx) = recording_observer("a");
        let (b, mut b_rx) = recording_observer("b");
        set.add_observer(a);
        set.add_observer(b);
        let payload = json!({ "op": "external" });
        deliver_to_observers(&set, payload.clone());
        assert_eq!(next_event(&mut a_rx).await, payload);
        assert_eq!(next_event(&mut b_rx).await, payload);
    }
}
