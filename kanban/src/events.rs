//! The two trigger types this worker provides, and the observable
//! `kanban::on-*` function next to each one (iii 0.25).
//!
//! - `kanban:change` fires after every mutation with the whole ticket
//!   (a `TicketView`: parent, live children and progress included). Config
//!   `{ ticket_id?, parent_id?, release_id?, events?, metadata? }`:
//!   `ticket_id` matches the uuid or the human key; `parent_id` (uuid or key)
//!   fires for that ticket and for each of its subtasks; `release_id` (id or
//!   name) fires for a ticket in that release and for one a write just took
//!   out of it; `events` narrows by event name. An unknown reference matches
//!   nothing. A write that touches two tickets (attach, detach) fires once
//!   per ticket. `config.updated` (`{ event, at }`, no ticket) fires after
//!   the configuration (columns, statuses, priorities) was reloaded; only a
//!   binding without a ticket, parent or release filter gets it.
//! - `kanban:comment` fires once per comment created. Config
//!   `{ ticket_id?, parent_id?, author?, exclude_author?, root_only? }`: a
//!   comment on that ticket (`ticket_id`) or on that ticket or any of its
//!   subtasks (`parent_id`); a binding with neither never fires.
//!
//! Bindings live in an in-process map keyed by trigger id (the SDK populates
//! only `id` on unregister). Delivery is fire-and-forget (`Void`), in the
//! binding's own namespace, with the binding's metadata as the sidecar.
//!
//! Next to each type, an owner-delivered observable function delivers the
//! same event: `kanban::on-change` (`kanban:change`) and `kanban::on-comment`
//! (`kanban:comment`). An observer's args are the trigger config and filter
//! exactly as a binding's config does (`ticket: summary` included); its
//! [`ObserveHandler`] files it in the observer half of the type's subscriber
//! registry, and events reach it with `obs.call`. Only this worker emits
//! them: invoking an on-* function directly is refused with FORBIDDEN.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{
    IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction,
    RegisterTriggerType,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::board::{ReleaseRef, TicketSummary, TicketView, summarize};
use crate::store::{Activity, Comment};

pub const CHANGE_TRIGGER: &str = "kanban:change";
pub const COMMENT_TRIGGER: &str = "kanban:comment";

/// The observable function next to `kanban:change`.
pub const ON_CHANGE_FN: &str = "kanban::on-change";
/// The observable function next to `kanban:comment`.
pub const ON_COMMENT_FN: &str = "kanban::on-comment";

/// The code an on-* function refuses direct invocation with.
pub const FORBIDDEN: &str = "FORBIDDEN";

const ON_CHANGE_DESC: &str = "Fires after every kanban mutation with the whole ticket (parent, subtasks and progress included), and with { event: config.updated, at } after the configuration was reloaded. Observe it with the kanban:change config as args: { ticket_id?, parent_id?, release_id?, events?, ticket? }. Only the kanban worker emits it: invoking it directly is refused.";
const ON_COMMENT_DESC: &str = "Fires when a comment is added. Observe it with the kanban:comment config as args: { ticket_id? | parent_id?, author?, exclude_author?, root_only?, ticket? } (args with neither ticket_id nor parent_id never fire). Only the kanban worker emits it: invoking it directly is refused.";

/// Every event name `kanban:change` can carry.
pub const CHANGE_EVENTS: [&str; 7] = [
    "ticket.created",
    "ticket.updated",
    "ticket.moved",
    "ticket.deleted",
    "ticket.restored",
    "comment.created",
    CONFIG_EVENT,
];

/// The one `kanban:change` event that carries no ticket: the configuration
/// was reloaded, so columns or statuses may have changed.
pub const CONFIG_EVENT: &str = "config.updated";

/// How much of the ticket a delivery carries. `full` is the ticket with its
/// comments and activity inline; `summary` is the board-card projection
/// (key, title, status, priority, assignee, labels, order, timestamps,
/// comment_count) — the right choice for an agent that re-reads the ticket
/// with `kanban::ticket::get` anyway.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum TicketDetail {
    #[default]
    Full,
    Summary,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct ChangeConfig {
    /// Ticket uuid or human key. Omit to fire for every ticket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    /// Ticket uuid or human key of a parent: fires for the parent itself and for each of its subtasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Release id or name: fires for a ticket in the release and for a ticket a write just moved out of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_id: Option<String>,
    /// `full` (default) delivers the whole ticket; `summary` only its board-card fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketDetail>,
    /// Event names to fire for (ticket.created, ticket.updated, ticket.moved,
    /// ticket.deleted, ticket.restored, comment.created, config.updated). Omit for all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
    /// Metadata attached to every invocation this binding receives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommentConfig {
    /// Ticket uuid or human key. Give this or `parent_id`; a binding with neither never fires.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    /// Ticket uuid or human key of a parent: comments on the parent or on any of its subtasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Fire only for comments by exactly this author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Fire for every author except this one — pass your own profile id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_author: Option<String>,
    /// Ignore replies; fire only for top-level comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_only: Option<bool>,
    /// `full` (default) delivers the whole ticket; `summary` only its board-card fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketDetail>,
}

