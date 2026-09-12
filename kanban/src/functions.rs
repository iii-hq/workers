//! The `kanban::*` function catalog: typed request/response structs and the
//! handlers that route every mutation through the store and the trigger
//! fan-out.

use std::future::Future;
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};

use crate::agents::{self, AgentList};
use crate::board::{
    self, ActivityFilter, BoardContext, CommentFilter, CreateCommentInput, CreateTicketInput,
    TicketFilter, TicketRow, UpdatePatch, row,
};
use crate::config::KanbanConfig;
use crate::config::{CONFIG_ID, Column};
use crate::events::{
    ChangeEvent, CommentEvent, SubscriberCounts, Subscribers, emit_change, emit_comment,
};
use crate::store::{Activity, Board, BoardStore, Comment, ConfigCell, Ticket, config_snapshot};

pub const FUNCTION_IDS: [&str; 13] = [
    "kanban::config::info",
    "kanban::board::get",
    "kanban::ticket::create",
    "kanban::ticket::get",
    "kanban::ticket::list",
    "kanban::ticket::update",
    "kanban::ticket::move",
    "kanban::ticket::delete",
    "kanban::ticket::restore",
    "kanban::comment::create",
    "kanban::comment::list",
    "kanban::activity::list",
    "kanban::agent::list",
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
    /// Return the compact row (key, title, status, priority, assignee, comment_count) instead of the full summary.
    #[serde(default)]
    pub compact: bool,
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
    /// Who is making the change. Defaults to `user`.
    #[serde(default)]
    pub actor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct TicketGetInput {
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
    /// Return the compact row (key, title, status, priority, assignee, comment_count) instead of the full summary.
    #[serde(default)]
    pub compact: bool,
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
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub actor: Option<String>,
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

/* ── responses ────────────────────────────────────────────────────────── */

#[derive(Debug, Serialize, JsonSchema)]
pub struct ConfigInfo {
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
    pub id_prefix: String,
    pub data_path_resolved: String,
    pub board_file: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TicketList {
    pub tickets: Vec<TicketRow>,
    pub count: u64,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CommentCreated {
    pub comment: Comment,
    pub ticket: Ticket,
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

struct Outcome<T> {
    result: T,
    ticket: Option<Ticket>,
    comment: Option<Comment>,
}

/// Run one mutation under the store lock, then fan the result out to the
/// `kanban:change` (and, for a comment, `kanban:comment`) bindings.
async fn apply<T>(
    ctx: &Ctx,
    event: &str,
    run: impl FnOnce(&mut Board, &BoardContext, &str) -> Result<Outcome<T>, String>,
) -> Result<Outcome<T>, Error> {
    let at = board::now();
    let outcome = ctx
        .store
        .update(|board, config| run(board, &BoardContext::from(config), &at))
        .await
        .map_err(Error::Handler)?;
    if let Some(ticket) = &outcome.ticket {
        emit_change(
            &ctx.iii,
            &ctx.subscribers,
            &ChangeEvent {
                event: event.to_string(),
                ticket_id: ticket.id.clone(),
                ticket_key: ticket.key.clone(),
                ticket: ticket.clone().into(),
                comment: outcome.comment.clone(),
                activity: ticket.activity.last().cloned(),
                at: at.clone(),
            },
        );
        if let Some(comment) = &outcome.comment {
            emit_comment(
                &ctx.iii,
                &ctx.subscribers,
                &CommentEvent {
                    event: "comment.created".to_string(),
                    ticket_id: ticket.id.clone(),
                    ticket_key: ticket.key.clone(),
                    ticket: ticket.clone().into(),
                    comment: comment.clone(),
                    author: comment.author.clone(),
                    at,
                },
            );
        }
    }
    Ok(outcome)
}

fn ticket_outcome(ticket: Option<Ticket>) -> Outcome<Option<Ticket>> {
    Outcome {
        result: ticket.clone(),
        ticket,
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
        configuration_id: CONFIG_ID.to_string(),
        data_path: config.data_path.clone(),
        data_path_resolved: config.data_path_resolved().display().to_string(),
        board_file: config.board_file().display().to_string(),
        project_root: crate::config::project_root().display().to_string(),
        id_prefix: config.id_prefix.clone(),
        default_status: config.default_status.clone(),
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
        limit: limit(input.limit),
        ..TicketFilter::default()
    };
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
        id_prefix: config.id_prefix.clone(),
        data_path_resolved: config.data_path_resolved().display().to_string(),
        board_file: config.board_file().display().to_string(),
        updated_at: board::now(),
    }
}

async fn board_get(ctx: Arc<Ctx>, input: BoardGetInput) -> Result<BoardView, Error> {
    let config = config_snapshot(&ctx.config);
    let board = read(&ctx).await?;
    Ok(board_view(&board, &config, &input))
}

async fn ticket_create(ctx: Arc<Ctx>, input: TicketCreateInput) -> Result<Ticket, Error> {
    let create = CreateTicketInput {
        title: input.title,
        description: input.description,
        status: input.status,
        priority: input.priority,
        assignee: input.assignee,
        labels: input.labels,
        actor: input.actor,
    };
    let outcome = apply(&ctx, "ticket.created", |board, context, at| {
        let ticket = board::create_ticket(board, &create, context, at)?;
        Ok(Outcome {
            result: ticket.clone(),
            ticket: Some(ticket),
            comment: None,
        })
    })
    .await?;
    Ok(outcome.result)
}

async fn ticket_get(ctx: Arc<Ctx>, input: TicketGetInput) -> Result<Ticket, Error> {
    let board = read(&ctx).await?;
    board::find_ticket(&board, &input.id)
        .cloned()
        .ok_or_else(|| not_found(&input.id))
}

async fn ticket_list(ctx: Arc<Ctx>, input: TicketListInput) -> Result<TicketList, Error> {
    let board = read(&ctx).await?;
    let filter = TicketFilter {
        status: input.status,
        assignee: input.assignee,
        include_deleted: input.include_deleted,
        updated_since: input.updated_since,
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

async fn ticket_update(ctx: Arc<Ctx>, input: TicketUpdateInput) -> Result<Ticket, Error> {
    let patch = UpdatePatch {
        title: input.title,
        description: input.description,
        priority: input.priority,
        assignee: input.assignee,
        status: input.status,
        labels: input.labels,
    };
    let actor = input
        .actor
        .as_deref()
        .map(str::trim)
        .filter(|actor| !actor.is_empty())
        .unwrap_or("user")
        .to_string();
    let outcome = apply(&ctx, "ticket.updated", |board, context, at| {
        let updated = board::update_ticket(board, &input.id, &patch, &actor, context, at)?;
        Ok(ticket_outcome(updated.map(|result| result.ticket)))
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.id))
}

async fn ticket_move(ctx: Arc<Ctx>, input: TicketMoveInput) -> Result<Ticket, Error> {
    let outcome = apply(&ctx, "ticket.moved", |board, context, at| {
        let moved = board::move_ticket(
            board,
            &input.id,
            &input.status,
            limit(input.index),
            input.actor.as_deref(),
            context,
            at,
        )?;
        Ok(ticket_outcome(moved))
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.id))
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

async fn ticket_delete(ctx: Arc<Ctx>, input: TicketRefInput) -> Result<Ticket, Error> {
    let actor = actor_of(&input);
    let outcome = apply(&ctx, "ticket.deleted", |board, _, at| {
        Ok(ticket_outcome(board::delete_ticket(
            board, &input.id, &actor, at,
        )))
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.id))
}

async fn ticket_restore(ctx: Arc<Ctx>, input: TicketRefInput) -> Result<Ticket, Error> {
    let actor = actor_of(&input);
    let outcome = apply(&ctx, "ticket.restored", |board, _, at| {
        Ok(ticket_outcome(board::restore_ticket(
            board, &input.id, &actor, at,
        )))
    })
    .await?;
    outcome.result.ok_or_else(|| not_found(&input.id))
}

async fn comment_create(ctx: Arc<Ctx>, input: CommentCreateInput) -> Result<CommentCreated, Error> {
    let create = CreateCommentInput {
        ticket_id: input.ticket_id.clone(),
        body: input.body,
        parent_id: input.parent_id,
        author: input.author,
    };
    let outcome = apply(&ctx, "comment.created", |board, _, at| {
        Ok(match board::create_comment(board, &create, at)? {
            Some(result) => Outcome {
                result: Some(CommentCreated {
                    comment: result.comment.clone(),
                    ticket: result.ticket.clone(),
                }),
                ticket: Some(result.ticket),
                comment: Some(result.comment),
            },
            None => Outcome {
                result: None,
                ticket: None,
                comment: None,
            },
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

/* ── registration ─────────────────────────────────────────────────────── */

fn register<I, O, F, Fut>(ctx: &Arc<Ctx>, id: &'static str, description: &'static str, handler: F)
where
    I: DeserializeOwned + JsonSchema + Send + 'static,
    O: Serialize + JsonSchema + Send + 'static,
    F: Fn(Arc<Ctx>, I) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<O, Error>> + Send + 'static,
{
    let captured = ctx.clone();
    ctx.iii.register_function(
        id,
        RegisterFunction::new_async(move |input: I| handler(captured.clone(), input))
            .description(description),
    );
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
        "Read the board: every configured column with its tickets, plus the resolved paths and priorities. Narrow it with statuses, updated_since and limit, and ask for compact rows to keep the response small.",
        board_get,
    );
    register(
        ctx,
        "kanban::ticket::create",
        "Create a ticket on the board and return it.",
        ticket_create,
    );
    register(
        ctx,
        "kanban::ticket::get",
        "Read one ticket with its comments and activity. The id accepts the internal uuid or the human-readable key.",
        ticket_get,
    );
    register(
        ctx,
        "kanban::ticket::list",
        "List tickets, optionally narrowed to a column, an assignee or an updated_since timestamp; compact rows carry only key, title, status, priority, assignee and comment_count.",
        ticket_list,
    );
    register(
        ctx,
        "kanban::ticket::update",
        "Update a ticket field (title, description, status, priority, assignee, labels) and return the ticket.",
        ticket_update,
    );
    register(
        ctx,
        "kanban::ticket::move",
        "Move a ticket to a column (and position inside it); used by the board drag and drop.",
        ticket_move,
    );
    register(
        ctx,
        "kanban::ticket::delete",
        "Soft-delete a ticket: it leaves the board but stays in the board file.",
        ticket_delete,
    );
    register(
        ctx,
        "kanban::ticket::restore",
        "Put a soft-deleted ticket back on the board.",
        ticket_restore,
    );
    register(
        ctx,
        "kanban::comment::create",
        "Comment on a ticket, or reply to a comment by passing parent_id. Returns the new comment and the ticket.",
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
}
