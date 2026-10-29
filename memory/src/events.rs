//! The two custom trigger types this worker emits — `memory::item-changed`
//! and `memory::bank-changed` — using the session-manager reactivity model:
//! consumers bind handlers with the standard two-step pattern; the engine
//! routes each registration to our [`TriggerHandler`]s; delivery is
//! fire-and-forget (`TriggerAction::Void`), at-least-once, unordered. The
//! console memory page live-refreshes off these.
//!
//! On a 0.25 engine each trigger type has an owner-delivered observable
//! function next to it: `memory::on-item-changed` and
//! `memory::on-bank-changed`. Their observers are filed in the trigger
//! type's own [`SubscriberSet`], next to its bindings, take the binding
//! config as args, and receive every event a binding receives through
//! [`Observer::call`]. The functions' handlers refuse every invocation with
//! `FORBIDDEN`, so nobody can make this worker hand observers a forged event.

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
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::types::Memory;

pub const ITEM_CHANGED: &str = "memory::item-changed";
pub const ITEM_CHANGED_DESC: &str =
    "A memory was created, updated, superseded, or deleted in a memory bank. \
     Payload: { event_type, bank, memory, reason? } (reason on consolidation changes, and \
     `redacted: <labels>` when secrets or invisible characters were removed on save/update). Bind \
     console/live views here.";

pub const BANK_CHANGED: &str = "memory::bank-changed";
pub const BANK_CHANGED_DESC: &str =
    "A memory bank was created or trashed. Payload: { event_type, bank }.";

/// The observable function next to [`ITEM_CHANGED`].
pub const ON_ITEM_CHANGED: &str = "memory::on-item-changed";
pub const ON_ITEM_CHANGED_DESC: &str =
    "Fires when a memory is created, updated, superseded, or deleted in a memory bank. \
     Observe it with args { bank? } (only that bank's events when set); every observer \
     receives { event_type, bank, memory, reason? }, the event memory::item-changed bindings \
     receive. Only the memory worker fires it: invoking it directly is refused (FORBIDDEN).";

/// The observable function next to [`BANK_CHANGED`].
pub const ON_BANK_CHANGED: &str = "memory::on-bank-changed";
pub const ON_BANK_CHANGED_DESC: &str =
    "Fires when a memory bank is created or trashed. Observe it with args { bank? } (only that \
     bank's events when set); every observer receives { event_type, bank }, the event \
     memory::bank-changed bindings receive. Only the memory worker fires it: invoking it \
     directly is refused (FORBIDDEN).";

/// The code a direct invocation of an observable function is refused with.
pub const FORBIDDEN: &str = "FORBIDDEN";

/// The error an observable function's handler answers every invocation with.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args: {{}} }}, target)); \
             only the memory worker delivers its events"
        ),
        stacktrace: None,
    }
}

/// The event every `memory::on-item-changed` observer receives: exactly the
/// `memory::item-changed` binding payload, `{event_type, bank, memory, reason?}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ItemChangedEvent {
    /// `created`, `updated`, `superseded` or `deleted`.
    pub event_type: String,
    /// The bank the memory lives in.
    pub bank: String,
    /// The memory as it is after the change.
    pub memory: Memory,
    /// Why the change happened; present only when it carries one (e.g. a
    /// consolidation supersede).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// The event every `memory::on-bank-changed` observer receives: exactly the
/// `memory::bank-changed` binding payload, `{event_type, bank}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BankChangedEvent {
    /// What happened to the bank, e.g. `created` or `trashed`.
    pub event_type: String,
    /// The bank's name.
    pub bank: String,
}

/// The handler of `memory::on-item-changed`: its events come only from the
/// [`Emitter`]. Typed on the delivered event so the function publishes the
/// event's schema.
pub fn refuse_on_item_changed(_event: ItemChangedEvent) -> Result<(), Error> {
    Err(refuse_invocation(ON_ITEM_CHANGED))
}

/// The handler of `memory::on-bank-changed`: its events come only from the
/// [`Emitter`]. Typed on the delivered event so the function publishes the
/// event's schema.
pub fn refuse_on_bank_changed(_event: BankChangedEvent) -> Result<(), Error> {
    Err(refuse_invocation(ON_BANK_CHANGED))
}

