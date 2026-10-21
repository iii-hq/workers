//! The `kanban::*` function catalog: typed request/response structs and the
//! handlers that route every mutation through the store and the trigger
//! fan-out.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use mention_contract::{
    MentionGetRequest, MentionSearchRequest, MentionSearchResponse, MentionView,
};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use crate::agents::{self, AgentList};
use crate::board::{
    self, ActivityFilter, BoardContext, CommentFilter, CreateCommentInput, CreateTicketInput,
    TicketBrief, TicketFilter, TicketRow, TicketView, UpdatePatch, row, ticket_view, view_of,
};
use crate::config::Column;
use crate::config::KanbanConfig;
use crate::events::{
    ChangeEvent, CommentEvent, SubscriberCounts, Subscribers, emit_change, emit_comment,
};
use crate::mentions;
use crate::plan::{
    self, Buckets, ConvertInput, ConvertInto, EntityView, ProjectFields, ProjectView,
    ReleaseFields, ReleaseView,
};
use crate::store::{
    Activity, Board, BoardStore, Comment, ConfigCell, Pill, Ticket, config_snapshot,
};

pub const FUNCTION_IDS: [&str; 28] = [
    "kanban::config::info",
    "kanban::board::get",
    "kanban::ticket::create",
    "kanban::ticket::get",
    "kanban::ticket::list",
    "kanban::ticket::links",
    "kanban::ticket::update",
    "kanban::ticket::move",
    "kanban::ticket::delete",
    "kanban::ticket::restore",
    "kanban::comment::create",
    "kanban::comment::list",
    "kanban::activity::list",
    "kanban::agent::list",
    "kanban::project::list",
    "kanban::project::create",
    "kanban::project::update",
    "kanban::project::archive",
    "kanban::project::restore",
    "kanban::release::list",
    "kanban::release::create",
    "kanban::release::update",
    "kanban::release::ship",
    "kanban::release::reopen",
    "kanban::release::delete",
    "kanban::label::convert",
    mentions::SEARCH_FN,
    mentions::GET_FN,
];

pub struct Ctx {
    pub iii: Arc<IIIClient>,
    pub store: BoardStore,
    pub config: ConfigCell,
    pub subscribers: Subscribers,
}

