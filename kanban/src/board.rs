//! Pure board operations: everything a `kanban::*` function does to the
//! in-memory [`Board`] before the store writes it back. No I/O, no engine —
//! this is the module the tests exercise directly.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Column, KanbanConfig};
use crate::store::{Activity, Board, Change, Comment, Ticket, default_actor};

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The slice of configuration a mutation needs.
#[derive(Debug, Clone)]
pub struct BoardContext {
    pub prefix: String,
    pub default_status: String,
    pub columns: Vec<Column>,
    pub priorities: Vec<String>,
}

impl From<&KanbanConfig> for BoardContext {
    fn from(config: &KanbanConfig) -> Self {
        Self {
            prefix: config.id_prefix.clone(),
            default_status: config.default_status.clone(),
            columns: config.columns.clone(),
            priorities: config.priorities.clone(),
        }
    }
}

impl BoardContext {
    fn has_column(&self, id: &str) -> bool {
        self.columns.iter().any(|column| column.id == id)
    }
}

/// The board-card projection of a ticket.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TicketSummary {
    pub id: String,
    pub key: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub labels: Vec<String>,
    pub order: u64,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub comment_count: u64,
}

pub fn summarize(ticket: &Ticket) -> TicketSummary {
    TicketSummary {
        id: ticket.id.clone(),
        key: ticket.key.clone(),
        title: ticket.title.clone(),
        status: ticket.status.clone(),
        priority: ticket.priority.clone(),
        assignee: ticket.assignee.clone(),
        labels: ticket.labels.clone(),
        order: ticket.order,
        created_at: ticket.created_at.clone(),
        updated_at: ticket.updated_at.clone(),
        deleted_at: ticket.deleted_at.clone(),
        comment_count: ticket.comments.len() as u64,
    }
}

/// The compact projection: what an agent needs to recognise a ticket in a
/// list (`compact: true` on `kanban::board::get` / `kanban::ticket::list`).
/// About a third of a summary's size — no uuid, timestamps, labels or order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct TicketBrief {
    pub key: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub assignee: Option<String>,
    pub comment_count: u64,
}

/// One list row: the summary by default, the brief when `compact` was asked.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum TicketRow {
    Summary(TicketSummary),
    Brief(TicketBrief),
}

pub fn row(ticket: &Ticket, compact: bool) -> TicketRow {
    if compact {
        TicketRow::Brief(TicketBrief {
            key: ticket.key.clone(),
            title: ticket.title.clone(),
            status: ticket.status.clone(),
            priority: ticket.priority.clone(),
            assignee: ticket.assignee.clone(),
            comment_count: ticket.comments.len() as u64,
        })
    } else {
        TicketRow::Summary(summarize(ticket))
    }
}

fn trimmed(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn actor_or_user(actor: Option<&str>) -> String {
    trimmed(actor).unwrap_or_else(default_actor)
}

fn clean_labels(labels: &[String]) -> Vec<String> {
    labels
        .iter()
        .filter_map(|label| trimmed(Some(label)))
        .collect()
}

/// Resolve the internal uuid or the human key (case-insensitive) to the
/// ticket's id in the map.
fn resolve_id(board: &Board, id_or_key: &str) -> Option<String> {
    let wanted = id_or_key.trim();
    if wanted.is_empty() {
        return None;
    }
    if board.tickets.contains_key(wanted) {
        return Some(wanted.to_string());
    }
    let folded = wanted.to_lowercase();
    board
        .tickets
        .values()
        .find(|ticket| ticket.key.to_lowercase() == folded)
        .map(|ticket| ticket.id.clone())
}

pub fn find_ticket<'a>(board: &'a Board, id_or_key: &str) -> Option<&'a Ticket> {
    resolve_id(board, id_or_key).and_then(|id| board.tickets.get(&id))
}

fn find_ticket_mut<'a>(board: &'a mut Board, id_or_key: &str) -> Option<&'a mut Ticket> {
    let id = resolve_id(board, id_or_key)?;
    board.tickets.get_mut(&id)
}