/// Config accepted by both trigger types.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BindingConfig {
    /// Only deliver events for this bank.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bank: Option<String>,
}

/// `null` configs are treated as `{}`; unknown fields are rejected so a
/// misspelled filter key fails at registration, not silently.
pub fn parse_binding_config(raw: &Value) -> Result<BindingConfig, String> {
    if raw.is_null() {
        return Ok(BindingConfig::default());
    }
    serde_json::from_value(raw.clone()).map_err(|e| format!("invalid binding config: {e}"))
}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    namespace: Option<String>,
    filter: BindingConfig,
}

/// One observer as a sink of a trigger type's registry. Abstracted so the
/// fan-out is unit-testable without an engine.
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

#[derive(Clone)]
struct ObserverEntry {
    observer: Arc<dyn EventObserver>,
    filter: BindingConfig,
}

impl BindingConfig {
    /// Whether an event of `bank` passes this filter.
    fn matches(&self, bank: &str) -> bool {
        self.bank.as_deref().is_none_or(|want| want == bank)
    }
}

/// Thread-safe subscriber registry for one trigger type: its bindings and
/// the observers of the observable function next to it.
#[derive(Clone)]
pub struct SubscriberSet {
    trigger_type: &'static str,
    inner: Arc<Mutex<HashMap<String, Binding>>>,
    observers: Arc<Mutex<HashMap<String, ObserverEntry>>>,
}

impl SubscriberSet {
    fn new(trigger_type: &'static str) -> Self {
        Self {
            trigger_type,
            inner: Arc::new(Mutex::new(HashMap::new())),
            observers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    fn add(&self, config: TriggerConfig) -> Result<(), String> {
        let filter = parse_binding_config(&config.config)?;
        self.lock().insert(
            config.id,
            Binding {
                function_id: config.function_id,
                namespace: config.namespace,
                filter,
            },
        );
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.lock().remove(id);
    }

    fn snapshot(&self) -> Vec<Binding> {
        self.lock().values().cloned().collect()
    }

    /// Files an observer; `args` is its binding config.
    fn add_observer(&self, args: &Value, observer: Arc<dyn EventObserver>) -> Result<(), String> {
        let filter = parse_binding_config(args)?;
        let id = observer.id().to_string();
        self.lock_observers()
            .insert(id, ObserverEntry { observer, filter });
        Ok(())
    }

    fn remove_observer(&self, id: &str) {
        self.lock_observers().remove(id);
    }

    fn observers(&self) -> Vec<ObserverEntry> {
        self.lock_observers().values().cloned().collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Binding>> {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn lock_observers(&self) -> std::sync::MutexGuard<'_, HashMap<String, ObserverEntry>> {
        self.observers
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }
}

#[derive(Clone)]
pub struct TriggerSets {
    pub items: SubscriberSet,
    pub banks: SubscriberSet,
}

struct MemoryTriggerHandler {
    set: SubscriberSet,
}

#[async_trait]
impl TriggerHandler for MemoryTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let id = config.id.clone();
        self.set.add(config).map_err(Error::Handler)?;
        tracing::info!(trigger_type = self.set.trigger_type, id = %id, "trigger subscription registered");
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.set.remove(&config.id);
        Ok(())
    }
}

/// The observe hooks of `memory::on-item-changed` / `memory::on-bank-changed`:
/// observers join the registry of the trigger type next to the function.
struct MemoryObserveHandler {
    set: SubscriberSet,
}

#[async_trait]
impl ObserveHandler for MemoryObserveHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        self.set
            .add_observer(obs.args_value(), Arc::new(obs.clone()))
            .map_err(ObserveRejection::from)?;
        tracing::info!(trigger_type = self.set.trigger_type, observer = %obs.id(), target = %obs.target(), "observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.set.remove_observer(obs.id());
        tracing::info!(trigger_type = self.set.trigger_type, observer = %obs.id(), "observer removed");
    }
}

