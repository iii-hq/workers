//! The two event functions this worker hosts.
//!
//! - `kanban::on-change` is called after every mutation with the whole ticket
//!   (a `TicketView`: parent, live children and progress included). Args
//!   `{ ticket_id?, parent_id?, release_id?, ticket?, events? }`: `ticket_id`
//!   matches the uuid or the human key; `parent_id` (uuid or key) selects
//!   that ticket and each of its subtasks; `release_id` (id or name) selects
//!   a ticket in that release and one a write just took out of it; `events`
//!   narrows by event name. An unknown reference matches nothing. A write
//!   that touches two tickets (attach, detach) calls it once per ticket.
//!   `config.updated` (`{ event, at }`, no ticket) follows a reload of the
//!   configuration (columns, statuses, priorities); only an observer without
//!   a ticket, parent or release arg gets it.
//! - `kanban::on-comment` is called once per comment created. Args
//!   `{ ticket_id?, parent_id?, author?, exclude_author?, root_only?, ticket? }`:
//!   a comment on that ticket (`ticket_id`) or on that ticket or any of its
//!   subtasks (`parent_id`); an observer with neither is rejected
//!   ([`TICKET_OR_PARENT_REQUIRED`]).
//!
//! Both are private (only this worker calls them) and their handler returns
//! the event. Their match hooks choose the observers whose args select the
//! event; their deliver hooks call each chosen observer whose filters match
//! the call through itself (`Void`), with the event, or with its summary-sized
//! form when its args say `ticket: summary`. The observer's metadata reaches
//! its target as the sidecar; this worker never sees it.

use iii_sdk::errors::Error;
use iii_sdk::{
    Call, CallAction, CallRequest, CallRequestWithMetadata, IIIClient, Match, ObserveRejection,
    Observer, RegisterFunction,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{ReleaseRef, TicketSummary, TicketView, summarize};
use crate::store::{Activity, Comment};

pub const ON_CHANGE_FN: &str = "kanban::on-change";
pub const ON_COMMENT_FN: &str = "kanban::on-comment";

/// `kanban::on-comment` rejects an observer that names neither a ticket nor a
/// parent.
pub const TICKET_OR_PARENT_REQUIRED: &str = "TICKET_OR_PARENT_REQUIRED";

/// The call metadata key that carries a change's releases to the match hook.
/// They select observers by `release_id` and are not part of the event.
const RELEASES_KEY: &str = "releases";

/// Every event name `kanban::on-change` can carry.
pub const CHANGE_EVENTS: [&str; 7] = [
    "ticket.created",
    "ticket.updated",
    "ticket.moved",
    "ticket.deleted",
    "ticket.restored",
    "comment.created",
    CONFIG_EVENT,
];

/// The one `kanban::on-change` event that carries no ticket: the
/// configuration was reloaded, so columns or statuses may have changed.
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

/// The args of a `kanban::on-change` observer.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangeArgs {
    /// Ticket uuid or human key. Omit for every ticket.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    /// Ticket uuid or human key of a parent: the parent itself and each of its subtasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Release id or name: a ticket in the release and a ticket a write just moved out of it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release_id: Option<String>,
    /// `full` (default) delivers the whole ticket; `summary` only its board-card fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketDetail>,
    /// Event names to deliver (ticket.created, ticket.updated, ticket.moved,
    /// ticket.deleted, ticket.restored, comment.created, config.updated). Omit for all.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<Vec<String>>,
}

/// The args of a `kanban::on-comment` observer.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommentArgs {
    /// Ticket uuid or human key. Give this or `parent_id`; an observer with neither is rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket_id: Option<String>,
    /// Ticket uuid or human key of a parent: comments on the parent or on any of its subtasks.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    /// Only comments by exactly this author.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    /// Every author except this one — pass your own profile id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_author: Option<String>,
    /// Ignore replies; only top-level comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_only: Option<bool>,
    /// `full` (default) delivers the whole ticket; `summary` only its board-card fields.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<TicketDetail>,
}

/// The `ticket` field of a delivery: the whole ticket, or its summary when the
/// observer asked for `ticket: summary`.
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
    /// `release_id` arg matches against. Not part of the event: the call
    /// carries them in its metadata ([`change_metadata`]).
    #[serde(skip)]
    pub releases: Vec<ReleaseRef>,
}

/// `config.updated` on `kanban::on-change`: the board's configuration
/// changed. It names no ticket.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ConfigEvent {
    /// Always `config.updated`.
    pub event: String,
    pub at: String,
}

/// What a `kanban::on-change` observer receives: a ticket event, or a
/// configuration event. Published as the function's request and response
/// format only.
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

