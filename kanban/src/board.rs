//! Pure board operations: everything a `kanban::*` function does to the
//! in-memory [`Board`] before the store writes it back. No I/O, no engine —
//! this is the module the tests exercise directly.

use std::collections::HashSet;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::config::{Column, KanbanConfig};
use crate::store::{Activity, Board, Change, Comment, Ticket, default_actor};

pub fn now() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// The timestamp of the next write, taken under the store lock: the clock
/// (`now_ms`, Unix milliseconds), or 1 ms past the newest `updated_at` on the
/// board when the clock has not moved past it (same millisecond, or stepped
/// back). Every write to a ticket therefore carries a strictly greater
/// `updated_at`, which the console uses as the ticket's version.
pub fn next_at(board: &Board, now_ms: i64) -> String {
    let newest = board
        .tickets
        .values()
        .filter_map(|ticket| chrono::DateTime::parse_from_rfc3339(&ticket.updated_at).ok())
        .map(|updated| updated.timestamp_millis())
        .max();
    let ms = newest.map_or(now_ms, |newest| now_ms.max(newest + 1));
    chrono::DateTime::from_timestamp_millis(ms)
        .expect("a parsed RFC 3339 year plus 1 ms is in range")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
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

pub fn brief(ticket: &Ticket) -> TicketBrief {
    TicketBrief {
        key: ticket.key.clone(),
        title: ticket.title.clone(),
        status: ticket.status.clone(),
        priority: ticket.priority.clone(),
        assignee: ticket.assignee.clone(),
        comment_count: ticket.comments.len() as u64,
    }
}

pub fn row(ticket: &Ticket, compact: bool) -> TicketRow {
    if compact {
        TicketRow::Brief(brief(ticket))
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

/// Trimmed, non-empty labels; a repeat in any ASCII case is dropped, the first spelling stays.
fn clean_labels(labels: &[String]) -> Vec<String> {
    let mut clean: Vec<String> = Vec::new();
    for label in labels.iter().filter_map(|label| trimmed(Some(label))) {
        // ponytail: linear scan; a ticket carries a handful of labels.
        if !clean.iter().any(|seen| seen.eq_ignore_ascii_case(&label)) {
            clean.push(label);
        }
    }
    clean
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

/// How well a ticket matches a search query; lower is better, `None` is no
/// match. Keys win (`KAN-12`, `kan-1`, `12`, `#12`), then the title (whole
/// prefix, every query word starting a title word, substring), then the
/// description and labels.
fn search_tier(ticket: &Ticket, query: &str) -> Option<u8> {
    let key = ticket.key.to_lowercase();
    let number = key.rsplit_once('-').map(|(_, number)| number).unwrap_or("");
    let digits = query.strip_prefix('#').unwrap_or(query);
    let is_number = !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit());
    if key == query || (is_number && number == digits) {
        return Some(0);
    }
    if key.starts_with(query) || (is_number && number.starts_with(digits)) {
        return Some(1);
    }
    let title = ticket.title.to_lowercase();
    if title.starts_with(query) {
        return Some(2);
    }
    let words: Vec<&str> = title
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();
    let mut terms = query.split_whitespace().peekable();
    if terms.peek().is_some() && terms.all(|term| words.iter().any(|word| word.starts_with(term))) {
        return Some(3);
    }
    if title.contains(query) {
        return Some(4);
    }
    let in_labels = ticket
        .labels
        .iter()
        .any(|label| label.to_lowercase().contains(query));
    if in_labels || ticket.description.to_lowercase().contains(query) {
        return Some(5);
    }
    None
}

/// Live tickets matching `query` (case-insensitive), best first, the most
/// recently updated first within a tier. An empty query lists the most
/// recently updated tickets.
pub fn search_tickets<'a>(board: &'a Board, query: &str, limit: usize) -> Vec<&'a Ticket> {
    let query = query.trim().to_lowercase();
    let mut hits: Vec<(u8, &Ticket)> = board
        .tickets
        .values()
        .filter(|ticket| ticket.deleted_at.is_none())
        .filter_map(|ticket| {
            if query.is_empty() {
                Some((0, ticket))
            } else {
                search_tier(ticket, &query).map(|tier| (tier, ticket))
            }
        })
        .collect();
    hits.sort_by(|(a_tier, a), (b_tier, b)| {
        a_tier
            .cmp(b_tier)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.key.cmp(&b.key))
    });
    hits.truncate(limit);
    hits.into_iter().map(|(_, ticket)| ticket).collect()
}

/// Combining marks are part of the word they modify: `c` + U+0327 blocks a key like `ç` does.
// ponytail: the five main combining blocks; marks of other scripts rely on `is_alphanumeric` (Other_Alphabetic), a full \p{M} table would need a crate.
fn is_combining_mark(c: char) -> bool {
    matches!(
        c,
        '\u{300}'..='\u{36F}'
            | '\u{1AB0}'..='\u{1AFF}'
            | '\u{1DC0}'..='\u{1DFF}'
            | '\u{20D0}'..='\u{20FF}'
            | '\u{FE20}'..='\u{FE2F}'
    )
}

/// Lowercased `<prefix>-` heads of every key on the board plus the configured
/// prefix: a ticket keeps the prefix it was numbered with when `id_prefix`
/// changes, so the configured one alone loses the links to older tickets.
fn key_heads(board: &Board, configured: &str) -> HashSet<String> {
    let configured = format!("{}-", configured.to_ascii_lowercase());
    board
        .tickets
        .values()
        .filter_map(|ticket| ticket.key.rsplit_once('-'))
        .map(|(prefix, _)| format!("{}-", prefix.to_ascii_lowercase()))
        .chain(std::iter::once(configured))
        .collect()
}