/// Register both trigger types with the engine, and next to each its
/// observable function over the same subscriber set. Must run **before**
/// `functions::register_all` so handlers capture the subscriber sets they
/// fan out to.
pub fn register_trigger_types(iii: &Arc<IIIClient>) -> TriggerSets {
    let sets = TriggerSets {
        items: SubscriberSet::new(ITEM_CHANGED),
        banks: SubscriberSet::new(BANK_CHANGED),
    };
    // Deprecated in 0.25 and still served by 0.24 and 0.25 engines: kept for
    // consumers that bind the trigger types.
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            ITEM_CHANGED,
            ITEM_CHANGED_DESC,
            MemoryTriggerHandler {
                set: sets.items.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            BANK_CHANGED,
            BANK_CHANGED_DESC,
            MemoryTriggerHandler {
                set: sets.banks.clone(),
            },
        )
        .trigger_request_format::<BindingConfig>(),
    );
    iii.register_function(
        ON_ITEM_CHANGED,
        RegisterFunction::new(refuse_on_item_changed)
            .description(ON_ITEM_CHANGED_DESC)
            .observe(Observe::owner().args_format::<BindingConfig>().handler(
                MemoryObserveHandler {
                    set: sets.items.clone(),
                },
            )),
    );
    iii.register_function(
        ON_BANK_CHANGED,
        RegisterFunction::new(refuse_on_bank_changed)
            .description(ON_BANK_CHANGED_DESC)
            .observe(Observe::owner().args_format::<BindingConfig>().handler(
                MemoryObserveHandler {
                    set: sets.banks.clone(),
                },
            )),
    );
    tracing::info!(
        trigger_types = ?[ITEM_CHANGED, BANK_CHANGED],
        observable_functions = ?[ON_ITEM_CHANGED, ON_BANK_CHANGED],
        "registered trigger types and observable functions"
    );
    sets
}

/// Memory event kinds carried in `event_type`.
#[derive(Debug, Clone, Copy)]
pub enum ItemEvent {
    Created,
    Updated,
    Superseded,
    Deleted,
}

impl ItemEvent {
    fn as_str(self) -> &'static str {
        match self {
            ItemEvent::Created => "created",
            ItemEvent::Updated => "updated",
            ItemEvent::Superseded => "superseded",
            ItemEvent::Deleted => "deleted",
        }
    }
}

