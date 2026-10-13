//! Chat mentions of tickets: `@kanban(id="<uuid>")`.
//!
//! `kanban::mention::search` feeds the console's `@kanban:` menu and
//! `kanban::mention::get` turns one id into the pill, the preview card and
//! the agent's one-line summary. The get function carries the provider
//! descriptor in its metadata (see `mention-contract`); agents read the
//! full ticket through `kanban::ticket::get`.

use mention_contract::{
    MentionField, MentionItem, MentionOpen, MentionProvider, MentionSearchRequest, MentionView,
    color, tone,
};
use serde_json::json;

use crate::board::{self, summarize};
use crate::config::KanbanConfig;
use crate::store::{Board, Ticket};

pub const SEARCH_FN: &str = "kanban::mention::search";
pub const GET_FN: &str = "kanban::mention::get";
pub const MENTION_NAME: &str = "kanban";
/// The console page that shows one ticket (`kanban/ui`).
pub const TICKET_PAGE: &str = "kanban-ticket";
pub const ICON: &str = "ticket";

pub fn provider() -> MentionProvider {
    MentionProvider::new(MENTION_NAME, "Tickets", SEARCH_FN)
        .description("A kanban ticket, by uuid or human key (KAN-12)")
        .icon(ICON)
        .color(color::BLUE)
        .details("kanban::ticket::get", "id")
}

/// Search rows for the `@kanban:` menu.
pub fn search(
    board: &Board,
    config: &KanbanConfig,
    request: &MentionSearchRequest,
) -> Vec<MentionItem> {
    board::search_tickets(board, request.trimmed_query(), request.effective_limit())
        .into_iter()
        .map(|ticket| item(ticket, config))
        .collect()
}

/// The view of one ticket (live or soft-deleted).
pub fn view(ticket: &Ticket, config: &KanbanConfig) -> MentionView {
    let status = column_label(config, &ticket.status);
    let mut fields = vec![
        MentionField::new("Status", status.clone()),
        MentionField::new("Priority", ticket.priority.clone())
            .tone(priority_tone(&ticket.priority)),
    ];
    if let Some(assignee) = ticket.assignee.as_deref().filter(|a| !a.is_empty()) {
        fields.push(MentionField::new("Assignee", assignee));
    }
    if !ticket.labels.is_empty() {
        fields.push(MentionField::new("Labels", ticket.labels.join(", ")));
    }
    if !ticket.comments.is_empty() {
        fields.push(MentionField::new(
            "Comments",
            ticket.comments.len().to_string(),
        ));
    }
    if ticket.deleted_at.is_some() {
        fields.push(MentionField::new("Deleted", "yes").tone(tone::DANGER));
    }
    MentionView {
        id: ticket.id.clone(),
        label: ticket.title.clone(),
        hint: Some(ticket.key.clone()),
        description: Some(status),
        icon: Some(ICON.to_string()),
        color: Some(ticket_color(ticket).to_string()),
        open: Some(MentionOpen::Page {
            page: TICKET_PAGE.to_string(),
            context: Some(json!({ "id": ticket.key })),
        }),
        summary: Some(summary(ticket, config)),
        data: Some(preview_data(ticket)),
        updated_at: Some(ticket.updated_at.clone()),
        fields,
    }
}

/// Longest description excerpt a mention carries.
const DESCRIPTION_EXCERPT_CHARS: usize = 280;

/// What the kanban preview card draws from: the board-card projection plus
/// the start of the description (never the comments or activity).
fn preview_data(ticket: &Ticket) -> serde_json::Value {
    let mut data = serde_json::to_value(summarize(ticket)).unwrap_or_else(|_| json!({}));
    let description = ticket.description.trim();
    let excerpt: String = description
        .chars()
        .take(DESCRIPTION_EXCERPT_CHARS)
        .collect();
    let truncated = excerpt.len() < description.len();
    if let Some(object) = data.as_object_mut() {
        object.insert(
            "description".into(),
            json!(if truncated {
                format!("{}…", excerpt.trim_end())
            } else {
                excerpt
            }),
        );
    }
    data
}

fn item(ticket: &Ticket, config: &KanbanConfig) -> MentionItem {
    let mut description = format!(
        "{} · {}",
        column_label(config, &ticket.status),
        ticket.priority
    );
    if let Some(assignee) = ticket.assignee.as_deref().filter(|a| !a.is_empty()) {
        description.push_str(" · ");
        description.push_str(assignee);
    }
    MentionItem {
        id: ticket.id.clone(),
        label: ticket.title.clone(),
        hint: Some(ticket.key.clone()),
        description: Some(description),
        icon: None,
        color: Some(ticket_color(ticket).to_string()),
    }
}

fn summary(ticket: &Ticket, config: &KanbanConfig) -> String {
    let title = serde_json::to_string(&ticket.title).unwrap_or_default();
    let mut line = format!(
        "Kanban ticket {} {title} · status: {} · priority: {}",
        ticket.key,
        column_label(config, &ticket.status),
        ticket.priority,
    );
    match ticket.assignee.as_deref().filter(|a| !a.is_empty()) {
        Some(assignee) => line.push_str(&format!(" · assignee: {assignee}")),
        None => line.push_str(" · unassigned"),
    }
    if !ticket.labels.is_empty() {
        line.push_str(&format!(" · labels: {}", ticket.labels.join(", ")));
    }
    if !ticket.comments.is_empty() {
        line.push_str(&format!(" · {} comment(s)", ticket.comments.len()));
    }
    if ticket.deleted_at.is_some() {
        line.push_str(" · deleted");
    }
    line
}

fn column_label(config: &KanbanConfig, status: &str) -> String {
    config
        .columns
        .iter()
        .find(|column| column.id == status)
        .map(|column| column.label.clone())
        .unwrap_or_else(|| status.to_string())
}

fn ticket_color(ticket: &Ticket) -> &'static str {
    if ticket.deleted_at.is_some() {
        return color::NEUTRAL;
    }
    match ticket.priority.as_str() {
        "urgent" => color::ROSE,
        "high" => color::AMBER,
        "medium" => color::BLUE,
        "low" => color::NEUTRAL,
        _ => color::BLUE,
    }
}

fn priority_tone(priority: &str) -> &'static str {
    match priority {
        "urgent" => tone::DANGER,
        "high" => tone::WARNING,
        _ => tone::NEUTRAL,
    }
}