/// The `ticket` field of a delivery: the whole ticket, or its summary when the
/// binding asked for `ticket: summary`.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TicketPayload {
    Full(Box<TicketView>),
    Summary(Box<TicketSummary>),
}

impl From<TicketView> for TicketPayload {
    fn from(view: TicketView) -> Self {
        TicketPayload::Full(Box::new(view))
    }
}

impl TicketPayload {
    pub fn summarized(&self) -> TicketPayload {
        match self {
            TicketPayload::Full(view) => TicketPayload::Summary(Box::new(summarize(&view.ticket))),
            TicketPayload::Summary(summary) => TicketPayload::Summary(summary.clone()),
        }
    }

    /// The parent's uuid and, when the payload carries it, its key.
    fn parent(&self) -> Option<(&str, Option<&str>)> {
        match self {
            TicketPayload::Full(view) => view
                .parent
                .as_ref()
                .map(|parent| (parent.id.as_str(), Some(parent.key.as_str()))),
            TicketPayload::Summary(summary) => summary.parent_id.as_deref().map(|id| (id, None)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChangeEvent {
    pub event: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub ticket: TicketPayload,
    pub comment: Option<Comment>,
    pub activity: Option<Activity>,
    pub at: String,
    /// The releases the ticket is in or this write took it out of; what a
    /// `release_id` filter matches against. Not part of the delivery.
    #[serde(skip)]
    pub releases: Vec<ReleaseRef>,
}

/// `config.updated` on `kanban:change`: the board's configuration changed.
/// It names no ticket.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigEvent {
    /// Always `config.updated`.
    pub event: String,
    pub at: String,
}

/// What a `kanban:change` binding receives: a ticket event, or a
/// configuration event. Published as the trigger's call format only.
#[derive(JsonSchema)]
#[serde(untagged)]
#[allow(dead_code, clippy::large_enum_variant)]
enum ChangeDelivery {
    Ticket(ChangeEvent),
    Config(ConfigEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommentEvent {
    /// Always `comment.created`.
    pub event: String,
    pub ticket_id: String,
    pub ticket_key: String,
    pub ticket: TicketPayload,
    pub comment: Comment,
    pub author: String,
    pub at: String,
}

pub type Bindings = Arc<RwLock<HashMap<String, TriggerConfig>>>;

/// One observer of a `kanban::on-*` function, as the emitter sees it.
/// Production wraps the SDK's [`Observer`]; tests record.
pub trait EventObserver: Send + Sync {
    fn id(&self) -> &str;
    /// Hand one event to the observer. Fire-and-forget like a binding's
    /// delivery: never blocks the write that produced the event.
    fn deliver(&self, payload: Value);
}

impl EventObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn deliver(&self, payload: Value) {
        let obs = self.clone();
        tokio::spawn(async move {
            match obs.call::<Value>(payload).await {
                Ok(_) => {}
                // The observer's own filters declined this event.
                Err(Error::Remote { code, .. }) if code == "FILTERS_NOT_MATCHED" => {}
                Err(error) => tracing::warn!(
                    observer = %obs.id(),
                    function_id = %obs.function_id(),
                    error = %error,
                    "kanban observer delivery failed"
                ),
            }
        });
    }
}

/// Observers of one on-* function keyed by observer id, each with its args
/// (the trigger config it filters on).
pub type Observers = Arc<RwLock<HashMap<String, (Arc<dyn EventObserver>, Value)>>>;

/// The subscriber registry: per type, its bindings and the observers of the
/// on-* function next to it, kept apart by id.
#[derive(Clone, Default)]
pub struct Subscribers {
    pub change: Bindings,
    pub comment: Bindings,
    pub change_observers: Observers,
    pub comment_observers: Observers,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct SubscriberCounts {
    pub change: usize,
    pub comment: usize,
}

fn len<T>(table: &RwLock<HashMap<String, T>>) -> usize {
    table.read().unwrap_or_else(|p| p.into_inner()).len()
}

impl Subscribers {
    /// Bindings plus observers, per type.
    pub fn counts(&self) -> SubscriberCounts {
        SubscriberCounts {
            change: len(&self.change) + len(&self.change_observers),
            comment: len(&self.comment) + len(&self.comment_observers),
        }
    }
}

/// File an observer with its args (null means no filter, like an empty
/// binding config). Malformed args are accepted and simply never match, as
/// a binding's config is.
pub fn add_observer(table: &Observers, observer: Arc<dyn EventObserver>, args: Value) {
    let args = if args.is_null() {
        Value::Object(Default::default())
    } else {
        args
    };
    let id = observer.id().to_string();
    table
        .write()
        .unwrap_or_else(|p| p.into_inner())
        .insert(id, (observer, args));
}

pub fn remove_observer(table: &Observers, id: &str) {
    table.write().unwrap_or_else(|p| p.into_inner()).remove(id);
}

/// One handler shape for both types: remember the binding, forget it on
/// unregister. A malformed config is accepted here and simply never matches.
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

/// Files each observer of a `kanban::on-*` function in the observer half of
/// its trigger type's subscriber registry, and takes it out again when the
/// engine removes it.
struct ObserverTable(Observers);

#[async_trait]
impl ObserveHandler for ObserverTable {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        add_observer(&self.0, Arc::new(obs.clone()), obs.args_value().clone());
        tracing::info!(observer = %obs.id(), target = %obs.target(), function_id = %obs.function_id(), "kanban observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        remove_observer(&self.0, obs.id());
        tracing::info!(observer = %obs.id(), function_id = %obs.function_id(), "kanban observer removed");
    }
}

/// The error every `kanban::on-*` handler returns: events come only from
/// this worker, in-process, never by invoking the function.
pub fn refuse_invocation(function_id: &str) -> Error {
    Error::Remote {
        code: FORBIDDEN.to_string(),
        message: format!(
            "{function_id} is an event source: observe it \
             (iii.observe({{ function_id: '{function_id}', args }}, target)) instead of invoking \
             it; only the kanban worker emits its events"
        ),
        stacktrace: None,
    }
}

/// Handler of `kanban::on-change`: always FORBIDDEN.
pub fn refuse_on_change(_event: Value) -> Result<(), Error> {
    Err(refuse_invocation(ON_CHANGE_FN))
}

/// Handler of `kanban::on-comment`: always FORBIDDEN.
pub fn refuse_on_comment(_event: Value) -> Result<(), Error> {
    Err(refuse_invocation(ON_COMMENT_FN))
}

/// Register both trigger types and, next to each, its owner-delivered
/// observable function over the same subscriber registry. On an engine
/// without observe the functions register as plain functions whose hooks
/// never run.
pub fn register_trigger_types(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    // Deprecated in 0.25 and still served by 0.24 and 0.25 engines: kept for
    // consumers that bind the trigger types.
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGE_TRIGGER,
            "Fires after every kanban mutation with the whole ticket (parent, subtasks and progress included). Config: { ticket_id?, parent_id?, release_id?, events?, metadata? } — ticket_id and parent_id match the uuid or the human key (parent_id also fires for each subtask); release_id is a release id or name (fires for tickets in it and for tickets moved out of it).",
            BindingTable(subscribers.change.clone()),
        )
        .trigger_request_format::<ChangeConfig>()
        .call_request_format::<ChangeDelivery>(),
    );
    #[allow(deprecated)]
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            COMMENT_TRIGGER,
            "Fires when a comment is added, optionally filtered by author. Config: { ticket_id? | parent_id?, author?, exclude_author?, root_only? } — ticket_id is one ticket; parent_id is a ticket and all its subtasks (uuid or human key).",
            BindingTable(subscribers.comment.clone()),
        )
        .trigger_request_format::<CommentConfig>()
        .call_request_format::<CommentEvent>(),
    );
    iii.register_function(
        ON_CHANGE_FN,
        RegisterFunction::new(refuse_on_change)
            .description(ON_CHANGE_DESC)
            .observe(
                Observe::owner()
                    .args_format::<ChangeConfig>()
                    .handler(ObserverTable(subscribers.change_observers.clone())),
            ),
    );
    iii.register_function(
        ON_COMMENT_FN,
        RegisterFunction::new(refuse_on_comment)
            .description(ON_COMMENT_DESC)
            .observe(
                Observe::owner()
                    .args_format::<CommentConfig>()
                    .handler(ObserverTable(subscribers.comment_observers.clone())),
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

/// An empty reference is no filter; otherwise the uuid or the key, case-insensitively.
pub fn matches_ticket(reference: &str, ticket_id: &str, ticket_key: &str) -> bool {
    let wanted = reference.trim();
    if wanted.is_empty() {
        return true;
    }
    wanted.eq_ignore_ascii_case(ticket_id) || wanted.eq_ignore_ascii_case(ticket_key)
}

/// A ticket that is `reference` itself or a subtask of it (uuid or key).
fn in_family(reference: &str, ticket_id: &str, ticket_key: &str, ticket: &TicketPayload) -> bool {
    matches_ticket(reference, ticket_id, ticket_key)
        || ticket
            .parent()
            .is_some_and(|(id, key)| matches_ticket(reference, id, key.unwrap_or_default()))
}

pub fn matches_change(config: &Value, event: &ChangeEvent) -> bool {
    if !matches_ticket(
        text(config, "ticket_id"),
        &event.ticket_id,
        &event.ticket_key,
    ) {
        return false;
    }
    let parent = text(config, "parent_id");
    if !parent.is_empty() && !in_family(parent, &event.ticket_id, &event.ticket_key, &event.ticket)
    {
        return false;
    }
    let release = text(config, "release_id");
    if !release.is_empty()
        && !event.releases.iter().any(|candidate| {
            release.eq_ignore_ascii_case(&candidate.id)
                || release.eq_ignore_ascii_case(candidate.name.trim())
        })
    {
        return false;
    }
    let events = event_names(config);
    events.is_empty() || events.contains(&event.event.as_str())
}

fn event_names(config: &Value) -> Vec<&str> {
    config
        .get("events")
        .and_then(Value::as_array)
        .map(|entries| {
            entries
                .iter()
                .filter_map(Value::as_str)
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .collect()
        })
        .unwrap_or_default()
}

/// `config.updated` names no ticket: a binding that filters on one (or a
/// parent or release) never gets it; `events` still narrows.
pub fn matches_config(config: &Value) -> bool {
    if ["ticket_id", "parent_id", "release_id"]
        .iter()
        .any(|key| !text(config, key).is_empty())
    {
        return false;
    }
    let events = event_names(config);
    events.is_empty() || events.contains(&CONFIG_EVENT)
}

pub fn matches_comment(config: &Value, event: &CommentEvent) -> bool {
    let (reference, parent) = (text(config, "ticket_id"), text(config, "parent_id"));
    if reference.is_empty() && parent.is_empty() {
        return false;
    }
    if !matches_ticket(reference, &event.ticket_id, &event.ticket_key) {
        return false;
    }
    if !parent.is_empty() && !in_family(parent, &event.ticket_id, &event.ticket_key, &event.ticket)
    {
        return false;
    }
    let author = text(config, "author");
    if !author.is_empty() && event.author != author {
        return false;
    }
    let exclude = text(config, "exclude_author");
    if !exclude.is_empty() && event.author == exclude {
        return false;
    }
    if config.get("root_only").and_then(Value::as_bool) == Some(true)
        && event.comment.parent_id.is_some()
    {
        return false;
    }
    true
}

/// Metadata a subscription wants on every invocation: `config.metadata`
/// first (the console's per-tab bindings carry it there), else the
/// registration metadata.
pub fn subscription_metadata(binding: &TriggerConfig) -> Option<Value> {
    binding
        .config
        .get("metadata")
        .filter(|value| !value.is_null())
        .cloned()
        .or_else(|| binding.metadata.clone())
}

fn matching(table: &Bindings, keep: impl Fn(&TriggerConfig) -> bool) -> Vec<TriggerConfig> {
    table
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|binding| keep(binding))
        .cloned()
        .collect()
}

/// Whether a binding asked for the summary-sized payload.
pub fn wants_summary(config: &Value) -> bool {
    config.get("ticket").and_then(Value::as_str) == Some("summary")
}

/// The two wire shapes of one event: `full` as serialized, `summary` with the
/// `ticket` field replaced by its board-card projection.
pub fn payloads<T: Serialize>(event: &T, ticket: &TicketPayload) -> Result<(Value, Value), String> {
    let full = serde_json::to_value(event).map_err(|e| e.to_string())?;
    let mut summary = full.clone();
    summary["ticket"] = serde_json::to_value(ticket.summarized()).map_err(|e| e.to_string())?;
    Ok((full, summary))
}

/// Hand the event to every observer whose args match: the `summary` shape
/// when its args ask for `ticket: summary`, else the `full` one.
fn notify_observers(
    table: &Observers,
    keep: impl Fn(&Value) -> bool,
    full: &Value,
    summary: &Value,
) {
    let observers: Vec<(Arc<dyn EventObserver>, bool)> = table
        .read()
        .unwrap_or_else(|p| p.into_inner())
        .values()
        .filter(|(_, args)| keep(args))
        .map(|(observer, args)| (observer.clone(), wants_summary(args)))
        .collect();
    for (observer, summarized) in observers {
        observer.deliver(if summarized {
            summary.clone()
        } else {
            full.clone()
        });
    }
}

fn deliver(iii: Arc<IIIClient>, bindings: Vec<TriggerConfig>, full: Value, summary: Value) {
    if bindings.is_empty() {
        return;
    }
    tokio::spawn(async move {
        for binding in bindings {
            let payload = if wants_summary(&binding.config) {
                summary.clone()
            } else {
                full.clone()
            };
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
                tracing::warn!(
                    function_id = binding.function_id,
                    error = %error,
                    "kanban trigger delivery failed"
                );
            }
        }
    });
}

pub fn emit_change(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &ChangeEvent) {
    let bindings = matching(&subscribers.change, |binding| {
        matches_change(&binding.config, event)
    });
    match payloads(event, &event.ticket) {
        Ok((full, summary)) => {
            notify_observers(
                &subscribers.change_observers,
                |args| matches_change(args, event),
                &full,
                &summary,
            );
            deliver(iii.clone(), bindings, full, summary);
        }
        Err(error) => tracing::error!(error = %error, "kanban change event failed to serialize"),
    }
}

/// Tell every unfiltered `kanban:change` binding and `kanban::on-change`
/// observer the configuration changed.
pub fn emit_config_updated(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    let bindings = matching(&subscribers.change, |binding| {
        matches_config(&binding.config)
    });
    let event = ConfigEvent {
        event: CONFIG_EVENT.to_string(),
        at: crate::board::now(),
    };
    match serde_json::to_value(&event) {
        Ok(payload) => {
            notify_observers(
                &subscribers.change_observers,
                matches_config,
                &payload,
                &payload,
            );
            deliver(iii.clone(), bindings, payload.clone(), payload);
        }
        Err(error) => tracing::error!(error = %error, "kanban config event failed to serialize"),
    }
}

pub fn emit_comment(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &CommentEvent) {
    let bindings = matching(&subscribers.comment, |binding| {
        matches_comment(&binding.config, event)
    });
    match payloads(event, &event.ticket) {
        Ok((full, summary)) => {
            notify_observers(
                &subscribers.comment_observers,
                |args| matches_comment(args, event),
                &full,
                &summary,
            );
            deliver(iii.clone(), bindings, full, summary);
        }
        Err(error) => tracing::error!(error = %error, "kanban comment event failed to serialize"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn change_call_format_admits_the_config_event() {
        let schema = serde_json::to_value(schemars::schema_for!(ChangeDelivery)).unwrap();
        let variants = schema["anyOf"]
            .as_array()
            .expect("one schema per delivery shape");
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn config_event_reaches_unfiltered_bindings_only() {
        assert!(matches_config(&json!({})));
        assert!(matches_config(
            &json!({ "events": ["config.updated", "ticket.moved"] })
        ));
        assert!(!matches_config(&json!({ "events": ["ticket.moved"] })));
        assert!(!matches_config(&json!({ "ticket_id": "KAN-1" })));
        assert!(!matches_config(&json!({ "parent_id": "KAN-1" })));
        assert!(!matches_config(&json!({ "release_id": "r1" })));
        assert!(matches_config(&json!({ "ticket_id": "  " })));
    }

    /// An observer of a `kanban::on-*` function that records what it is handed.
    struct Recording {
        id: String,
        received: std::sync::Mutex<Vec<Value>>,
    }

    impl Recording {
        fn new(id: &str) -> Arc<Self> {
            Arc::new(Self {
                id: id.to_string(),
                received: std::sync::Mutex::new(Vec::new()),
            })
        }

        fn received(&self) -> Vec<Value> {
            self.received.lock().unwrap().clone()
        }
    }

    impl EventObserver for Recording {
        fn id(&self) -> &str {
            &self.id
        }

        fn deliver(&self, payload: Value) {
            self.received.lock().unwrap().push(payload);
        }
    }

    fn binding(id: &str, config: Value) -> TriggerConfig {
        TriggerConfig {
            id: id.to_string(),
            function_id: format!("consumer::{id}"),
            config,
            metadata: None,
            namespace: None,
        }
    }

    #[test]
    fn on_functions_refuse_direct_invocation() {
        assert_eq!(ON_CHANGE_FN, "kanban::on-change");
        assert_eq!(ON_COMMENT_FN, "kanban::on-comment");
        for (function_id, refused) in [
            (ON_CHANGE_FN, refuse_on_change(json!({}))),
            (
                ON_CHANGE_FN,
                refuse_on_change(json!({ "event": "ticket.moved" })),
            ),
            (ON_COMMENT_FN, refuse_on_comment(Value::Null)),
            (
                ON_COMMENT_FN,
                refuse_on_comment(json!({ "ticket_id": "KAN-1" })),
            ),
        ] {
            let error = refused.unwrap_err();
            assert!(
                matches!(&error, Error::Remote { code, message, .. }
                    if code == FORBIDDEN && message.contains(function_id)),
                "{error:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_change_observer_receives_the_event_a_binding_receives() {
        let subscribers = Subscribers::default();
        for (id, config) in [
            ("all", json!({})),
            ("moves", json!({ "events": ["ticket.moved"] })),
        ] {
            subscribers
                .change
                .write()
                .unwrap()
                .insert(id.to_string(), binding(id, config));
        }
        let all = Recording::new("o-all");
        let moves = Recording::new("o-moves");
        add_observer(&subscribers.change_observers, all.clone(), json!({}));
        add_observer(
            &subscribers.change_observers,
            moves.clone(),
            json!({ "events": ["ticket.moved"] }),
        );
        let counts = subscribers.counts();
        assert_eq!((counts.change, counts.comment), (4, 0));

        // The bindings this event reaches, and the observers with the same
        // config: the unfiltered one only.
        let bound = matching(&subscribers.change, |b| matches_config(&b.config));
        assert_eq!(bound.len(), 1);
        assert_eq!(bound[0].id, "all");

        let iii = Arc::new(IIIClient::new("ws://127.0.0.1:1"));
        emit_config_updated(&iii, &subscribers);
        let received = all.received();
        assert_eq!(received.len(), 1);
        assert_eq!(received[0]["event"], CONFIG_EVENT);
        assert!(received[0].get("at").is_some());
        assert!(moves.received().is_empty(), "filtered like its binding");

        remove_observer(&subscribers.change_observers, "o-all");
        emit_config_updated(&iii, &subscribers);
        assert_eq!(all.received().len(), 1);
        assert_eq!(subscribers.counts().change, 3);
    }

    #[test]
    fn observers_get_the_summary_or_full_payload_a_binding_with_the_same_config_gets() {
        let table = Observers::default();
        let full_obs = Recording::new("full");
        let summary_obs = Recording::new("summary");
        let other_ticket = Recording::new("other");
        // Null args are no filter, like an empty binding config.
        add_observer(&table, full_obs.clone(), Value::Null);
        let summary_args = json!({ "ticket_id": "KAN-1", "ticket": "summary" });
        add_observer(&table, summary_obs.clone(), summary_args.clone());
        add_observer(
            &table,
            other_ticket.clone(),
            json!({ "ticket_id": "KAN-2" }),
        );

        let full = json!({ "ticket": { "key": "KAN-1", "description": "long" } });
        let summary = json!({ "ticket": { "key": "KAN-1" } });
        let keep = |args: &Value| {
            args.get("ticket_id")
                .and_then(Value::as_str)
                .is_none_or(|id| id == "KAN-1")
        };
        notify_observers(&table, keep, &full, &summary);

        assert!(!wants_summary(&json!({})));
        assert!(wants_summary(&summary_args));
        assert_eq!(full_obs.received(), vec![full.clone()]);
        assert_eq!(summary_obs.received(), vec![summary]);
        assert!(other_ticket.received().is_empty());

        // Re-adding an id replaces it; removing it stops delivery.
        add_observer(&table, Recording::new("full"), json!({}));
        assert_eq!(len(&table), 3);
        remove_observer(&table, "full");
        assert_eq!(len(&table), 2);
    }
}