fn by_order(a: &Ticket, b: &Ticket) -> std::cmp::Ordering {
    a.order
        .cmp(&b.order)
        .then_with(|| a.created_at.cmp(&b.created_at))
}

#[derive(Debug, Clone, Default)]
pub struct TicketFilter {
    /// Column id.
    pub status: Option<String>,
    /// Agent profile id; `Some("")` matches unassigned tickets.
    pub assignee: Option<String>,
    pub include_deleted: bool,
    /// Only tickets whose `updated_at` is after this ISO timestamp.
    pub updated_since: Option<String>,
    pub limit: Option<usize>,
}

pub fn list_tickets<'a>(board: &'a Board, filter: &TicketFilter) -> Vec<&'a Ticket> {
    let status = filter.status.as_deref().map(str::trim).unwrap_or_default();
    let assignee = filter.assignee.as_deref().map(str::trim);
    let since = trimmed(filter.updated_since.as_deref());
    let mut rows: Vec<&Ticket> = board
        .tickets
        .values()
        .filter(|ticket| filter.include_deleted || ticket.deleted_at.is_none())
        .filter(|ticket| status.is_empty() || ticket.status == status)
        .filter(|ticket| match assignee {
            None => true,
            Some(wanted) => ticket.assignee.as_deref().unwrap_or_default() == wanted,
        })
        .filter(|ticket| since.as_ref().is_none_or(|s| &ticket.updated_at > s))
        .collect();
    rows.sort_by(|a, b| by_order(a, b));
    if let Some(limit) = filter.limit.filter(|limit| *limit > 0) {
        rows.truncate(limit);
    }
    rows
}

/// Next free position at the bottom of a column, ignoring `except`.
fn next_order(board: &Board, status: &str, except: Option<&str>) -> u64 {
    board
        .tickets
        .values()
        .filter(|ticket| {
            ticket.status == status
                && ticket.deleted_at.is_none()
                && except.is_none_or(|id| ticket.id != id)
        })
        .map(|ticket| ticket.order)
        .max()
        .unwrap_or(0)
        + 1
}

fn next_key(board: &mut Board, prefix: &str) -> String {
    let mut number = board.next_number.max(1);
    let mut key = format!("{prefix}-{number}");
    while board.tickets.values().any(|ticket| ticket.key == key) {
        number += 1;
        key = format!("{prefix}-{number}");
    }
    board.next_number = number + 1;
    key
}

fn record(
    ticket: &mut Ticket,
    kind: &str,
    actor: &str,
    at: &str,
    changes: Vec<Change>,
    comment_id: Option<String>,
) {
    ticket.activity.push(Activity {
        id: new_id(),
        ticket_id: ticket.id.clone(),
        seq: ticket.activity.len() as u64 + 1,
        kind: kind.to_string(),
        actor: actor.to_string(),
        at: at.to_string(),
        changes: if changes.is_empty() {
            None
        } else {
            Some(changes)
        },
        comment_id,
    });
}