/// How many observers each event function has.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema)]
pub struct ObserverCounts {
    pub change: usize,
    pub comment: usize,
}

pub fn observer_counts(iii: &IIIClient) -> ObserverCounts {
    ObserverCounts {
        change: iii.observers(ON_CHANGE_FN).len(),
        comment: iii.observers(ON_COMMENT_FN).len(),
    }
}

fn format<T: JsonSchema>() -> Value {
    serde_json::to_value(schemars::schema_for!(T)).unwrap_or(Value::Null)
}

/// Register `kanban::on-change` and `kanban::on-comment` with their hooks.
/// The hooks are set by id first, so they see every observer.
pub fn register(iii: &IIIClient) {
    iii.on_observer_added_for(ON_COMMENT_FN, |obs: Observer| async move {
        let args = obs.args_value();
        if text(args, "ticket_id").is_empty() && text(args, "parent_id").is_empty() {
            return Err(ObserveRejection::new(
                TICKET_OR_PARENT_REQUIRED,
                "kanban::on-comment needs ticket_id or parent_id",
            ));
        }
        Ok(())
    });
    iii.on_observer_match_for(
        ON_CHANGE_FN,
        Match::predicate(|obs, call| wants_change(obs.args_value(), call)),
    );
    iii.on_observer_match_for(
        ON_COMMENT_FN,
        Match::predicate(|obs, call| wants_comment(obs.args_value(), call)),
    );
    for function_id in [ON_CHANGE_FN, ON_COMMENT_FN] {
        let engine = iii.clone();
        iii.on_observer_deliver_for(function_id, move |call: Call, observers: Vec<Observer>| {
            let engine = engine.clone();
            async move { deliver(&engine, call, observers).await }
        });
    }

    iii.register_function(
        ON_CHANGE_FN,
        RegisterFunction::new(|event: Value| Ok::<_, Error>(event))
            .description(
                "Called after every kanban mutation with the whole ticket (parent, subtasks and progress included), and with { event: \"config.updated\", at } after a configuration reload. Observe it with args { ticket_id?, parent_id?, release_id?, ticket?, events? }: ticket_id and parent_id match the uuid or the human key (parent_id also selects each subtask); release_id is a release id or name (tickets in it and tickets moved out of it); only an observer without ticket_id, parent_id or release_id gets config.updated. Each observer's target receives the event, or with ticket: \"summary\" the event with its ticket cut to the board-card fields.",
            )
            .request_format(format::<ChangeDelivery>())
            .response_format(format::<ChangeDelivery>())
            .args_format::<ChangeArgs>()
            .private(),
    );
    iii.register_function(
        ON_COMMENT_FN,
        RegisterFunction::new(|event: Value| Ok::<_, Error>(event))
            .description(
                "Called once per comment created. Observe it with args { ticket_id? | parent_id?, author?, exclude_author?, root_only?, ticket? }: ticket_id is one ticket, parent_id a ticket and all its subtasks (uuid or human key); one of them is required (TICKET_OR_PARENT_REQUIRED). Each observer's target receives the event, or with ticket: \"summary\" the event with its ticket cut to the board-card fields.",
            )
            .request_format(format::<CommentEvent>())
            .response_format(format::<CommentEvent>())
            .args_format::<CommentArgs>()
            .private(),
    );
}

fn str_at<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    str_at(args, key).unwrap_or_default().trim()
}

/// An empty reference is no filter; otherwise the uuid or the key, case-insensitively.
pub fn matches_ticket(reference: &str, ticket_id: &str, ticket_key: &str) -> bool {
    let wanted = reference.trim();
    if wanted.is_empty() {
        return true;
    }
    wanted.eq_ignore_ascii_case(ticket_id) || wanted.eq_ignore_ascii_case(ticket_key)
}

/// The ticket an event is about, as observer args select it: its uuid, its
/// key, and its parent's uuid and (when the event carries it) key.
struct Subject<'a> {
    id: &'a str,
    key: &'a str,
    parent: Option<(&'a str, Option<&'a str>)>,
}

impl<'a> Subject<'a> {
    fn new(id: &'a str, key: &'a str, ticket: &'a TicketPayload) -> Self {
        Self {
            id,
            key,
            parent: ticket.parent(),
        }
    }

    /// As [`Subject::new`], read from an event as it is called: the parent of
    /// a whole ticket is `ticket.parent`, of a summary `ticket.parent_id`.
    fn of_input(event: &'a Value) -> Self {
        let ticket = &event["ticket"];
        let parent = match ticket.get("parent") {
            Some(parent) => str_at(parent, "id").map(|id| (id, str_at(parent, "key"))),
            None => str_at(ticket, "parent_id").map(|id| (id, None)),
        };
        Self {
            id: str_at(event, "ticket_id").unwrap_or_default(),
            key: str_at(event, "ticket_key").unwrap_or_default(),
            parent,
        }
    }