/// Lowercased keys (`kan-12`) written as `<head><digits>` (`heads` from
/// [`key_heads`]) in the ticket's title, description or comments:
/// case-insensitive. Same boundary as the UI's link rewriter: the characters
/// around a key must not be a Unicode letter, digit or combining mark, `/` or
/// `-`, so `KAN-12` is not found inside `KAN-120`, `KAN-12x`, `XKAN-12`,
/// `fix/KAN-12-login`, `KAN-12-2` or a `.../browse/KAN-12` URL.
fn cited_keys(ticket: &Ticket, heads: &HashSet<String>) -> HashSet<String> {
    let is_word = |c: char| c.is_alphanumeric() || is_combining_mark(c) || c == '/' || c == '-';
    let mut keys = HashSet::new();
    let texts = [&ticket.title, &ticket.description]
        .into_iter()
        .chain(ticket.comments.iter().map(|comment| &comment.body));
    for text in texts {
        // ASCII-only: keeps byte offsets and leaves accented letters intact (the prefix is ASCII).
        let text = text.to_ascii_lowercase();
        for head in heads {
            for (at, _) in text.match_indices(head.as_str()) {
                let rest = &text[at + head.len()..];
                let digits = rest
                    .find(|c: char| !c.is_ascii_digit())
                    .unwrap_or(rest.len());
                if digits > 0
                    && !text[..at].chars().next_back().is_some_and(is_word)
                    && !rest[digits..].chars().next().is_some_and(is_word)
                {
                    keys.insert(format!("{head}{}", &rest[..digits]));
                }
            }
        }
    }
    keys
}

fn key_number(ticket: &Ticket) -> u64 {
    ticket
        .key
        .rsplit_once('-')
        .and_then(|(_, number)| number.parse().ok())
        .unwrap_or(u64::MAX)
}

/// The live tickets the ticket cites by key and the live tickets that cite
/// it, each by ticket number; `None` when the ticket does not exist.
pub fn ticket_links<'a>(
    board: &'a Board,
    id_or_key: &str,
    prefix: &str,
) -> Option<(Vec<&'a Ticket>, Vec<&'a Ticket>)> {
    let ticket = find_ticket(board, id_or_key)?;
    let own_key = ticket.key.to_lowercase();
    let heads = key_heads(board, prefix);
    let cites = cited_keys(ticket, &heads);
    let (mut mentions, mut mentioned_by) = (Vec::new(), Vec::new());
    // ponytail: O(tickets x text x prefixes) per call; keep a key -> citing-tickets index updated on write if the board outgrows a scan.
    for other in board
        .tickets
        .values()
        .filter(|other| other.deleted_at.is_none() && other.id != ticket.id)
    {
        if cites.contains(&other.key.to_lowercase()) {
            mentions.push(other);
        }
        if cited_keys(other, &heads).contains(&own_key) {
            mentioned_by.push(other);
        }
    }
    mentions.sort_by_key(|ticket| key_number(ticket));
    mentioned_by.sort_by_key(|ticket| key_number(ticket));
    Some((mentions, mentioned_by))
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
/// `assignee: Some(None)` = unassign. `labels` replaces the set; then
/// `remove_labels` and `add_labels` edit it (in that order) on the labels the
/// ticket has when the write runs, so concurrent label edits do not overwrite
/// each other.
#[derive(Debug, Clone, Default)]
pub struct UpdatePatch {
    pub title: Option<String>,
    pub description: Option<String>,
    pub priority: Option<String>,
    pub assignee: Option<Option<String>>,
    pub status: Option<String>,
    pub labels: Option<Vec<String>>,
    pub add_labels: Option<Vec<String>>,
    pub remove_labels: Option<Vec<String>>,
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
            if ticket.deleted_at.is_some() && ticket.status != status {
                return Err("cannot move a deleted ticket".to_string());
            }
            apply("status", json!(ticket.status), json!(status));
            ticket.status = status;
        }
        if patch.labels.is_some() || patch.add_labels.is_some() || patch.remove_labels.is_some() {
            let mut labels = match &patch.labels {
                Some(labels) => clean_labels(labels),
                None => ticket.labels.clone(),
            };
            for gone in clean_labels(patch.remove_labels.as_deref().unwrap_or_default()) {
                labels.retain(|label| !label.eq_ignore_ascii_case(&gone));
            }
            for added in clean_labels(patch.add_labels.as_deref().unwrap_or_default()) {
                if !labels
                    .iter()
                    .any(|label| label.eq_ignore_ascii_case(&added))
                {
                    labels.push(added);
                }
            }
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
/// Moving a ticket to where it already is changes nothing (no `updated_at`
/// bump), and a deleted ticket cannot be moved: restore it first.
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
    if board.tickets[&id].deleted_at.is_some() {
        return Err("cannot move a deleted ticket".to_string());
    }
    let actor = actor_or_user(actor);
    let previous = board.tickets[&id].status.clone();

    let mut column: Vec<&Ticket> = board
        .tickets
        .values()
        .filter(|row| row.status == status && row.deleted_at.is_none())
        .collect();
    column.sort_by(|a, b| by_order(a, b));
    let mut ordered: Vec<String> = column
        .iter()
        .filter(|row| row.id != id)
        .map(|row| row.id.clone())
        .collect();
    let position = index.unwrap_or(ordered.len()).min(ordered.len());
    ordered.insert(position, id.clone());
    if previous == status && column.iter().map(|row| &row.id).eq(&ordered) {
        return Ok(Some(board.tickets[&id].clone()));
    }
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