/// The `memory::item-changed` payload: `{event_type, bank, memory, reason?}`
/// — `reason` only when the change carries one.
fn item_payload(event: ItemEvent, bank: &str, memory: &Memory, reason: Option<&str>) -> Value {
    let mut payload = json!({
        "event_type": event.as_str(),
        "bank": bank,
        "memory": memory,
    });
    if let Some(reason) = reason {
        payload["reason"] = json!(reason);
    }
    payload
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

    pub async fn item(&self, event: ItemEvent, bank: &str, memory: &Memory) {
        self.item_with(event, bank, memory, None).await;
    }

    /// `item` plus the why (`memory::supersede`'s `reason`), carried on the
    /// event so consumers can explain automatic changes.
    pub async fn item_with(
        &self,
        event: ItemEvent,
        bank: &str,
        memory: &Memory,
        reason: Option<&str>,
    ) {
        let payload = item_payload(event, bank, memory, reason);
        self.fan_out(&self.sets.items, bank, payload).await;
    }

    pub async fn bank(&self, event_type: &str, bank: &str) {
        let payload = json!({ "event_type": event_type, "bank": bank });
        self.fan_out(&self.sets.banks, bank, payload).await;
    }

    async fn fan_out(&self, set: &SubscriberSet, bank: &str, payload: Value) {
        for binding in set.snapshot() {
            if !binding.filter.matches(bank) {
                continue;
            }
            let request = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: payload.clone(),
                action: Some(TriggerAction::Void),
                timeout_ms: None,
            };
            let namespace = binding.namespace.as_deref().unwrap_or("default");
            let res = self.iii.trigger(request.namespace(namespace)).await;
            if let Err(e) = res {
                tracing::warn!(function_id = %binding.function_id, error = %e, "void fan-out failed");
            }
        }
        // Observers get the same event; each delivery runs on its own task so
        // a slow target never holds up the write that emitted it, as a void
        // binding does not.
        for entry in set.observers() {
            if !entry.filter.matches(bank) {
                continue;
            }
            let observer = entry.observer.clone();
            let event = payload.clone();
            tokio::spawn(async move {
                if let Err(error) = observer.deliver(event).await {
                    tracing::warn!(observer = %observer.id(), %error, "observer delivery failed");
                }
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn memory_fixture() -> Memory {
        Memory {
            id: "fp1".into(),
            text: "t".into(),
            entities: vec![],
            tags: vec![],
            confidence: crate::types::Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: 1,
            updated_at: 1,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: None,
        }
    }

    /// The structs the on-* functions are typed on describe exactly what
    /// the emitter delivers.
    #[test]
    fn delivered_payloads_are_the_typed_events() {
        let m = memory_fixture();
        let with = item_payload(ItemEvent::Superseded, "b", &m, Some("same fact"));
        let event: ItemChangedEvent = serde_json::from_value(with.clone()).unwrap();
        assert_eq!(event.reason.as_deref(), Some("same fact"));
        assert_eq!(serde_json::to_value(&event).unwrap(), with);
        let without = item_payload(ItemEvent::Created, "b", &m, None);
        let event: ItemChangedEvent = serde_json::from_value(without.clone()).unwrap();
        assert_eq!(serde_json::to_value(&event).unwrap(), without);
        let bank = json!({ "event_type": "trashed", "bank": "blog" });
        let event: BankChangedEvent = serde_json::from_value(bank.clone()).unwrap();
        assert_eq!(serde_json::to_value(&event).unwrap(), bank);
    }

    #[test]
    fn null_config_is_match_all() {
        assert!(parse_binding_config(&Value::Null).unwrap().bank.is_none());
    }

    #[test]
    fn unknown_config_fields_are_rejected() {
        assert!(parse_binding_config(&json!({ "bnak": "typo" })).is_err());
    }

    #[test]
    fn bank_filter_parses() {
        let cfg = parse_binding_config(&json!({ "bank": "blog" })).unwrap();
        assert_eq!(cfg.bank.as_deref(), Some("blog"));
    }

    #[test]
    fn subscriber_preserves_target_namespace() {
        let set = SubscriberSet::new(ITEM_CHANGED);
        set.add(TriggerConfig {
            id: "sub-1".into(),
            function_id: "ui::refresh".into(),
            config: json!({}),
            metadata: None,
            namespace: Some("project-a".into()),
        })
        .unwrap();
        assert_eq!(set.snapshot()[0].namespace.as_deref(), Some("project-a"));
    }

    #[test]
    fn item_payload_carries_reason_only_when_given() {
        let m = memory_fixture();
        let with = item_payload(ItemEvent::Superseded, "b", &m, Some("same fact"));
        assert_eq!(with["event_type"], "superseded");
        assert_eq!(with["bank"], "b");
        assert_eq!(with["memory"]["id"], "fp1");
        assert_eq!(with["reason"], "same fact");
        let without = item_payload(ItemEvent::Updated, "b", &m, None);
        assert!(without.get("reason").is_none());
    }

    use std::time::Duration;
    use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

    /// An observer that forwards `(id, event)` for every delivery it gets.
    struct Recording {
        id: String,
        tx: UnboundedSender<(String, Value)>,
    }

    #[async_trait]
    impl EventObserver for Recording {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, event: Value) -> Result<(), String> {
            let _ = self.tx.send((self.id.clone(), event));
            Ok(())
        }
    }

    fn recording(id: &str, tx: &UnboundedSender<(String, Value)>) -> Arc<dyn EventObserver> {
        Arc::new(Recording {
            id: id.to_string(),
            tx: tx.clone(),
        })
    }

    async fn next(rx: &mut UnboundedReceiver<(String, Value)>) -> (String, Value) {
        tokio::time::timeout(Duration::from_secs(5), rx.recv())
            .await
            .expect("an observer delivery")
            .expect("channel open")
    }

    async fn assert_quiet(rx: &mut UnboundedReceiver<(String, Value)>) {
        let extra = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(extra.is_err(), "unexpected delivery: {extra:?}");
    }

    fn emitter() -> (Emitter, TriggerSets) {
        let sets = TriggerSets {
            items: SubscriberSet::new(ITEM_CHANGED),
            banks: SubscriberSet::new(BANK_CHANGED),
        };
        // Never connected: a Void trigger to it returns at once.
        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        (Emitter::new(sets.clone(), iii), sets)
    }

    #[test]
    fn on_functions_refuse_direct_invocation() {
        assert_eq!(ON_ITEM_CHANGED, "memory::on-item-changed");
        assert_eq!(ON_BANK_CHANGED, "memory::on-bank-changed");
        let item = ItemChangedEvent {
            event_type: "created".into(),
            bank: "b".into(),
            memory: memory_fixture(),
            reason: None,
        };
        let bank = BankChangedEvent {
            event_type: "created".into(),
            bank: "b".into(),
        };
        for (function_id, refused) in [
            (ON_ITEM_CHANGED, refuse_on_item_changed(item)),
            (ON_BANK_CHANGED, refuse_on_bank_changed(bank)),
        ] {
            let error = refused.unwrap_err();
            assert!(
                matches!(&error, Error::Remote { code, message, .. }
                    if code == FORBIDDEN && message.contains(function_id)),
                "{error:?}"
            );
        }
    }

    #[test]
    fn observer_args_are_the_binding_config() {
        let set = SubscriberSet::new(ITEM_CHANGED);
        let (tx, _rx) = unbounded_channel();
        assert!(set
            .add_observer(&json!({ "bnak": "typo" }), recording("bad", &tx))
            .is_err());
        assert!(set.observers().is_empty());
        set.add_observer(&Value::Null, recording("all", &tx))
            .unwrap();
        set.add_observer(&json!({ "bank": "blog" }), recording("blog", &tx))
            .unwrap();
        // Re-adding an id replaces it; removing it drops it; bindings untouched.
        set.add_observer(&json!({}), recording("all", &tx)).unwrap();
        assert_eq!(set.observers().len(), 2);
        set.remove_observer("all");
        assert_eq!(set.observers().len(), 1);
        assert!(set.snapshot().is_empty());
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        let (emitter, sets) = emitter();
        let (tx, mut rx) = unbounded_channel();
        sets.banks
            .add(TriggerConfig {
                id: "b-1".into(),
                function_id: "ui::refresh".into(),
                config: json!({ "bank": "blog" }),
                metadata: None,
                namespace: None,
            })
            .unwrap();
        sets.banks
            .add_observer(&json!({ "bank": "blog" }), recording("blog", &tx))
            .unwrap();
        sets.banks
            .add_observer(&json!({ "bank": "other" }), recording("other", &tx))
            .unwrap();
        sets.items
            .add_observer(&json!({}), recording("items", &tx))
            .unwrap();

        // The binding this event reaches, and the observer with its config.
        let bound: Vec<_> = sets
            .banks
            .snapshot()
            .into_iter()
            .filter(|b| b.filter.matches("blog"))
            .collect();
        assert_eq!(bound.len(), 1);
        emitter.bank("created", "blog").await;
        let (id, event) = next(&mut rx).await;
        assert_eq!(id, "blog");
        assert_eq!(event, json!({ "event_type": "created", "bank": "blog" }));
        assert_quiet(&mut rx).await;

        // Removed: no more deliveries.
        sets.banks.remove_observer("blog");
        emitter.bank("trashed", "blog").await;
        assert_quiet(&mut rx).await;
        assert_eq!(sets.banks.snapshot().len(), 1, "the binding stays");
    }

    #[tokio::test]
    async fn an_item_observer_receives_the_item_payload() {
        let (emitter, sets) = emitter();
        let (tx, mut rx) = unbounded_channel();
        sets.items
            .add_observer(&Value::Null, recording("all", &tx))
            .unwrap();
        sets.banks
            .add_observer(&Value::Null, recording("banks", &tx))
            .unwrap();
        let m = memory_fixture();
        emitter
            .item_with(ItemEvent::Superseded, "b", &m, Some("same fact"))
            .await;
        let (id, event) = next(&mut rx).await;
        assert_eq!(id, "all");
        assert_eq!(
            event,
            item_payload(ItemEvent::Superseded, "b", &m, Some("same fact"))
        );
        assert_quiet(&mut rx).await;
    }
}