    fn is(&self, reference: &str) -> bool {
        matches_ticket(reference, self.id, self.key)
    }

    /// The ticket is `reference` itself or a subtask of it (uuid or key).
    fn in_family(&self, reference: &str) -> bool {
        self.is(reference)
            || self
                .parent
                .is_some_and(|(id, key)| matches_ticket(reference, id, key.unwrap_or_default()))
    }
}

/// An `events` arg narrows by event name; absent or empty, every event.
fn selects_event(args: &Value, event: &str) -> bool {
    let mut names = args
        .get("events")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .peekable();
    names.peek().is_none() || names.any(|name| name == event)
}

fn selects_change<'r>(
    args: &Value,
    event: &str,
    subject: &Subject<'_>,
    releases: impl IntoIterator<Item = (&'r str, &'r str)>,
) -> bool {
    if !subject.is(text(args, "ticket_id")) {
        return false;
    }
    let parent = text(args, "parent_id");
    if !parent.is_empty() && !subject.in_family(parent) {
        return false;
    }
    let release = text(args, "release_id");
    if !release.is_empty()
        && !releases.into_iter().any(|(id, name)| {
            release.eq_ignore_ascii_case(id) || release.eq_ignore_ascii_case(name.trim())
        })
    {
        return false;
    }
    selects_event(args, event)
}

/// Whether an observer with these args wants this ticket event.
pub fn matches_change(args: &Value, event: &ChangeEvent) -> bool {
    selects_change(
        args,
        &event.event,
        &Subject::new(&event.ticket_id, &event.ticket_key, &event.ticket),
        event
            .releases
            .iter()
            .map(|release| (release.id.as_str(), release.name.as_str())),
    )
}

/// `config.updated` names no ticket: an observer that selects one (or a
/// parent or release) never gets it; `events` still narrows.
pub fn matches_config(args: &Value) -> bool {
    if ["ticket_id", "parent_id", "release_id"]
        .iter()
        .any(|key| !text(args, key).is_empty())
    {
        return false;
    }
    selects_event(args, CONFIG_EVENT)
}

/// `kanban::on-change`'s match hook: whether an observer with these args
/// wants this call. It reads the event where it lies, with the releases from
/// the call's metadata, so choosing an observer copies nothing.
pub fn wants_change(args: &Value, call: &Call) -> bool {
    let event = str_at(&call.input, "event").unwrap_or_default();
    if event == CONFIG_EVENT {
        return matches_config(args);
    }
    let releases = call
        .metadata
        .as_ref()
        .and_then(|metadata| metadata.get(RELEASES_KEY))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|release| {
            (
                str_at(release, "id").unwrap_or_default(),
                str_at(release, "name").unwrap_or_default(),
            )
        });
    selects_change(args, event, &Subject::of_input(&call.input), releases)
}

fn selects_comment(args: &Value, subject: &Subject<'_>, author: &str, reply: bool) -> bool {
    let (reference, parent) = (text(args, "ticket_id"), text(args, "parent_id"));
    if reference.is_empty() && parent.is_empty() {
        return false;
    }
    if !subject.is(reference) {
        return false;
    }
    if !parent.is_empty() && !subject.in_family(parent) {
        return false;
    }
    let wanted = text(args, "author");
    if !wanted.is_empty() && author != wanted {
        return false;
    }
    let excluded = text(args, "exclude_author");
    if !excluded.is_empty() && author == excluded {
        return false;
    }
    !(reply && args.get("root_only").and_then(Value::as_bool) == Some(true))
}

/// Whether an observer with these args wants this comment.
pub fn matches_comment(args: &Value, event: &CommentEvent) -> bool {
    selects_comment(
        args,
        &Subject::new(&event.ticket_id, &event.ticket_key, &event.ticket),
        &event.author,
        event.comment.parent_id.is_some(),
    )
}

/// `kanban::on-comment`'s match hook: whether an observer with these args
/// wants this call, read where the event lies.
pub fn wants_comment(args: &Value, call: &Call) -> bool {
    let input = &call.input;
    let reply = input
        .get("comment")
        .and_then(|comment| comment.get("parent_id"))
        .is_some_and(|parent| !parent.is_null());
    selects_comment(
        args,
        &Subject::of_input(input),
        str_at(input, "author").unwrap_or_default(),
        reply,
    )
}