/// Renumber a column 1..n in display order.
fn compact_column(board: &mut Board, status: &str) {
    let mut ids: Vec<(String, u64, String)> = board
        .tickets
        .values()
        .filter(|ticket| ticket.status == status && ticket.deleted_at.is_none())
        .map(|ticket| (ticket.id.clone(), ticket.order, ticket.created_at.clone()))
        .collect();
    ids.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.2.cmp(&b.2)));
    for (index, (id, _, _)) in ids.into_iter().enumerate() {
        if let Some(ticket) = board.tickets.get_mut(&id) {
            ticket.order = index as u64 + 1;
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct CreateTicketInput {
    pub title: String,
    pub description: Option<String>,
    pub status: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<String>,
    pub labels: Option<Vec<String>>,
    pub actor: Option<String>,
}

pub fn create_ticket(
    board: &mut Board,
    input: &CreateTicketInput,
    context: &BoardContext,
    at: &str,
) -> Result<Ticket, String> {
    let title = trimmed(Some(&input.title)).ok_or("title is required")?;
    let requested = trimmed(input.status.as_deref());
    if let Some(status) = &requested
        && !context.has_column(status)
    {
        return Err(format!("unknown status: {status}"));
    }
    let status = requested.unwrap_or_else(|| context.default_status.clone());
    let actor = actor_or_user(input.actor.as_deref());
    let priority = trimmed(input.priority.as_deref())
        .or_else(|| context.priorities.first().cloned())
        .unwrap_or_else(|| "medium".to_string());
    let key = next_key(board, &context.prefix);
    let mut ticket = Ticket {
        id: new_id(),
        key,
        title,
        description: input.description.clone().unwrap_or_default(),
        status: status.clone(),
        priority,
        assignee: trimmed(input.assignee.as_deref()),
        labels: input
            .labels
            .as_deref()
            .map(clean_labels)
            .unwrap_or_default(),
        order: next_order(board, &status, None),
        created_at: at.to_string(),
        updated_at: at.to_string(),
        deleted_at: None,
        created_by: actor.clone(),
        comments: Vec::new(),
        activity: Vec::new(),
    };
    record(
        &mut ticket,
        "ticket.created",
        &actor,
        at,
        vec![Change {
            field: "status".into(),
            from: Value::Null,
            to: json!(status),
        }],
        None,
    );
    board.tickets.insert(ticket.id.clone(), ticket.clone());
    Ok(ticket)
}

/// Fields `kanban::ticket::update` accepts. `None` = leave alone;
/// `assignee: Some(None)` = unassign.
#[derive(Debug, Clone, Default)]
pub struct UpdatePatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<Option<String>>,
    pub status: Option<String>,
    pub labels: Option<Vec<String>>,
}

#[derive(Debug)]
pub struct UpdateResult {
    pub ticket: Ticket,
    pub changes: Vec<Change>,
}

pub fn update_ticket(
    board: &mut Board,
    id_or_key: &str,
    patch: &UpdatePatch,
    actor: &str,
    context: &BoardContext,
    at: &str,
) -> Result<Option<UpdateResult>, String> {
    let Some(id) = resolve_id(board, id_or_key) else {
        return Ok(None);
    };
    let mut changes: Vec<Change> = Vec::new();
    {
        let ticket = board.tickets.get_mut(&id).expect("resolved id exists");
        let mut apply = |field: &str, before: Value, next: Value| {
            if before == next {
                return;
            }
            changes.push(Change {
                field: field.to_string(),
                from: before,
                to: next,
            });
        };
        if let Some(title) = &patch.title {
            let title = title.trim().to_string();
            if title.is_empty() {
                return Err("title cannot be empty".to_string());
            }
            apply("title", json!(ticket.title), json!(title));
            ticket.title = title;
        }
        if let Some(description) = &patch.description {
            let description = description.trim().to_string();
            apply("description", json!(ticket.description), json!(description));
            ticket.description = description;
        }
        if let Some(priority) = &patch.priority {
            let priority = priority.trim().to_string();
            apply("priority", json!(ticket.priority), json!(priority));
            ticket.priority = priority;
        }
        if let Some(assignee) = &patch.assignee {
            let assignee = assignee.as_deref().and_then(|value| trimmed(Some(value)));
            apply("assignee", json!(ticket.assignee), json!(assignee));
            ticket.assignee = assignee;
        }
        if let Some(status) = &patch.status {
            let status = status.trim().to_string();
            if !context.has_column(&status) {
                return Err(format!("unknown status: {status}"));
            }
            apply("status", json!(ticket.status), json!(status));
            ticket.status = status;
        }
        if let Some(labels) = &patch.labels {
            let labels = clean_labels(labels);
            apply("labels", json!(ticket.labels), json!(labels));
            ticket.labels = labels;
        }
    }
    if changes.is_empty() {
        let ticket = board.tickets[&id].clone();
        return Ok(Some(UpdateResult { ticket, changes }));
    }
    if changes.iter().any(|change| change.field == "status") {
        let status = board.tickets[&id].status.clone();
        let order = next_order(board, &status, Some(&id));
        board
            .tickets
            .get_mut(&id)
            .expect("resolved id exists")
            .order = order;
    }
    let ticket = board.tickets.get_mut(&id).expect("resolved id exists");
    ticket.updated_at = at.to_string();
    record(ticket, "ticket.updated", actor, at, changes.clone(), None);
    Ok(Some(UpdateResult {
        ticket: ticket.clone(),
        changes,
    }))
}

/// Move a ticket to a column at an optional zero-based position. Reordering
/// inside a lane records no activity; a lane change records `ticket.moved`.
pub fn move_ticket(
    board: &mut Board,
    id_or_key: &str,
    status: &str,
    index: Option<usize>,
    actor: Option<&str>,
    context: &BoardContext,
    at: &str,
) -> Result<Option<Ticket>, String> {
    let status = status.trim();
    if status.is_empty() || !context.has_column(status) {
        return Err(format!("unknown status: {status}"));
    }
    let Some(id) = resolve_id(board, id_or_key) else {
        return Ok(None);
    };
    let actor = actor_or_user(actor);
    let previous = board.tickets[&id].status.clone();

    let mut target: Vec<&Ticket> = board
        .tickets
        .values()
        .filter(|row| row.status == status && row.deleted_at.is_none() && row.id != id)
        .collect();
    target.sort_by(|a, b| by_order(a, b));
    let position = index.unwrap_or(target.len()).min(target.len());
    let mut ordered: Vec<String> = target.iter().map(|row| row.id.clone()).collect();
    ordered.insert(position, id.clone());
    for (offset, row_id) in ordered.into_iter().enumerate() {
        if let Some(row) = board.tickets.get_mut(&row_id) {
            row.order = offset as u64 + 1;
        }
    }
    {
        let ticket = board.tickets.get_mut(&id).expect("resolved id exists");
        ticket.status = status.to_string();
        ticket.updated_at = at.to_string();
    }
    if previous != status {
        compact_column(board, &previous);
        let ticket = board.tickets.get_mut(&id).expect("resolved id exists");
        record(
            ticket,
            "ticket.moved",
            &actor,
            at,
            vec![Change {
                field: "status".into(),
                from: json!(previous),
                to: json!(status),
            }],
            None,
        );
    }
    Ok(Some(board.tickets[&id].clone()))
}

pub fn delete_ticket(board: &mut Board, id_or_key: &str, actor: &str, at: &str) -> Option<Ticket> {
    let ticket = find_ticket_mut(board, id_or_key)?;
    if ticket.deleted_at.is_none() {
        ticket.deleted_at = Some(at.to_string());
        ticket.updated_at = at.to_string();
        record(ticket, "ticket.deleted", actor, at, Vec::new(), None);
    }
    Some(ticket.clone())
}

pub fn restore_ticket(board: &mut Board, id_or_key: &str, actor: &str, at: &str) -> Option<Ticket> {
    let id = resolve_id(board, id_or_key)?;
    if board.tickets[&id].deleted_at.is_none() {
        return Some(board.tickets[&id].clone());
    }
    let status = board.tickets[&id].status.clone();
    let order = next_order(board, &status, Some(&id));
    let ticket = board.tickets.get_mut(&id).expect("resolved id exists");
    ticket.deleted_at = None;
    ticket.updated_at = at.to_string();
    ticket.order = order;
    record(ticket, "ticket.restored", actor, at, Vec::new(), None);
    Some(ticket.clone())
}

#[derive(Debug, Clone, Default)]
pub struct CreateCommentInput {
    pub ticket_id: String,
    pub body: String,
    pub parent_id: Option<String>,
    pub author: Option<String>,
}

#[derive(Debug)]
pub struct CommentResult {
    pub comment: Comment,
    pub ticket: Ticket,
}

pub fn create_comment(
    board: &mut Board,
    input: &CreateCommentInput,
    at: &str,
) -> Result<Option<CommentResult>, String> {
    let Some(ticket) = find_ticket_mut(board, &input.ticket_id) else {
        return Ok(None);
    };
    if ticket.deleted_at.is_some() {
        return Err("cannot comment on a deleted ticket".to_string());
    }
    let body = trimmed(Some(&input.body)).ok_or("comment body is required")?;
    let parent_id = match trimmed(input.parent_id.as_deref()) {
        None => None,
        Some(wanted) => {
            let parent = ticket
                .comments
                .iter()
                .find(|comment| comment.id == wanted)
                .ok_or_else(|| format!("unknown comment: {wanted}"))?;
            Some(
                parent
                    .parent_id
                    .clone()
                    .unwrap_or_else(|| parent.id.clone()),
            )
        }
    };
    let author = actor_or_user(input.author.as_deref());
    let comment = Comment {
        id: new_id(),
        ticket_id: ticket.id.clone(),
        parent_id,
        body,
        author: author.clone(),
        created_at: at.to_string(),
    };
    ticket.comments.push(comment.clone());
    ticket.updated_at = at.to_string();
    record(
        ticket,
        "comment.created",
        &author,
        at,
        Vec::new(),
        Some(comment.id.clone()),
    );
    Ok(Some(CommentResult {
        comment,
        ticket: ticket.clone(),
    }))
}

#[derive(Debug, Clone, Default)]
pub struct CommentFilter {
    pub ticket_id: String,
    pub author: Option<String>,
    pub exclude_author: Option<String>,
    pub parent_id: Option<String>,
    pub root_only: bool,
    pub since: Option<String>,
    pub include_deleted: bool,
    pub limit: Option<usize>,
}

pub fn list_comments(board: &Board, filter: &CommentFilter) -> Vec<Comment> {
    let Some(ticket) = find_ticket(board, &filter.ticket_id) else {
        return Vec::new();
    };
    if ticket.deleted_at.is_some() && !filter.include_deleted {
        return Vec::new();
    }
    let author = trimmed(filter.author.as_deref());
    let exclude = trimmed(filter.exclude_author.as_deref());
    let parent = trimmed(filter.parent_id.as_deref());
    let since = trimmed(filter.since.as_deref());
    let mut rows: Vec<Comment> = ticket
        .comments
        .iter()
        .filter(|comment| author.as_ref().is_none_or(|a| &comment.author == a))
        .filter(|comment| exclude.as_ref().is_none_or(|a| &comment.author != a))
        .filter(|comment| {
            parent
                .as_ref()
                .is_none_or(|p| comment.parent_id.as_ref() == Some(p))
        })
        .filter(|comment| !filter.root_only || comment.parent_id.is_none())
        .filter(|comment| since.as_ref().is_none_or(|s| &comment.created_at > s))
        .cloned()
        .collect();
    rows.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    if let Some(limit) = filter.limit.filter(|limit| *limit > 0) {
        rows.truncate(limit);
    }
    rows
}

#[derive(Debug, Clone, Default)]
pub struct ActivityFilter {
    pub ticket_id: String,
    pub types: Option<Vec<String>>,
    pub since: Option<String>,
    pub limit: Option<usize>,
}

pub fn list_activity(board: &Board, filter: &ActivityFilter) -> Vec<Activity> {
    let Some(ticket) = find_ticket(board, &filter.ticket_id) else {
        return Vec::new();
    };
    let types = filter.types.clone().unwrap_or_default();
    let since = trimmed(filter.since.as_deref());
    let mut rows: Vec<Activity> = ticket
        .activity
        .iter()
        .filter(|entry| types.is_empty() || types.contains(&entry.kind))
        .filter(|entry| since.as_ref().is_none_or(|s| &entry.at > s))
        .cloned()
        .collect();
    rows.sort_by_key(|entry| entry.seq);
    if let Some(limit) = filter.limit.filter(|limit| *limit > 0) {
        rows.truncate(limit);
    }
    rows
}
