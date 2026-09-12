//! The two trigger types this worker provides.
//!
//! - `kanban:change` fires after every mutation with the whole ticket.
//!   Config `{ ticket_id?, events?, metadata? }`: `ticket_id` matches the
//!   uuid or the human key, `events` narrows by event name.
//! - `kanban:comment` fires once per comment created on one ticket. Config
//!   `{ ticket_id, author?, exclude_author?, root_only? }`; a binding without
//!   `ticket_id` never fires.
//!
//! Bindings live in an in-process map keyed by trigger id (the SDK populates
//! only `id` on unregister). Delivery is fire-and-forget (`Void`), in the
//! binding's own namespace, with the binding's metadata as the sidecar.

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerAction, TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::board::{TicketSummary, summarize};
use crate::store::{Activity, Comment, Ticket};

pub const CHANGE_TRIGGER: &str = "kanban:change";
pub const COMMENT_TRIGGER: &str = "kanban:comment";

/// Every event name `kanban:change` can carry.
pub const CHANGE_EVENTS: [&str; 6] = [
    "ticket.created",
    "ticket.updated",
    "ticket.moved",
    "ticket.deleted",
    "ticket.restored",
    "comment.created",
];

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
    /// `full` (default) delivers the whole ticket; `summary` only its board-card fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketDetail>,
    /// Event names to fire for (ticket.created, ticket.updated, ticket.moved,
    /// ticket.deleted, ticket.restored, comment.created). Omit for all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
    /// Metadata attached to every invocation this binding receives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CommentConfig {
    /// Required: ticket uuid or human key. A binding without it never fires.
    pub ticket_id: String,
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
    Full(Ticket),
    Summary(TicketSummary),
}

impl From<Ticket> for TicketPayload {
    fn from(ticket: Ticket) -> Self {
        TicketPayload::Full(ticket)
    }
}

impl TicketPayload {
    pub fn summarized(&self) -> TicketPayload {
        match self {
            TicketPayload::Full(ticket) => TicketPayload::Summary(summarize(ticket)),
            TicketPayload::Summary(summary) => TicketPayload::Summary(summary.clone()),
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

#[derive(Clone, Default)]
pub struct Subscribers {
    pub change: Bindings,
    pub comment: Bindings,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct SubscriberCounts {
    pub change: usize,
    pub comment: usize,
}

impl Subscribers {
    pub fn counts(&self) -> SubscriberCounts {
        SubscriberCounts {
            change: self.change.read().unwrap_or_else(|p| p.into_inner()).len(),
            comment: self.comment.read().unwrap_or_else(|p| p.into_inner()).len(),
        }
    }
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

pub fn register_trigger_types(iii: &Arc<IIIClient>, subscribers: &Subscribers) {
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            CHANGE_TRIGGER,
            "Fires after every kanban mutation with the whole ticket. Config: { ticket_id?, events?, metadata? } — ticket_id matches the uuid or the human key.",
            BindingTable(subscribers.change.clone()),
        )
        .trigger_request_format::<ChangeConfig>()
        .call_request_format::<ChangeEvent>(),
    );
    let _ = iii.register_trigger_type(
        RegisterTriggerType::new(
            COMMENT_TRIGGER,
            "Fires when a comment is added to one ticket, optionally filtered by author. Config: { ticket_id, author?, exclude_author?, root_only? }.",
            BindingTable(subscribers.comment.clone()),
        )
        .trigger_request_format::<CommentConfig>()
        .call_request_format::<CommentEvent>(),
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

pub fn matches_change(config: &Value, event: &ChangeEvent) -> bool {
    if !matches_ticket(
        text(config, "ticket_id"),
        &event.ticket_id,
        &event.ticket_key,
    ) {
        return false;
    }
    let events: Vec<&str> = config
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
        .unwrap_or_default();
    events.is_empty() || events.contains(&event.event.as_str())
}

pub fn matches_comment(config: &Value, event: &CommentEvent) -> bool {
    let reference = text(config, "ticket_id");
    if reference.is_empty() || !matches_ticket(reference, &event.ticket_id, &event.ticket_key) {
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
        Ok((full, summary)) => deliver(iii.clone(), bindings, full, summary),
        Err(error) => tracing::error!(error = %error, "kanban change event failed to serialize"),
    }
}

pub fn emit_comment(iii: &Arc<IIIClient>, subscribers: &Subscribers, event: &CommentEvent) {
    let bindings = matching(&subscribers.comment, |binding| {
        matches_comment(&binding.config, event)
    });
    match payloads(event, &event.ticket) {
        Ok((full, summary)) => deliver(iii.clone(), bindings, full, summary),
        Err(error) => tracing::error!(error = %error, "kanban comment event failed to serialize"),
    }
}