/// Whether an observer asked for the summary-sized payload.
pub fn wants_summary(args: &Value) -> bool {
    args.get("ticket").and_then(Value::as_str) == Some("summary")
}

/// The two wire shapes of one event: `full` as serialized, `summary` with the
/// `ticket` field replaced by its board-card projection.
pub fn payloads<T: Serialize>(event: &T, ticket: &TicketPayload) -> Result<(Value, Value), String> {
    let full = serde_json::to_value(event).map_err(|e| e.to_string())?;
    let summary = with_summary(full.clone(), ticket)?;
    Ok((full, summary))
}

fn with_summary(mut event: Value, ticket: &TicketPayload) -> Result<Value, String> {
    event["ticket"] = serde_json::to_value(ticket.summarized()).map_err(|e| e.to_string())?;
    Ok(event)
}

/// The summary-sized form of an event as it is called; `config.updated`
/// carries no ticket and stays as it is.
fn summary_of(event: &Value) -> Result<Value, Error> {
    let Some(ticket) = event.get("ticket") else {
        return Ok(event.clone());
    };
    let ticket: TicketPayload = serde_json::from_value(ticket.clone())
        .map_err(|e| Error::Handler(format!("kanban event ticket: {e}")))?;
    with_summary(event.clone(), &ticket).map_err(Error::Handler)
}

/// Call `obs`'s target with `payload` through the observer, fire-and-forget.
async fn call_through(iii: &IIIClient, obs: &Observer, payload: Value) {
    let request = CallRequest::observer(obs, payload).action(CallAction::Void);
    if let Err(error) = iii.call(request).await {
        tracing::warn!(
            function_id = obs.function_id(),
            observer_id = obs.id(),
            target = obs.target(),
            error = %error,
            "kanban event delivery failed"
        );
    }
}

/// The deliver hook of both functions: every matched observer whose filters
/// match the call gets the event through itself, the summary-sized form when
/// its args say `ticket: summary`. The caller gets the event back.
async fn deliver(iii: &IIIClient, call: Call, observers: Vec<Observer>) -> Result<Value, Error> {
    let chosen: Vec<&Observer> = observers.iter().filter(|obs| obs.matches(&call)).collect();
    let summary = if chosen.iter().any(|obs| wants_summary(obs.args_value())) {
        Some(summary_of(&call.input)?)
    } else {
        None
    };
    for obs in chosen {
        let payload = match &summary {
            Some(summary) if wants_summary(obs.args_value()) => summary.clone(),
            _ => call.input.clone(),
        };
        call_through(iii, obs, payload).await;
    }
    Ok(call.input)
}

/// The metadata a `kanban::on-change` call carries: the releases the match
/// hook selects `release_id` observers by, when the change names any.
pub fn change_metadata(event: &ChangeEvent) -> Option<Value> {
    (!event.releases.is_empty()).then(|| json!({ RELEASES_KEY: event.releases }))
}

/// Call `function_id` with the event, fire-and-forget: the call runs on its
/// own task, so the mutation that produced it is not held by its observers,
/// and it names the writing handler as its caller.
async fn emit(iii: &IIIClient, function_id: &'static str, payload: Value, metadata: Option<Value>) {
    let mut request = CallRequestWithMetadata::from(CallRequest::new(function_id, payload))
        .action(CallAction::Void);
    if let Some(metadata) = metadata {
        request = request.metadata(metadata);
    }
    if let Err(error) = iii.call(request).await {
        tracing::warn!(function_id, error = %error, "kanban event call failed");
    }
}

/// Call `kanban::on-change` with a ticket event, when anything observes it.
pub async fn emit_change(iii: &IIIClient, event: &ChangeEvent) {
    if !iii.is_observed(ON_CHANGE_FN) {
        return;
    }
    match serde_json::to_value(event) {
        Ok(payload) => emit(iii, ON_CHANGE_FN, payload, change_metadata(event)).await,
        Err(error) => tracing::error!(error = %error, "kanban change event failed to serialize"),
    }
}

/// Tell the `kanban::on-change` observers that select no ticket that the
/// configuration changed.
pub async fn emit_config_updated(iii: &IIIClient) {
    if !iii.is_observed(ON_CHANGE_FN) {
        return;
    }
    let event = ConfigEvent {
        event: CONFIG_EVENT.to_string(),
        at: crate::board::now(),
    };
    match serde_json::to_value(&event) {
        Ok(payload) => emit(iii, ON_CHANGE_FN, payload, None).await,
        Err(error) => tracing::error!(error = %error, "kanban config event failed to serialize"),
    }
}