/* ── requests ─────────────────────────────────────────────────────────── */

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct Empty {}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct BoardGetInput {
    /// Include soft-deleted tickets. Default false.
    #[serde(default)]
    pub include_deleted: bool,
    /// Only these column ids (board order kept). Omit for every column.
    #[serde(default)]
    pub statuses: Option<Vec<String>>,
    /// Only tickets updated after this ISO timestamp — what changed since your last read.
    #[serde(default)]
    pub updated_since: Option<String>,
    /// At most this many tickets per column, from the top of the column.
    #[serde(default)]
    pub limit: Option<u64>,
    /// Return the compact row (id, key, title, status, priority, assignee, comment_count) instead of the full summary.
    #[serde(default)]
    pub compact: bool,
    /// Only the subtasks of this ticket (uuid or key); empty string = tickets that are not subtasks.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Only tickets in this project (id or name); empty string = tickets in no project.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Only tickets in this release (id or name); empty string = tickets in no release.
    #[serde(default)]
    pub release_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketCreateInput {
    /// Required.
    pub title: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Column id; defaults to the configured default_status.
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    /// Agent profile id.
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Badges to show on the card: `[{ "icon": "calendar", "text": "Fri 17:00" }]`; `icon` is a lucide icon name.
    #[serde(default)]
    pub pills: Option<Vec<Pill>>,
    /// Make the new ticket a subtask of this ticket (uuid or key). The parent must be live and not a subtask itself.
    #[serde(default)]
    pub parent: Option<String>,
    /// Project id or name. An archived project refuses new tickets.
    #[serde(default)]
    pub project: Option<String>,
    /// Release id or name.
    #[serde(default)]
    pub release: Option<String>,
    /// Who is making the change. Defaults to `user`.
    #[serde(default)]
    pub actor: Option<String>,
    /// Return the compact row (id, key, title, status, priority, assignee, comment_count) instead of the full ticket.
    #[serde(default)]
    pub compact: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketGetInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketLinksInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub id: String,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TicketListInput {
    /// Column id.
    #[serde(default)]
    pub status: Option<String>,
    /// Agent profile id; empty string matches unassigned.
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub include_deleted: bool,
    /// Only tickets updated after this ISO timestamp.
    #[serde(default)]
    pub updated_since: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    /// Return the compact row (id, key, title, status, priority, assignee, comment_count) instead of the full summary.
    #[serde(default)]
    pub compact: bool,
    /// Only the subtasks of this ticket (uuid or key); empty string = tickets that are not subtasks.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Only tickets in this project (id or name); empty string = tickets in no project.
    #[serde(default)]
    pub project_id: Option<String>,
    /// Only tickets in this release (id or name); empty string = tickets in no release.
    #[serde(default)]
    pub release_id: Option<String>,
}

fn double_option<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(de).map(Some)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketUpdateInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(default)]
    pub priority: Option<String>,
    /// Agent profile id, or null to unassign.
    #[serde(default, deserialize_with = "double_option")]
    pub assignee: Option<Option<String>>,
    /// Replaces the whole label set. Prefer add_labels / remove_labels: they edit the labels the ticket has when the write runs, so concurrent label edits do not overwrite each other.
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    /// Labels to add to the current ones; one already present (ASCII case ignored) is skipped. Applied after `labels` and `remove_labels`.
    #[serde(default)]
    pub add_labels: Option<Vec<String>>,
    /// Labels to drop from the current ones (ASCII case ignored). Applied after `labels`, before `add_labels`.
    #[serde(default)]
    pub remove_labels: Option<Vec<String>>,
    /// Replaces every badge on the card: `[{ "icon": "alarm-clock", "text": "09:00" }]`; `icon` is a lucide icon name. `[]` clears them.
    #[serde(default)]
    pub pills: Option<Vec<Pill>>,
    /// Ticket uuid or key to make this ticket a subtask of, or null to detach it. One level only: the parent must be live and not a subtask, and a ticket with subtasks cannot become one.
    #[serde(default, deserialize_with = "double_option")]
    pub parent: Option<Option<String>>,
    /// Project id or name, or null to clear. An archived project refuses new tickets.
    #[serde(default, deserialize_with = "double_option")]
    pub project: Option<Option<String>>,
    /// Release id or name, or null to clear.
    #[serde(default, deserialize_with = "double_option")]
    pub release: Option<Option<String>>,
    #[serde(default)]
    pub actor: Option<String>,
    /// Return the compact row (id, key, title, status, priority, assignee, comment_count) instead of the full ticket.
    #[serde(default)]
    pub compact: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketMoveInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub id: String,
    /// Target column id.
    pub status: String,
    /// Zero-based position in the target column.
    #[serde(default)]
    pub index: Option<u64>,
    #[serde(default)]
    pub actor: Option<String>,
    /// Return the compact row (id, key, title, status, priority, assignee, comment_count) instead of the full ticket.
    #[serde(default)]
    pub compact: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketRefInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub id: String,
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentCreateInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub ticket_id: String,
    /// Comment body. Markdown is rendered in the console.
    pub body: String,
    /// Comment id to reply to; replies stay one level deep.
    #[serde(default)]
    pub parent_id: Option<String>,
    /// Agent profile id, or `user`. Defaults to `user`.
    #[serde(default)]
    pub author: Option<String>,
    /// Return the ticket as its compact row (id, key, title, status, priority, assignee, comment_count) instead of the full ticket.
    #[serde(default)]
    pub compact: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CommentListInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub ticket_id: String,
    /// Only comments by exactly this author.
    #[serde(default)]
    pub author: Option<String>,
    /// Skip comments by this author — e.g. your own.
    #[serde(default)]
    pub exclude_author: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    #[serde(default)]
    pub root_only: bool,
    /// ISO timestamp; only comments created after it.
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub include_deleted: bool,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ActivityListInput {
    /// Ticket uuid or human-readable key (KAN-1, case-insensitive).
    pub ticket_id: String,
    #[serde(default)]
    pub types: Option<Vec<String>>,
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ProjectListInput {
    /// Include archived projects. Default false.
    #[serde(default)]
    pub include_archived: bool,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectCreateInput {
    /// Required; unique among projects (ASCII case ignored).
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Agent profile id, or `user`.
    #[serde(default)]
    pub lead: Option<String>,
    /// blue | purple | teal | green | amber | rose.
    #[serde(default)]
    pub color: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ProjectUpdateInput {
    /// Project id or name.
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Agent profile id, `user`, or null to clear.
    #[serde(default, deserialize_with = "double_option")]
    pub lead: Option<Option<String>>,
    /// blue | purple | teal | green | amber | rose, or null to clear.
    #[serde(default, deserialize_with = "double_option")]
    pub color: Option<Option<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EntityRefInput {
    /// Project or release id, or its name.
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseCreateInput {
    /// Required; unique among releases (ASCII case ignored).
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// Target date, `YYYY-MM-DD`.
    #[serde(default)]
    pub target_at: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseUpdateInput {
    /// Release id or name.
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    /// Target date `YYYY-MM-DD`, or null to clear.
    #[serde(default, deserialize_with = "double_option")]
    pub target_at: Option<Option<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ReleaseShipInput {
    /// Release id or name.
    pub id: String,
    /// Where the live tickets that are not done go: another release's id or name, or null to take them out of any release. Required when there are such tickets; the target must exist and not be released.
    #[serde(default, deserialize_with = "double_option")]
    pub move_open_to: Option<Option<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LabelConvertInput {
    /// The label to convert; matched ASCII case-insensitively.
    pub label: String,
    /// Convert into a project or a release.
    pub into: ConvertInto,
    /// An existing project or release (id or name) to add the tickets to.
    #[serde(default)]
    pub id: Option<String>,
    /// Without `id`: the name of the project or release to create, or to reuse when one has this name.
    #[serde(default)]
    pub name: Option<String>,
    /// Description of a project or release created by this call.
    #[serde(default)]
    pub description: Option<String>,
    /// Target date `YYYY-MM-DD` of a release created by this call.
    #[serde(default)]
    pub target_at: Option<String>,
    /// Also move tickets that are in another project or release. Default false: they are kept where they are.
    #[serde(default)]
    pub replace: bool,
    /// Remove the label from the tickets that end up in the project or release. Default false.
    #[serde(default)]
    pub remove_label: bool,
    /// Releases: mark the release released when every live ticket in it is done, dated at the last of them.
    #[serde(default)]
    pub mark_released_if_done: bool,
    /// Compute the buckets and change nothing.
    #[serde(default)]
    pub dry_run: bool,
}

/* ── responses ────────────────────────────────────────────────────────── */

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConfigInfo {
    /// The worker name, `kanban`: every function id, trigger type and page id starts with it.
    pub worker: String,
    pub configuration_id: String,
    /// Configured path, as written.
    pub data_path: String,
    /// Absolute folder the board file is read from.
    pub data_path_resolved: String,
    /// Absolute path of board.json.
    pub board_file: String,
    pub project_root: String,
    pub id_prefix: String,
    pub default_status: String,
    /// The column that counts as done for progress, releases and subtasks.
    pub done_status: String,
    /// The column that counts as canceled (out of progress and release totals); null when the board has none.
    pub canceled_status: Option<String>,
    /// Board columns left to right.
    pub columns: Vec<Column>,
    pub priorities: Vec<String>,
    pub agents_path: String,
    pub agents_path_resolved: String,
    pub subscribers: SubscriberCounts,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BoardColumnView {
    pub id: String,
    pub label: String,
    pub tickets: Vec<TicketRow>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct BoardView {
    /// The columns asked for (all by default), each with the tickets the filters kept.
    pub columns: Vec<BoardColumnView>,
    /// Every live ticket on the board, regardless of the filters above.
    pub ticket_count: u64,
    pub deleted_count: u64,
    pub priorities: Vec<String>,
    pub default_status: String,
    /// The column that counts as done for progress, releases and subtasks.
    pub done_status: String,
    /// The column that counts as canceled (out of progress and release totals); null when the board has none.
    pub canceled_status: Option<String>,
    pub id_prefix: String,
    pub data_path_resolved: String,
    pub board_file: String,
    pub updated_at: String,
    /// Every project, archived ones included, by name.
    pub projects: Vec<ProjectView>,
    /// Every release, by target date (none last) then name.
    pub releases: Vec<ReleaseView>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TicketList {
    pub tickets: Vec<TicketRow>,
    pub count: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TicketLinks {
    /// Live tickets whose key appears in this ticket's title, description or comments, by ticket number.
    pub mentions: Vec<TicketRow>,
    /// Live tickets whose title, description or comments mention this ticket's key, by ticket number.
    pub mentioned_by: Vec<TicketRow>,
}

/// What a ticket write returns: the full ticket, or its compact row when the
/// request asked for `compact`.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum TicketWritten {
    Full(Box<TicketView>),
    Brief(TicketBrief),
}

pub fn written(view: TicketView, compact: bool) -> TicketWritten {
    if compact {
        TicketWritten::Brief(board::brief(&view.ticket))
    } else {
        TicketWritten::Full(Box::new(view))
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommentCreated {
    pub comment: Comment,
    pub ticket: TicketWritten,
}

pub fn comment_created(comment: Comment, ticket: TicketView, compact: bool) -> CommentCreated {
    CommentCreated {
        comment,
        ticket: written(ticket, compact),
    }
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ProjectList {
    pub projects: Vec<ProjectView>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ReleaseList {
    pub releases: Vec<ReleaseView>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ShipResult {
    pub release: ReleaseView,
    /// The tickets that were not done and moved out of the release.
    pub moved: Vec<TicketRow>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Deleted {
    pub deleted: bool,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConvertResult {
    /// The project or release after the conversion; with `dry_run`, the existing one or null.
    pub entity: Option<EntityView>,
    pub created: bool,
    /// Ticket keys per outcome, from where the tickets were before the call.
    pub buckets: Buckets,
    /// The tickets that were changed.
    pub tickets: Vec<TicketRow>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommentList {
    pub comments: Vec<Comment>,
    pub count: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ActivityList {
    pub activity: Vec<Activity>,
    pub count: u64,
}

/* ── plumbing ─────────────────────────────────────────────────────────── */

fn not_found(id: &str) -> Error {
    Error::Handler(format!("ticket not found: {id}"))
}

fn limit(value: Option<u64>) -> Option<usize> {
    value.map(|limit| limit as usize)
}

/// What a mutation hands back to [`apply`]: the function's result and the
/// tickets to announce.
pub struct Outcome<T> {
    pub result: T,
    /// The tickets that changed, announced on `kanban:change` one event each:
    /// the first under the call's own event name, any further one (a parent
    /// written by an attach, a ticket moved by a ship) as `ticket.updated`.
    /// Empty fires nothing.
    pub tickets: Vec<TicketView>,
    pub comment: Option<Comment>,
}

impl<T> Outcome<T> {
    /// A mutation that changed no ticket (a project, a release).
    pub fn silent(result: T) -> Self {
        Self {
            result,
            tickets: Vec::new(),
            comment: None,
        }
    }
}

/// The `kanban:change` events of one write, one per announced ticket: the
/// first under the call's own `event` name (carrying the comment, if any), any
/// further one (a parent written by an attach, a ticket moved by a ship) as
/// `ticket.updated`. `at` is the write's timestamp.
pub fn change_events(
    board: &Board,
    event: &str,
    tickets: &[TicketView],
    comment: Option<&Comment>,
    at: &str,
) -> Vec<ChangeEvent> {
    tickets
        .iter()
        .enumerate()
        .map(|(index, view)| ChangeEvent {
            event: if index == 0 { event } else { "ticket.updated" }.to_string(),
            ticket_id: view.ticket.id.clone(),
            ticket_key: view.ticket.key.clone(),
            ticket: view.clone().into(),
            comment: comment.filter(|_| index == 0).cloned(),
            activity: view.ticket.activity.last().cloned(),
            at: at.to_string(),
            releases: board::event_releases(board, &view.ticket, at),
        })
        .collect()
}

/// Run one mutation under the store lock, then fan the result out to the
/// `kanban:change` (and, for a comment, `kanban:comment`) bindings.
async fn apply<T>(
    ctx: &Ctx,
    event: &str,
    run: impl FnOnce(&mut Board, &BoardContext, &str) -> Result<Outcome<T>, String>,
) -> Result<Outcome<T>, Error> {
    let (outcome, at, events) = ctx
        .store
        .update(|board, config| {
            let at = board::next_at(board, chrono::Utc::now().timestamp_millis());
            let outcome = run(board, &BoardContext::from(config), &at)?;
            let events = change_events(
                board,
                event,
                &outcome.tickets,
                outcome.comment.as_ref(),
                &at,
            );
            Ok((outcome, at, events))
        })
        .await
        .map_err(Error::Handler)?;
    for event in &events {
        emit_change(&ctx.iii, &ctx.subscribers, event);
    }
    if let (Some(view), Some(comment)) = (outcome.tickets.first(), &outcome.comment) {
        emit_comment(
            &ctx.iii,
            &ctx.subscribers,
            &CommentEvent {
                event: "comment.created".to_string(),
                ticket_id: view.ticket.id.clone(),
                ticket_key: view.ticket.key.clone(),
                ticket: view.clone().into(),
                comment: comment.clone(),
                author: comment.author.clone(),
                at,
            },
        );
    }
    Ok(outcome)
}

/// The views of these tickets on the board as it is now (unknown ids skipped).
pub fn views<'a>(
    board: &Board,
    context: &BoardContext,
    ids: impl IntoIterator<Item = &'a String>,
) -> Vec<TicketView> {
    ids.into_iter()
        .filter_map(|id| {
            view_of(
                board,
                id,
                &context.done_status,
                context.canceled_status.as_deref(),
            )
        })
        .collect()
}

/// The outcome of a mutation of one ticket, plus the tickets it wrote besides
/// (`related`: the old and new parent of an attach). The events fan out only
/// when the ticket differs from how it was before: a patch, move, delete or
/// restore that changed nothing fires no trigger (its activity tail would be
/// stale). The result is the ticket's view on the board after the write.
pub fn ticket_outcome(
    board: &Board,
    context: &BoardContext,
    before: Option<Ticket>,
    after: Option<Ticket>,
    related: &[String],
) -> Outcome<Option<TicketView>> {
    let view = after.as_ref().map(|ticket| {
        ticket_view(
            board,
            ticket,
            &context.done_status,
            context.canceled_status.as_deref(),
        )
    });
    let tickets = if before == after {
        Vec::new()
    } else {
        view.iter()
            .cloned()
            .chain(views(board, context, related))
            .collect()
    };
    Outcome {
        result: view,
        tickets,
        comment: None,
    }
}

async fn read(ctx: &Ctx) -> Result<Board, Error> {
    ctx.store.read().await.map_err(Error::Handler)
}

/* ── handlers ─────────────────────────────────────────────────────────── */

async fn config_info(ctx: Arc<Ctx>, _: Empty) -> Result<ConfigInfo, Error> {
    let config = config_snapshot(&ctx.config);
    Ok(ConfigInfo {
        worker: "kanban".to_string(),
        configuration_id: crate::configuration::config_id().to_string(),
        data_path: config.data_path.clone(),
        data_path_resolved: config.data_path_resolved().display().to_string(),
        board_file: config.board_file().display().to_string(),
        project_root: crate::config::project_root().display().to_string(),
        id_prefix: config.id_prefix.clone(),
        default_status: config.default_status.clone(),
        done_status: config.done_status.clone(),
        canceled_status: config.canceled_status.clone(),
        columns: config.columns.clone(),
        priorities: config.priorities.clone(),
        agents_path: config.agents_path.clone(),
        agents_path_resolved: config.agents_path_resolved().display().to_string(),
        subscribers: ctx.subscribers.counts(),
    })
}

/// The board projection `kanban::board::get` returns — pure, so the filters
/// are testable without an engine.
pub fn board_view(board: &Board, config: &KanbanConfig, input: &BoardGetInput) -> BoardView {
    let wanted: Vec<&str> = input
        .statuses
        .as_deref()
        .unwrap_or_default()
        .iter()
        .map(|status| status.trim())
        .filter(|status| !status.is_empty())
        .collect();
    let filter = |status: Option<String>| TicketFilter {
        status,
        include_deleted: input.include_deleted,
        updated_since: input.updated_since.clone(),
        parent_id: input.parent_id.clone(),
        project_id: input.project_id.clone(),
        release_id: input.release_id.clone(),
        limit: limit(input.limit),
        ..TicketFilter::default()
    };
    let context = BoardContext::from(config);
    BoardView {
        columns: config
            .columns
            .iter()
            .filter(|column| wanted.is_empty() || wanted.contains(&column.id.as_str()))
            .map(|column| BoardColumnView {
                id: column.id.clone(),
                label: column.label.clone(),
                tickets: board::list_tickets(board, &filter(Some(column.id.clone())))
                    .into_iter()
                    .map(|ticket| row(ticket, input.compact))
                    .collect(),
            })
            .collect(),
        ticket_count: board::list_tickets(
            board,
            &TicketFilter {
                include_deleted: input.include_deleted,
                ..TicketFilter::default()
            },
        )
        .len() as u64,
        deleted_count: board
            .tickets
            .values()
            .filter(|ticket| ticket.deleted_at.is_some())
            .count() as u64,
        priorities: config.priorities.clone(),
        default_status: config.default_status.clone(),
        done_status: config.done_status.clone(),
        canceled_status: config.canceled_status.clone(),
        id_prefix: config.id_prefix.clone(),
        data_path_resolved: config.data_path_resolved().display().to_string(),
        board_file: config.board_file().display().to_string(),
        updated_at: board::now(),
        projects: plan::project_views(board, &context, true),
        releases: plan::release_views(board, &context),
    }
}

async fn board_get(ctx: Arc<Ctx>, input: BoardGetInput) -> Result<BoardView, Error> {
    let config = config_snapshot(&ctx.config);
    let board = read(&ctx).await?;
    Ok(board_view(&board, &config, &input))
}

async fn ticket_create(ctx: Arc<Ctx>, input: TicketCreateInput) -> Result<TicketWritten, Error> {
    let create = CreateTicketInput {
        title: input.title,
        description: input.description,
        status: input.status,
        priority: input.priority,
        assignee: input.assignee,
        labels: input.labels,
        pills: input.pills,
        parent: input.parent,
        project: input.project,
        release: input.release,
        actor: input.actor,
    };
    let outcome = apply(&ctx, "ticket.created", |board, context, at| {
        let ticket = board::create_ticket(board, &create, context, at)?;
        // The new ticket first, then the parent it was filed under.
        let ids: Vec<String> = std::iter::once(ticket.id.clone())
            .chain(ticket.parent_id.clone())
            .collect();
        let tickets = views(board, context, &ids);
        Ok(Outcome {
            result: tickets[0].clone(),
            tickets,
            comment: None,
        })
    })
    .await?;
    Ok(written(outcome.result, input.compact))
}

async fn ticket_get(ctx: Arc<Ctx>, input: TicketGetInput) -> Result<TicketView, Error> {
    let config = config_snapshot(&ctx.config);
    let board = read(&ctx).await?;
    board::find_ticket(&board, &input.id)
        .map(|ticket| {
            ticket_view(
                &board,
                ticket,
                &config.done_status,
                config.canceled_status.as_deref(),
            )
        })
        .ok_or_else(|| not_found(&input.id))
}

async fn ticket_list(ctx: Arc<Ctx>, input: TicketListInput) -> Result<TicketList, Error> {
    let board = read(&ctx).await?;
    let filter = TicketFilter {
        status: input.status,
        assignee: input.assignee,
        include_deleted: input.include_deleted,
        updated_since: input.updated_since,
        parent_id: input.parent_id,
        project_id: input.project_id,
        release_id: input.release_id,
        limit: limit(input.limit),
    };
    let tickets: Vec<TicketRow> = board::list_tickets(&board, &filter)
        .into_iter()
        .map(|ticket| row(ticket, input.compact))
        .collect();
    Ok(TicketList {
        count: tickets.len() as u64,
        tickets,
    })
}

async fn ticket_links(ctx: Arc<Ctx>, input: TicketLinksInput) -> Result<TicketLinks, Error> {
    let config = config_snapshot(&ctx.config);
    let board = read(&ctx).await?;
    let (mentions, mentioned_by) = board::ticket_links(&board, &input.id, &config.id_prefix)
        .ok_or_else(|| not_found(&input.id))?;
    let rows = |tickets: Vec<&Ticket>| -> Vec<TicketRow> {
        tickets
            .into_iter()
            .map(|ticket| row(ticket, false))
            .collect()
    };
    Ok(TicketLinks {
        mentions: rows(mentions),
        mentioned_by: rows(mentioned_by),
    })
}

async fn ticket_update(ctx: Arc<Ctx>, input: TicketUpdateInput) -> Result<TicketWritten, Error> {
    let patch = UpdatePatch {
        title: input.title,
        description: input.description,
        priority: input.priority,
        assignee: input.assignee,
        status: input.status,
        labels: input.labels,
        add_labels: input.add_labels,
        remove_labels: input.remove_labels,
        pills: input.pills,
        parent: input.parent,
        project: input.project,
        release: input.release,
    };
    let actor = input
        .actor
        .as_deref()
        .map(str::trim)
        .filter(|actor| !actor.is_empty())
        .unwrap_or("user")
        .to_string();
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        let before = board::find_ticket(board, &input.id).cloned();
        let updated = board::update_ticket(board, &input.id, &patch, &actor, context, at)?;
        let related = updated
            .as_ref()
            .map(|result| result.related.clone())
            .unwrap_or_default();
        Ok(ticket_outcome(
            board,
            context,
            before,
            updated.map(|result| result.ticket),
            &related,
        ))
    })
    .await?;
    outcome
        .result
        .map(|ticket| written(ticket, input.compact))
        .ok_or_else(|| not_found(&input.id))
}

async fn ticket_move(ctx: Arc<Ctx>, input: TicketMoveInput) -> Result<TicketWritten, Error> {
    let outcome = apply(&ctx, "ticket.moved", |board, context, at| {
        let before = board::find_ticket(board, &input.id).cloned();
        let moved = board::move_ticket(
            board,
            &input.id,
            &input.status,
            limit(input.index),
            input.actor.as_deref(),
            context,
            at,
        )?;
        Ok(ticket_outcome(board, context, before, moved, &[]))
    })
    .await?;
    outcome
        .result
        .map(|ticket| written(ticket, input.compact))
        .ok_or_else(|| not_found(&input.id))
}

fn actor_of(input: &TicketRefInput) -> String {
    input
        .actor
        .as_deref()
        .map(str::trim)
        .filter(|actor| !actor.is_empty())
        .unwrap_or("user")
        .to_string()
}

async fn ticket_delete(ctx: Arc<Ctx>, input: TicketRefInput) -> Result<TicketView, Error> {
    let actor = actor_of(&input);
    let outcome = apply(&ctx, "ticket.deleted", |board, context, at| {
        let before = board::find_ticket(board, &input.id).cloned();
        let after = board::delete_ticket(board, &input.id, &actor, at);
        Ok(ticket_outcome(board, context, before, after, &[]))
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.id))
}

async fn ticket_restore(ctx: Arc<Ctx>, input: TicketRefInput) -> Result<TicketView, Error> {
    let actor = actor_of(&input);
    let outcome = apply(&ctx, "ticket.restored", |board, context, at| {
        let before = board::find_ticket(board, &input.id).cloned();
        let restored = board::restore_ticket(board, &input.id, &actor, at);
        let related = restored
            .as_ref()
            .map(|result| result.related.clone())
            .unwrap_or_default();
        Ok(ticket_outcome(
            board,
            context,
            before,
            restored.map(|result| result.ticket),
            &related,
        ))
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.id))
}

async fn comment_create(ctx: Arc<Ctx>, input: CommentCreateInput) -> Result<CommentCreated, Error> {
    let compact = input.compact;
    let create = CreateCommentInput {
        ticket_id: input.ticket_id.clone(),
        body: input.body,
        parent_id: input.parent_id,
        author: input.author,
    };
    let outcome = apply(&ctx, "comment.created", |board, context, at| {
        Ok(match board::create_comment(board, &create, at)? {
            Some(result) => {
                let view = ticket_view(
                    board,
                    &result.ticket,
                    &context.done_status,
                    context.canceled_status.as_deref(),
                );
                Outcome {
                    result: Some(comment_created(
                        result.comment.clone(),
                        view.clone(),
                        compact,
                    )),
                    tickets: vec![view],
                    comment: Some(result.comment),
                }
            }
            None => Outcome::silent(None),
        })
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.ticket_id))
}

async fn comment_list(ctx: Arc<Ctx>, input: CommentListInput) -> Result<CommentList, Error> {
    let board = read(&ctx).await?;
    let comments = board::list_comments(
        &board,
        &CommentFilter {
            ticket_id: input.ticket_id,
            author: input.author,
            exclude_author: input.exclude_author,
            parent_id: input.parent_id,
            root_only: input.root_only,
            since: input.since,
            include_deleted: input.include_deleted,
            limit: limit(input.limit),
        },
    );
    Ok(CommentList {
        count: comments.len() as u64,
        comments,
    })
}

async fn activity_list(ctx: Arc<Ctx>, input: ActivityListInput) -> Result<ActivityList, Error> {
    let board = read(&ctx).await?;
    let activity = board::list_activity(
        &board,
        &ActivityFilter {
            ticket_id: input.ticket_id,
            types: input.types,
            since: input.since,
            limit: limit(input.limit),
        },
    );
    Ok(ActivityList {
        count: activity.len() as u64,
        activity,
    })
}

async fn agent_list(ctx: Arc<Ctx>, _: Empty) -> Result<AgentList, Error> {
    let dir = config_snapshot(&ctx.config).agents_path_resolved();
    Ok(agents::list_agents(&ctx.iii, &dir).await)
}

async fn project_list(ctx: Arc<Ctx>, input: ProjectListInput) -> Result<ProjectList, Error> {
    let context = BoardContext::from(&config_snapshot(&ctx.config));
    let board = read(&ctx).await?;
    Ok(ProjectList {
        projects: plan::project_views(&board, &context, input.include_archived),
    })
}

async fn project_create(ctx: Arc<Ctx>, input: ProjectCreateInput) -> Result<ProjectView, Error> {
    let fields = ProjectFields {
        name: Some(input.name),
        description: input.description,
        lead: input.lead.map(Some),
        color: input.color.map(Some),
    };
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        Ok(Outcome::silent(plan::create_project(
            board, &fields, context, at,
        )?))
    })
    .await?;
    Ok(outcome.result)
}

async fn project_update(ctx: Arc<Ctx>, input: ProjectUpdateInput) -> Result<ProjectView, Error> {
    let fields = ProjectFields {
        name: input.name,
        description: input.description,
        lead: input.lead,
        color: input.color,
    };
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        Ok(Outcome::silent(plan::update_project(
            board, &input.id, &fields, context, at,
        )?))
    })
    .await?;
    Ok(outcome.result)
}

async fn set_project_archived(
    ctx: Arc<Ctx>,
    input: EntityRefInput,
    archived: bool,
) -> Result<ProjectView, Error> {
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        Ok(Outcome::silent(plan::set_project_archived(
            board, &input.id, archived, context, at,
        )?))
    })
    .await?;
    Ok(outcome.result)
}

async fn project_archive(ctx: Arc<Ctx>, input: EntityRefInput) -> Result<ProjectView, Error> {
    set_project_archived(ctx, input, true).await
}

async fn project_restore(ctx: Arc<Ctx>, input: EntityRefInput) -> Result<ProjectView, Error> {
    set_project_archived(ctx, input, false).await
}

async fn release_list(ctx: Arc<Ctx>, _: Empty) -> Result<ReleaseList, Error> {
    let context = BoardContext::from(&config_snapshot(&ctx.config));
    let board = read(&ctx).await?;
    Ok(ReleaseList {
        releases: plan::release_views(&board, &context),
    })
}

async fn release_create(ctx: Arc<Ctx>, input: ReleaseCreateInput) -> Result<ReleaseView, Error> {
    let fields = ReleaseFields {
        name: Some(input.name),
        description: input.description,
        target_at: input.target_at.map(Some),
    };
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        Ok(Outcome::silent(plan::create_release(
            board, &fields, context, at,
        )?))
    })
    .await?;
    Ok(outcome.result)
}

async fn release_update(ctx: Arc<Ctx>, input: ReleaseUpdateInput) -> Result<ReleaseView, Error> {
    let fields = ReleaseFields {
        name: input.name,
        description: input.description,
        target_at: input.target_at,
    };
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        Ok(Outcome::silent(plan::update_release(
            board, &input.id, &fields, context, at,
        )?))
    })
    .await?;
    Ok(outcome.result)
}

async fn release_ship(ctx: Arc<Ctx>, input: ReleaseShipInput) -> Result<ShipResult, Error> {
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        let (release, moved) =
            plan::ship_release(board, &input.id, input.move_open_to.clone(), context, at)?;
        Ok(Outcome {
            result: ShipResult {
                release,
                moved: moved
                    .iter()
                    .map(|id| row(&board.tickets[id], false))
                    .collect(),
            },
            tickets: views(board, context, &moved),
            comment: None,
        })
    })
    .await?;
    Ok(outcome.result)
}

async fn release_reopen(ctx: Arc<Ctx>, input: EntityRefInput) -> Result<ReleaseView, Error> {
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        Ok(Outcome::silent(plan::reopen_release(
            board, &input.id, context, at,
        )?))
    })
    .await?;
    Ok(outcome.result)
}

async fn release_delete(ctx: Arc<Ctx>, input: EntityRefInput) -> Result<Deleted, Error> {
    let outcome = apply(&ctx, "ticket.updated", |board, _, _| {
        plan::delete_release(board, &input.id)?;
        Ok(Outcome::silent(Deleted { deleted: true }))
    })
    .await?;
    Ok(outcome.result)
}

async fn label_convert(ctx: Arc<Ctx>, input: LabelConvertInput) -> Result<ConvertResult, Error> {
    let convert = ConvertInput {
        label: input.label,
        into: input.into,
        id: input.id,
        name: input.name,
        description: input.description,
        target_at: input.target_at,
        replace: input.replace,
        remove_label: input.remove_label,
        mark_released_if_done: input.mark_released_if_done,
    };
    if input.dry_run {
        // Read-only: the board file is not rewritten.
        let context = BoardContext::from(&config_snapshot(&ctx.config));
        let board = read(&ctx).await?;
        let plan = plan::plan_convert(&board, &convert).map_err(Error::Handler)?;
        return Ok(ConvertResult {
            entity: plan
                .existing()
                .and_then(|id| plan::entity_view(&board, convert.into, id, &context)),
            created: false,
            buckets: plan.buckets(&board),
            tickets: Vec::new(),
        });
    }
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        let planned = plan::plan_convert(board, &convert)?;
        let buckets = planned.buckets(board);
        let converted = plan::apply_convert(board, &planned, &convert, context, at);
        Ok(Outcome {
            tickets: views(board, context, &converted.changed),
            result: ConvertResult {
                entity: Some(converted.entity),
                created: converted.created,
                buckets,
                tickets: converted
                    .changed
                    .iter()
                    .map(|id| row(&board.tickets[id], false))
                    .collect(),
            },
            comment: None,
        })
    })
    .await?;
    Ok(outcome.result)
}

async fn mention_search(
    ctx: Arc<Ctx>,
    input: MentionSearchRequest,
) -> Result<MentionSearchResponse, Error> {
    let board = read(&ctx).await?;
    let config = config_snapshot(&ctx.config);
    Ok(MentionSearchResponse {
        items: mentions::search(&board, &config, &input),
    })
}

async fn mention_get(
    ctx: Arc<Ctx>,
    input: MentionGetRequest,
) -> Result<Option<MentionView>, Error> {
    let board = read(&ctx).await?;
    let config = config_snapshot(&ctx.config);
    Ok(board::find_ticket(&board, &input.id).map(|ticket| mentions::view(&board, ticket, &config)))
}

/* ── registration ─────────────────────────────────────────────────────── */

fn register<I, O, F, Fut>(ctx: &Arc<Ctx>, id: &'static str, description: &'static str, handler: F)
where
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    register_with(ctx, id, description, None, handler);
}

fn register_with<I, O, F, Fut>(
    ctx: &Arc<Ctx>,
    id: &'static str,
    description: &'static str,
    metadata: Option<Value>,
    handler: F,
) where
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    let captured = ctx.clone();
    let mut function =
        RegisterFunction::new_async(move |input: I| handler(captured.clone(), input))
            .description(description);
    if let Some(metadata) = metadata {
        function = function.metadata(metadata);
    }
    ctx.iii
        .register_function(id, function);
}