/// Call `kanban::on-comment` with a comment event, when anything observes it.
pub async fn emit_comment(iii: &IIIClient, event: &CommentEvent) {
    if !iii.is_observed(ON_COMMENT_FN) {
        return;
    }
    match serde_json::to_value(event) {
        Ok(payload) => emit(iii, ON_COMMENT_FN, payload, None).await,
        Err(error) => tracing::error!(error = %error, "kanban comment event failed to serialize"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call_of(input: Value, metadata: Option<Value>) -> Call {
        Call {
            input,
            metadata,
            ..Call::default()
        }
    }

    #[test]
    fn change_format_admits_the_config_event() {
        let schema = serde_json::to_value(schemars::schema_for!(ChangeDelivery)).unwrap();
        let variants = schema["anyOf"]
            .as_array()
            .expect("one schema per delivery shape");
        assert_eq!(variants.len(), 2);
    }

    #[test]
    fn config_event_reaches_unfiltered_observers_only() {
        assert!(matches_config(&json!({})));
        assert!(matches_config(
            &json!({ "events": ["config.updated", "ticket.moved"] })
        ));
        assert!(!matches_config(&json!({ "events": ["ticket.moved"] })));
        assert!(!matches_config(&json!({ "ticket_id": "KAN-1" })));
        assert!(!matches_config(&json!({ "parent_id": "KAN-1" })));
        assert!(!matches_config(&json!({ "release_id": "r1" })));
        assert!(matches_config(&json!({ "ticket_id": "  " })));

        let config = call_of(json!({ "event": CONFIG_EVENT, "at": "now" }), None);
        assert!(wants_change(&json!({}), &config));
        assert!(!wants_change(&json!({ "ticket_id": "KAN-1" }), &config));
    }

    #[test]
    fn the_match_hook_reads_parents_of_whole_tickets_and_summaries() {
        let whole = call_of(
            json!({
                "event": "ticket.updated",
                "ticket_id": "u-2",
                "ticket_key": "KAN-2",
                "ticket": { "parent": { "id": "u-1", "key": "KAN-1" } },
            }),
            None,
        );
        assert!(wants_change(&json!({ "parent_id": "kan-1" }), &whole));
        assert!(wants_change(&json!({ "parent_id": "u-1" }), &whole));
        assert!(!wants_change(&json!({ "parent_id": "KAN-3" }), &whole));

        let summary = call_of(
            json!({
                "event": "ticket.updated",
                "ticket_id": "u-2",
                "ticket_key": "KAN-2",
                "ticket": { "parent_id": "u-1" },
            }),
            None,
        );
        assert!(wants_change(&json!({ "parent_id": "u-1" }), &summary));
        assert!(
            !wants_change(&json!({ "parent_id": "KAN-1" }), &summary),
            "a summary carries no parent key"
        );
    }

    #[test]
    fn the_match_hook_takes_releases_from_the_call_metadata() {
        let input = json!({ "event": "ticket.moved", "ticket_id": "u-1", "ticket_key": "KAN-1", "ticket": {} });
        let moved = call_of(
            input.clone(),
            Some(json!({ "releases": [{ "id": "r-2", "name": "Phase 2" }] })),
        );
        assert!(wants_change(&json!({ "release_id": "phase 2" }), &moved));
        assert!(wants_change(&json!({ "release_id": "r-2" }), &moved));
        assert!(!wants_change(&json!({ "release_id": "Phase 3" }), &moved));
        assert!(!wants_change(
            &json!({ "release_id": "Phase 2" }),
            &call_of(input, None)
        ));
    }

    #[test]
    fn the_comment_match_hook_reads_author_and_thread() {
        let reply = call_of(
            json!({
                "event": "comment.created",
                "ticket_id": "u-1",
                "ticket_key": "KAN-1",
                "ticket": {},
                "comment": { "parent_id": "c-0" },
                "author": "tech-lead",
            }),
            None,
        );
        assert!(wants_comment(&json!({ "ticket_id": "KAN-1" }), &reply));
        assert!(!wants_comment(&json!({}), &reply));
        assert!(!wants_comment(
            &json!({ "ticket_id": "KAN-1", "root_only": true }),
            &reply
        ));
        assert!(!wants_comment(
            &json!({ "ticket_id": "KAN-1", "exclude_author": "tech-lead" }),
            &reply
        ));
        assert!(wants_comment(
            &json!({ "ticket_id": "KAN-1", "author": "tech-lead" }),
            &reply
        ));
    }

    #[test]
    fn a_config_event_has_no_ticket_to_summarize() {
        let event = json!({ "event": CONFIG_EVENT, "at": "now" });
        assert_eq!(summary_of(&event).unwrap(), event);
    }
}