/// Registration metadata of the mention search function: console plumbing,
/// hidden from agents and traces like the get function that declares it.
pub fn mention_search_metadata() -> Value {
    serde_json::json!({ "internal": true, "trace_hidden": true })
}

pub fn register_functions(ctx: &Arc<Ctx>) {
    register(
        ctx,
        "kanban::config::info",
        "Resolved kanban settings: the folder the board file lives in, the columns, and the numbering prefix.",
        config_info,
    );
    register(
        ctx,
        "kanban::board::get",
        "Read the board: every configured column with its tickets, the done column, every project and release with its counts, plus the resolved paths and priorities. Narrow it with statuses, updated_since, limit and parent_id / project_id / release_id (empty string = none), and ask for compact rows to keep the response small.",
        board_get,
    );
    register(
        ctx,
        "kanban::ticket::create",
        "Create a ticket on the board and return it; pass compact to get back only its compact row (id, key, title, status, priority, assignee, comment_count). Optionally make it a subtask (parent), or file it under a project or release (id or name).",
        ticket_create,
    );
    register(
        ctx,
        "kanban::ticket::get",
        "Read one ticket with its comments and activity, its parent, its live subtasks and their progress. The id accepts the internal uuid or the human-readable key.",
        ticket_get,
    );
    register(
        ctx,
        "kanban::ticket::list",
        "List tickets, optionally narrowed to a column, an assignee, an updated_since timestamp, a parent (its subtasks), a project or a release (empty string = none); compact rows carry only id, key, title, status, priority, assignee and comment_count.",
        ticket_list,
    );
    register(
        ctx,
        "kanban::ticket::links",
        "Cross-references of one ticket: the tickets it mentions and the tickets that mention it, found by key (KAN-12) in titles, descriptions and comments. Rows are the same summaries kanban::ticket::list returns. The id accepts the internal uuid or the human-readable key.",
        ticket_links,
    );
    register(
        ctx,
        "kanban::ticket::update",
        "Update a ticket field (title, description, status, priority, assignee, labels, parent, project, release) and return the ticket (compact returns only its compact row). To change labels pass add_labels / remove_labels (they edit the current labels, so concurrent edits do not overwrite each other); labels replaces the whole set. parent makes the ticket a subtask (one level: the parent must be live and not a subtask, a ticket with subtasks cannot become one) and null detaches it; the activity is recorded on both tickets. project and release take an id or a name; null clears. A patch that changes nothing records and fires nothing.",
        ticket_update,
    );
    register(
        ctx,
        "kanban::ticket::move",
        "Move a ticket to a column (and position inside it); used by the board drag and drop. Moving a ticket to where it already is changes nothing; a deleted ticket cannot be moved. Returns the ticket, or only its compact row with compact.",
        ticket_move,
    );
    register(
        ctx,
        "kanban::ticket::delete",
        "Soft-delete a ticket: it leaves the board but stays in the board file. Deleting a deleted ticket changes nothing.",
        ticket_delete,
    );
    register(
        ctx,
        "kanban::ticket::restore",
        "Put a soft-deleted ticket back on the board. A subtask whose parent is deleted, gone or a subtask itself comes back detached (activity on both tickets, an event for each). Restoring a ticket that is not deleted changes nothing.",
        ticket_restore,
    );
    register(
        ctx,
        "kanban::comment::create",
        "Comment on a ticket, or reply to a comment by passing parent_id. Returns the new comment and the ticket (only its compact row with compact).",
        comment_create,
    );
    register(
        ctx,
        "kanban::comment::list",
        "Read a ticket's comments with filtering, mirroring the kanban:comment trigger: by author, excluding an author, roots only, or after a timestamp.",
        comment_list,
    );
    register(
        ctx,
        "kanban::activity::list",
        "Read a ticket's activity timeline: field changes, lane moves, deletion and comment events, oldest first.",
        activity_list,
    );
    register(
        ctx,
        "kanban::agent::list",
        "List assignable agent profiles, resolved through iii-directory with a fallback to the configured agents folder.",
        agent_list,
    );
    register(
        ctx,
        "kanban::project::list",
        "List projects with their live ticket counts (total, done, per column). Archived projects only with include_archived.",
        project_list,
    );
    register(
        ctx,
        "kanban::project::create",
        "Create a project: a named group of tickets with an optional lead and color. Names are unique (ASCII case ignored).",
        project_create,
    );
    register(
        ctx,
        "kanban::project::update",
        "Rename a project or change its description, lead or color (null clears lead and color). The id accepts the project id or its name.",
        project_update,
    );
    register(
        ctx,
        "kanban::project::archive",
        "Archive a project. Its tickets stay; an archived project refuses new tickets. Archiving an archived project changes nothing.",
        project_archive,
    );
    register(
        ctx,
        "kanban::project::restore",
        "Put an archived project back. Restoring a project that is not archived changes nothing.",
        project_restore,
    );
    register(
        ctx,
        "kanban::release::list",
        "List releases by target date then name, each with its derived status (planned, active, released), live ticket counts, how many tickets were reopened after shipping, and a per-project breakdown.",
        release_list,
    );
    register(
        ctx,
        "kanban::release::create",
        "Create a release with an optional target date (YYYY-MM-DD). Names are unique (ASCII case ignored). Its status is derived from its tickets.",
        release_create,
    );
    register(
        ctx,
        "kanban::release::update",
        "Rename a release or change its description or target date (null clears the date). The id accepts the release id or its name.",
        release_update,
    );
    register(
        ctx,
        "kanban::release::ship",
        "Mark a release as released. Live tickets that are not done must go somewhere: move_open_to is another unreleased release (id or name), or null to take them out of any release; it is required when there are such tickets. Returns the release and the tickets moved.",
        release_ship,
    );
    register(
        ctx,
        "kanban::release::reopen",
        "Clear a release's released date. Tickets moved out when it shipped do not come back.",
        release_reopen,
    );
    register(
        ctx,
        "kanban::release::delete",
        "Delete a release no ticket references (soft-deleted tickets included).",
        release_delete,
    );
    register(
        ctx,
        "kanban::label::convert",
        "Turn a label into a project or a release: create one (or reuse the one with that name, or pass an existing id) and file every ticket carrying the label under it. Reports which tickets were added, already there, in another project/release (kept unless replace) or deleted (skipped). Running it twice adds nothing; dry_run changes nothing.",
        label_convert,
    );
    register_with(
        ctx,
        mentions::SEARCH_FN,
        "Search live tickets for the chat @kanban mention menu, by key (KAN-12, 12) or title words; best match first.",
        Some(mention_search_metadata()),
        mention_search,
    );
    register_with(
        ctx,
        mentions::GET_FN,
        "Resolve a @kanban(id=…) chat mention to its pill, preview card and agent summary; null for an unknown id. Declares the kanban mention provider.",
        Some(mentions::provider().metadata()),
        mention_get,
    );
}
