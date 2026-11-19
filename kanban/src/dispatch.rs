//! Board dispatch: start the agent assigned to a To do card, without a chat.
//!
//! Opt-in through the `dispatch` rules of the kanban configuration, which only
//! a person edits (agents are denied `configuration::set`). A rule names an
//! assignee (an agent profile id such as `deepwork`), the model its sessions
//! run on, and the projects whose cards it starts, each with the repository
//! root the session works in and how many of its cards may run at once.
//!
//! After every board write, a top-level card in `todo` whose assignee is the
//! rule's, in a mapped project, moved there (or assigned, or created) by one
//! of the rule's `actors`, is claimed (moved to `in_progress` with the
//! `starting` pill) and started with `harness::send`. A card beyond the
//! project's `max_running` waits in To do with the `queued` pill and starts
//! when a running one leaves In progress. A card carrying `dw:run` (an earlier
//! deepwork run) is resumed instead of started anew.
//!
//! The session id is `dispatch-<key>-<claim seq>`: the claim's activity
//! sequence number makes a re-send after a crash land on the same session
//! (`idempotency_key`), and lets a late event of an older session be told
//! apart. When a dispatched session's turn ends for good (`terminal`) while
//! its card is still In progress, the card goes to Backlog with `unfinished`
//! and the `needs you` pill: nothing is resumed on its own.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Capture, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{self, UpdatePatch};
use crate::functions::{self, CommentCreateInput, Ctx, Outcome};
use crate::store::{Board, Pill, Ticket, config_snapshot};

pub const ACTOR: &str = "kanban-dispatch";
pub const SESSION_PREFIX: &str = "dispatch-";
pub const ON_TURN_COMPLETED_FN: &str = "kanban::dispatch::on-turn-completed";
const TURN_COMPLETED_SOURCE: &str = "harness::on-turn-completed";
const TODO: &str = "todo";
const IN_PROGRESS: &str = "in_progress";
const BACKLOG: &str = "backlog";
const RUN_LABEL: &str = "dw:run";
const SEND_TIMEOUT_MS: u64 = 30_000;
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(300);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DispatchProject {
    /// Project id or name.
    pub project: String,
    /// Absolute path of the repository the session works in.
    pub root: String,
    /// Cards of this rule in this project that may be In progress at once.
    #[serde(default = "one")]
    pub max_running: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DispatchRule {
    /// The card assignee that starts a session, e.g. `deepwork`.
    pub assignee: String,
    /// Agent profile of the session; the assignee when omitted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
    /// Model of the session (`<provider>/<model>`); the deepwork profile pins none.
    pub model: String,
    /// Extra `args:` for the session's first message, e.g. `finish=pr`.
    #[serde(default)]
    pub args: String,
    /// Activity actors whose move, assignment or create starts a card.
    #[serde(default = "default_actors")]
    pub actors: Vec<String>,
    /// Hard input+output token budget of the whole session tree.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_total_tokens: Option<u64>,
    #[serde(default)]
    pub projects: Vec<DispatchProject>,
}

fn one() -> u32 {
    1
}

fn default_actors() -> Vec<String> {
    vec!["user".to_string()]
}

impl DispatchRule {
    pub fn agent(&self) -> &str {
        self.agent.as_deref().unwrap_or(&self.assignee)
    }
}

/// Keep the rules a dispatch can run: a trimmed assignee and model, and
/// projects with an absolute root. Hostile input is dropped, never fatal.
pub fn normalize(raw: Option<&Value>) -> Vec<DispatchRule> {
    let Some(entries) = raw.and_then(Value::as_array) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|entry| serde_json::from_value::<DispatchRule>(entry.clone()).ok())
        .filter_map(|mut rule| {
            rule.assignee = rule.assignee.trim().to_string();
            rule.model = rule.model.trim().to_string();
            rule.args = rule.args.trim().to_string();
            rule.agent = rule
                .agent
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty());
            rule.actors = rule
                .actors
                .into_iter()
                .map(|a| a.trim().to_string())
                .filter(|a| !a.is_empty())
                .collect();
            rule.max_total_tokens = rule.max_total_tokens.filter(|n| *n > 0);
            rule.projects.retain_mut(|p| {
                p.project = p.project.trim().to_string();
                p.root = p.root.trim().to_string();
                p.max_running = p.max_running.max(1);
                !p.project.is_empty() && std::path::Path::new(&p.root).is_absolute()
            });
            (!rule.assignee.is_empty() && !rule.model.is_empty() && !rule.projects.is_empty())
                .then_some(rule)
        })
        .collect()
}

pub fn schema() -> Value {
    json!({
        "type": "array",
        "description": "Start the agent assigned to a To do card with harness::send, without a chat. Empty: off.",
        "items": {
            "type": "object",
            "additionalProperties": false,
            "required": ["assignee", "model", "projects"],
            "properties": {
                "assignee": { "type": "string", "minLength": 1, "description": "Card assignee that starts a session, e.g. deepwork." },
                "agent": { "type": "string", "description": "Agent profile of the session; the assignee when omitted." },
                "model": { "type": "string", "minLength": 1, "description": "Model of the session, e.g. claude-code/claude-sonnet-5-5." },
                "args": { "type": "string", "description": "Extra args: for the first message, e.g. finish=pr." },
                "actors": { "type": "array", "items": { "type": "string" }, "description": "Activity actors whose move, assignment or create starts a card. Default: [\"user\"]." },
                "max_total_tokens": { "type": "integer", "minimum": 1, "description": "Hard input+output token budget of the whole session tree." },
                "projects": {
                    "type": "array",
                    "minItems": 1,
                    "items": {
                        "type": "object",
                        "additionalProperties": false,
                        "required": ["project", "root"],
                        "properties": {
                            "project": { "type": "string", "minLength": 1, "description": "Project id or name." },
                            "root": { "type": "string", "minLength": 1, "description": "Absolute path of the repository the session works in." },
                            "max_running": { "type": "integer", "minimum": 1, "description": "Cards of this rule in this project In progress at once. Default 1." }
                        }
                    }
                }
            }
        }
    })
}

/* ── what to do: pure, read from the board ───────────────────────────── */

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Claim the card and start (or resume) its session.
    Start {
        key: String,
        rule: usize,
        project_id: String,
        root: String,
        resume: bool,
    },
    /// The card waits for a free slot: show `queued`.
    Queue { key: String },
    /// A `queued` card that no rule starts any more: clear the pill.
    Unqueue { key: String },
}

pub fn queued_pill() -> Pill {
    Pill {
        icon: "clock".into(),
        text: "queued".into(),
    }
}

pub fn starting_pill() -> Pill {
    Pill {
        icon: "loader".into(),
        text: "starting".into(),
    }
}

pub fn needs_you_pill() -> Pill {
    Pill {
        icon: "hand".into(),
        text: "needs you".into(),
    }
}

fn live(ticket: &Ticket) -> bool {
    ticket.deleted_at.is_none()
}

fn assigned(ticket: &Ticket, assignee: &str) -> bool {
    ticket.assignee.as_deref().map(str::trim) == Some(assignee)
}

/// A card is the rule's when it is assigned to the rule's assignee, or to its
/// agent (deepwork rewrites its run ticket's assignee to `deepwork`).
fn assigned_to_rule(ticket: &Ticket, rule: &DispatchRule) -> bool {
    assigned(ticket, &rule.assignee) || assigned(ticket, rule.agent())
}

/// The actor of the write that made the card eligible: its newest create, or
/// change of `status` to `todo`, or of `assignee` to `assignee`.
pub fn trigger_actor<'a>(ticket: &'a Ticket, assignee: &str) -> &'a str {
    ticket
        .activity
        .iter()
        .rev()
        .find(|entry| {
            entry.kind == "ticket.created"
                || entry.changes.iter().flatten().any(|change| {
                    (change.field == "status" && change.to == json!(TODO))
                        || (change.field == "assignee" && change.to == json!(assignee))
                })
        })
        .map(|entry| entry.actor.as_str())
        .unwrap_or(ticket.created_by.as_str())
}

/// The sequence number of the dispatcher's live claim of this card: its
/// newest status change, when that was the dispatcher moving it In progress.
/// A later move by anyone (a person taking it back by hand, the run finishing)
/// ends the claim.
pub fn claim_seq(ticket: &Ticket) -> Option<u64> {
    let last_move = ticket.activity.iter().rev().find(|entry| {
        entry
            .changes
            .iter()
            .flatten()
            .any(|change| change.field == "status")
    })?;
    let claimed = last_move.actor == ACTOR
        && last_move
            .changes
            .iter()
            .flatten()
            .any(|change| change.field == "status" && change.to == json!(IN_PROGRESS));
    claimed.then_some(last_move.seq)
}

/// The session a deepwork run ticket names as its orchestrator, when it does.
fn orchestrator_session(ticket: &Ticket) -> Option<&str> {
    ticket
        .description
        .lines()
        .filter_map(|line| line.trim().strip_prefix("Orchestrator session:"))
        .map(str::trim)
        .next_back()
}

/// Whether `session` still runs this card under the dispatcher's claim: the
/// claim is live with its seq, and no other session took the run over (a
/// resume from the chat rewrites `Orchestrator session:` first).
fn runs_card(ticket: &Ticket, session: &str, seq: u64) -> bool {
    live(ticket)
        && ticket.status == IN_PROGRESS
        && claim_seq(ticket) == Some(seq)
        && orchestrator_session(ticket).is_none_or(|owner| owner == session)
}

pub fn session_id(key: &str, seq: u64) -> String {
    format!("{SESSION_PREFIX}{}-{seq}", key.to_ascii_lowercase())
}

/// `dispatch-kan-12-7` → (`KAN-12`, 7).
pub fn parse_session_id(session_id: &str) -> Option<(String, u64)> {
    let rest = session_id.strip_prefix(SESSION_PREFIX)?;
    let (key, seq) = rest.rsplit_once('-')?;
    let seq = seq.parse().ok()?;
    (!key.is_empty()).then(|| (key.to_ascii_uppercase(), seq))
}

fn resolve_project<'a>(board: &'a Board, reference: &str) -> Option<&'a str> {
    board
        .projects
        .values()
        .filter(|project| project.archived_at.is_none())
        .find(|project| {
            project.id.eq_ignore_ascii_case(reference)
                || project.name.trim().eq_ignore_ascii_case(reference)
        })
        .map(|project| project.id.as_str())
}

/// Every action the rules call for on this board, in board order.
pub fn plan(board: &Board, rules: &[DispatchRule]) -> Vec<Action> {
    let mut actions = Vec::new();
    let mut handled: Vec<&str> = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        // project id → (root, max_running)
        let mapped: Vec<(&str, &DispatchProject)> = rule
            .projects
            .iter()
            .filter_map(|p| resolve_project(board, &p.project).map(|id| (id, p)))
            .collect();
        let root_of = |project_id: Option<&str>| {
            project_id.and_then(|id| mapped.iter().find(|(pid, _)| *pid == id).map(|(_, p)| *p))
        };
        let top_level = |ticket: &&Ticket| live(ticket) && ticket.parent_id.is_none();
        let mut running: Vec<(&str, u32)> = Vec::new();
        for ticket in board.tickets.values().filter(top_level) {
            if ticket.status == IN_PROGRESS
                && assigned_to_rule(ticket, rule)
                && let Some(project) = root_of(ticket.project_id.as_deref())
            {
                match running.iter_mut().find(|(root, _)| *root == project.root) {
                    Some((_, count)) => *count += 1,
                    None => running.push((project.root.as_str(), 1)),
                }
            }
        }
        let mut queue: Vec<&Ticket> = board
            .tickets
            .values()
            .filter(top_level)
            .filter(|t| t.status == TODO && assigned_to_rule(t, rule))
            .filter(|t| root_of(t.project_id.as_deref()).is_some())
            .filter(|t| {
                let actor = match t.assignee.as_deref().map(str::trim) {
                    Some(assignee) => trigger_actor(t, assignee),
                    None => t.created_by.as_str(),
                };
                rule.actors.iter().any(|a| a == actor)
            })
            .collect();
        queue.sort_by(|a, b| board::by_order(a, b));
        for ticket in queue {
            if handled.contains(&ticket.id.as_str()) {
                continue;
            }
            handled.push(&ticket.id);
            let project = root_of(ticket.project_id.as_deref()).expect("filtered above");
            let count = running
                .iter()
                .find(|(root, _)| *root == project.root)
                .map(|(_, n)| *n)
                .unwrap_or(0);
            if count < project.max_running {
                match running.iter_mut().find(|(root, _)| *root == project.root) {
                    Some((_, n)) => *n += 1,
                    None => running.push((project.root.as_str(), 1)),
                }
                actions.push(Action::Start {
                    key: ticket.key.clone(),
                    rule: index,
                    project_id: ticket.project_id.clone().unwrap_or_default(),
                    root: project.root.clone(),
                    resume: ticket.labels.iter().any(|l| l == RUN_LABEL),
                });
            } else if !ticket.pills.contains(&queued_pill()) {
                actions.push(Action::Queue {
                    key: ticket.key.clone(),
                });
            }
        }
    }
    for ticket in board
        .tickets
        .values()
        .filter(|t| live(t) && t.status == TODO)
    {
        if ticket.pills.contains(&queued_pill()) && !handled.contains(&ticket.id.as_str()) {
            actions.push(Action::Unqueue {
                key: ticket.key.clone(),
            });
        }
    }
    actions
}

/// Whether a write that changed these tickets can change what the rules do:
/// a top-level card of a rule's assignee, or a card waiting `queued`. Every
/// other write (comments and subtasks of a run, cards of other assignees)
/// skips the board read a reconcile costs.
pub fn relevant<'a>(rules: &[DispatchRule], mut tickets: impl Iterator<Item = &'a Ticket>) -> bool {
    !rules.is_empty()
        && tickets.any(|ticket| {
            ticket.pills.contains(&queued_pill())
                || (ticket.parent_id.is_none()
                    && rules.iter().any(|rule| assigned_to_rule(ticket, rule)))
        })
}

/// The first message of a dispatched session. The ticket key first, then the
/// `args:` line deepwork reads; `ask=board` because nobody watches this
/// session in a chat.
pub fn first_message(key: &str, project_id: &str, resume: bool, extra: &str) -> String {
    let mut args = Vec::new();
    if resume {
        args.push(format!("resume={key}"));
    }
    if !project_id.is_empty() {
        args.push(format!("project={project_id}"));
    }
    args.push("ask=board".to_string());
    if !extra.is_empty() {
        args.push(extra.to_string());
    }
    format!("{key}\nargs: {}", args.join(" "))
}

pub fn send_request(
    rule: &DispatchRule,
    ticket: &Ticket,
    project_id: &str,
    root: &str,
    resume: bool,
    seq: u64,
) -> Value {
    let session = session_id(&ticket.key, seq);
    let title: String = format!("{} · {}", ticket.key, ticket.title)
        .chars()
        .take(80)
        .collect();
    let mut options = json!({
        "agent": rule.agent(),
        "functions": { "allow": ["*"] },
        "metadata": { "fs_scope": { "root": root } },
    });
    if let Some(tokens) = rule.max_total_tokens {
        options["max_total_tokens"] = json!(tokens);
    }
    json!({
        "session_id": session,
        "idempotency_key": session,
        "message": first_message(&ticket.key, project_id, resume, &rule.args),
        "model": rule.model,
        "session": { "title": title, "kind": "automation", "metadata": { "kanban_ticket": ticket.key } },
        "options": options,
    })
}

/// What a final turn of a dispatched session means for its card: a reason
/// when the card is still In progress under that same claim.
pub fn stopped_reason(board: &Board, event: &Value) -> Option<(String, String)> {
    if event.get("terminal").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let session = event.get("session_id")?.as_str()?;
    let (key, seq) = parse_session_id(session)?;
    let ticket = board::find_ticket(board, &key)?;
    if !runs_card(ticket, session, seq) {
        return None;
    }
    let status = event
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or("ended");
    let detail = ["result_error", "reason"]
        .iter()
        .find_map(|field| event.get(*field).filter(|v| !v.is_null()))
        .map(|v| match v {
            Value::String(s) => s.clone(),
            other => other.to_string(),
        });
    let reason = match (status, detail) {
        ("completed", None) => "ended its run".to_string(),
        (status, None) => status.to_string(),
        (status, Some(detail)) => {
            format!("{status}: {}", detail.chars().take(300).collect::<String>())
        }
    };
    Some((ticket.key.clone(), format!("session {session} {reason}")))
}

/* ── doing it ─────────────────────────────────────────────────────────── */

/// Coalesces reconcile passes: one runs at a time, and a write during a pass
/// asks for one more.
#[derive(Default)]
pub struct Dispatcher {
    ctx: OnceLock<Weak<Ctx>>,
    running: AtomicBool,
    dirty: AtomicBool,
}

impl Dispatcher {
    pub fn bind(&self, ctx: &Arc<Ctx>) {
        let _ = self.ctx.set(Arc::downgrade(ctx));
    }

    /// Called after every board write. Cheap when no rule is configured.
    pub fn poke(&self) {
        let Some(ctx) = self.ctx.get().and_then(Weak::upgrade) else {
            return;
        };
        if config_snapshot(&ctx.config).dispatch.is_empty() {
            return;
        }
        self.dirty.store(true, Ordering::SeqCst);
        if self.running.swap(true, Ordering::SeqCst) {
            return;
        }
        tokio::spawn(async move {
            let me = &ctx.dispatch;
            loop {
                me.dirty.store(false, Ordering::SeqCst);
                reconcile(&ctx).await;
                if me.dirty.load(Ordering::SeqCst) {
                    continue;
                }
                me.running.store(false, Ordering::SeqCst);
                if !(me.dirty.load(Ordering::SeqCst) && !me.running.swap(true, Ordering::SeqCst)) {
                    break;
                }
            }
        });
    }
}

async fn reconcile(ctx: &Arc<Ctx>) {
    let rules = config_snapshot(&ctx.config).dispatch;
    let board = match ctx.store.read().await {
        Ok(board) => board,
        Err(error) => {
            tracing::warn!(%error, "dispatch: board read failed");
            return;
        }
    };
    let in_todo = |t: &Ticket| live(t) && t.status == TODO;
    for action in plan(&board, &rules) {
        match action {
            Action::Queue { key } => {
                let _ = update(ctx, &key, in_todo, pills(vec![queued_pill()])).await;
            }
            Action::Unqueue { key } => {
                let _ = update(ctx, &key, in_todo, pills(Vec::new())).await;
            }
            Action::Start {
                key,
                rule,
                project_id,
                root,
                resume,
            } => {
                start(ctx, &rules[rule], &key, &project_id, &root, resume).await;
            }
        }
    }
}

fn pills(pills: Vec<Pill>) -> UpdatePatch {
    UpdatePatch {
        pills: Some(pills),
        ..UpdatePatch::default()
    }
}

/// One ticket update by the dispatcher, applied under the store lock only
/// while `guard` still holds for the ticket as it is then (a plan is a
/// snapshot, and a send can take seconds). Returns the ticket as written.
async fn update(
    ctx: &Ctx,
    key: &str,
    guard: impl Fn(&Ticket) -> bool,
    patch: UpdatePatch,
) -> Option<Ticket> {
    let result = functions::apply(ctx, "ticket.updated", |board, context, at| {
        let before = board::find_ticket(board, key).cloned();
        if !before.as_ref().is_some_and(&guard) {
            return Ok(Outcome::silent(None));
        }
        let updated = board::update_ticket(board, key, &patch, ACTOR, context, at)?;
        let related = updated
            .as_ref()
            .map(|r| r.related.clone())
            .unwrap_or_default();
        Ok(functions::ticket_outcome(
            board,
            context,
            before,
            updated.map(|r| r.ticket),
            &related,
        ))
    })
    .await;
    match result {
        Ok(outcome) => outcome.result.map(|view| view.ticket),
        Err(error) => {
            tracing::warn!(key, %error, "dispatch: ticket update failed");
            None
        }
    }
}

async fn comment(ctx: &Arc<Ctx>, key: &str, body: String) {
    let input = CommentCreateInput {
        ticket_id: key.to_string(),
        body,
        parent_id: None,
        author: Some(ACTOR.to_string()),
        compact: true,
    };
    if let Err(error) = functions::comment_create(ctx.clone(), input).await {
        tracing::warn!(key, %error, "dispatch: comment failed");
    }
}

async fn start(
    ctx: &Arc<Ctx>,
    rule: &DispatchRule,
    key: &str,
    project_id: &str,
    root: &str,
    resume: bool,
) {
    // The labels the dispatcher parks a card with go when it starts again.
    let claim = UpdatePatch {
        status: Some(IN_PROGRESS.to_string()),
        pills: Some(vec![starting_pill()]),
        remove_labels: Some(vec!["blocked".to_string(), "unfinished".to_string()]),
        ..UpdatePatch::default()
    };
    // Still what the plan saw: in To do, top-level, the rule's, in its project.
    let eligible = |t: &Ticket| {
        live(t)
            && t.status == TODO
            && t.parent_id.is_none()
            && assigned_to_rule(t, rule)
            && t.project_id.as_deref() == Some(project_id)
    };
    let Some(ticket) = update(ctx, key, eligible, claim).await else {
        return; // moved, reassigned or gone meanwhile
    };
    send(ctx, rule, &ticket, project_id, root, resume).await;
}

async fn harness_status(ctx: &Ctx, session: &str) -> Result<Value, Error> {
    ctx.iii
        .call(CallRequest {
            function_id: "harness::status".into(),
            payload: json!({ "session_id": session }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
}

/// Whether a `harness::status` answer is a session still working or waiting.
fn session_alive(status: &Value) -> bool {
    matches!(
        status.get("status").and_then(Value::as_str),
        Some("running" | "awaiting_functions")
    ) || status.get("expects_wake").and_then(Value::as_bool) == Some(true)
}

/// Send the claimed card's first message. A failed call is checked against
/// the harness first: a send that timed out may still have started the
/// session, and parking its card would let a second session start on it.
async fn send(
    ctx: &Arc<Ctx>,
    rule: &DispatchRule,
    ticket: &Ticket,
    project_id: &str,
    root: &str,
    resume: bool,
) {
    let Some(seq) = claim_seq(ticket) else {
        return;
    };
    let session = session_id(&ticket.key, seq);
    let payload = send_request(rule, ticket, project_id, root, resume, seq);
    let request = CallRequest {
        function_id: "harness::send".into(),
        payload,
        action: None,
        timeout_ms: Some(SEND_TIMEOUT_MS),
    };
    let error = match ctx.iii.call(request).await {
        Ok(_) => None,
        Err(error) => match harness_status(ctx, &session).await {
            Ok(status) if !status.is_null() => None, // it started after all
            Ok(_) => Some(error.to_string()),
            Err(_) => {
                // The harness does not answer: keep the claim; the sweep sends again.
                tracing::warn!(key = %ticket.key, %error, "dispatch: send unconfirmed, retried by the sweep");
                return;
            }
        },
    };
    let claimed = |t: &Ticket| runs_card(t, &session, seq);
    match error {
        None => {
            let started = UpdatePatch {
                pills: Some(Vec::new()),
                ..UpdatePatch::default()
            };
            // Clear `starting` unless the run already set its own pills.
            let _ = update(
                ctx,
                &ticket.key,
                |t| claimed(t) && t.pills == vec![starting_pill()],
                started,
            )
            .await;
            let verb = if resume { "Resumed" } else { "Started" };
            comment(
                ctx,
                &ticket.key,
                format!("▶ {verb} {} — session {session} in {root}.", rule.agent()),
            )
            .await;
        }
        Some(error) => {
            let park = UpdatePatch {
                status: Some(BACKLOG.to_string()),
                add_labels: Some(vec!["blocked".to_string()]),
                pills: Some(vec![needs_you_pill()]),
                ..UpdatePatch::default()
            };
            if update(ctx, &ticket.key, claimed, park).await.is_some() {
                comment(
                    ctx,
                    &ticket.key,
                    format!(
                        "⛔ Needs you — couldn't start {}: {error}. Fix the cause, then move this card to To do.",
                        rule.agent()
                    ),
                )
                .await;
            }
        }
    }
}

/// Park a card whose session ended, re-checked under the lock: the claim and
/// the session must still be the ones that ended.
async fn park_stopped(ctx: &Arc<Ctx>, key: &str, session: &str, seq: u64, reason: &str) {
    let park = UpdatePatch {
        status: Some(BACKLOG.to_string()),
        add_labels: Some(vec!["unfinished".to_string()]),
        pills: Some(vec![needs_you_pill()]),
        ..UpdatePatch::default()
    };
    if update(ctx, key, |t| runs_card(t, session, seq), park)
        .await
        .is_some()
    {
        comment(
            ctx,
            key,
            format!("⛔ Needs you — {reason}, with this card still In progress. Move it to To do to resume."),
        )
        .await;
    }
}

/// The fields of `harness::on-turn-completed` the dispatcher reads.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct TurnCompleted {
    pub session_id: String,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub terminal: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// What the observer delivers (`capture: input`): the event as `input`.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TurnCompletedDelivery {
    #[serde(default)]
    pub input: Option<TurnCompleted>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

/// Park the card of a dispatched session whose final turn ended.
async fn on_final_turn(ctx: &Arc<Ctx>, event: &Value) {
    let Some((_, seq)) = event
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(parse_session_id)
    else {
        return; // a child of a dispatched session, not the session itself
    };
    let board = match ctx.store.read().await {
        Ok(board) => board,
        Err(error) => {
            tracing::warn!(%error, "dispatch: board read failed; the sweep checks this session again");
            return;
        }
    };
    if let Some((key, reason)) = stopped_reason(&board, event) {
        let session = event["session_id"].as_str().unwrap_or_default();
        park_stopped(ctx, &key, session, seq, &reason).await;
    }
}

/// Register the final-turn sink, observe `harness::on-turn-completed` for
/// dispatched sessions (the observer stays pending while no harness runs),
/// and start the sweep.
pub fn register(ctx: &Arc<Ctx>) {
    ctx.dispatch.bind(ctx);
    let handler_ctx = ctx.clone();
    ctx.iii.register_function(
        ON_TURN_COMPLETED_FN,
        RegisterFunction::new_async(move |delivery: TurnCompletedDelivery| {
            let ctx = handler_ctx.clone();
            async move {
                if let Some(event) = delivery.input
                    && let Ok(event) = serde_json::to_value(event)
                {
                    on_final_turn(&ctx, &event).await;
                }
                Ok::<Ack, Error>(Ack { ok: true })
            }
        })
        .description(
            "Internal: park a dispatched card whose session ended while the card was In progress.",
        )
        .metadata(json!({ "internal": true })),
    );
    ctx.iii.observe(
        Source::function(TURN_COMPLETED_SOURCE)
            .filters(json!({
                "input.session_id": { "$prefix": SESSION_PREFIX },
                "input.terminal": { "$eq": true },
            }))
            .capture([Capture::Input]),
        ON_TURN_COMPLETED_FN,
    );
}

/// Every few minutes, and first at boot: the safety net under the
/// turn-completed event, which a restart, a harness that was down or a failed
/// write can miss. ponytail: a fixed 5-minute poll of the dispatched cards
/// still In progress (a handful); a harness event log would make it exact.
pub fn spawn_sweep(ctx: Arc<Ctx>) {
    tokio::spawn(async move {
        loop {
            sweep(&ctx).await;
            tokio::time::sleep(SWEEP_EVERY).await;
        }
    });
}

/// For each card the dispatcher still claims: a claim whose session the
/// harness never saw is sent again (same session id and idempotency key, so a
/// repeat is harmless); a session that ended parks its card; a live one that
/// still shows `starting` loses the pill. Then reconcile.
async fn sweep(ctx: &Arc<Ctx>) {
    let config = config_snapshot(&ctx.config);
    if config.dispatch.is_empty() {
        return;
    }
    let Ok(board) = ctx.store.read().await else {
        return;
    };
    for ticket in board.tickets.values() {
        let Some(seq) = claim_seq(ticket) else {
            continue;
        };
        let session = session_id(&ticket.key, seq);
        if !runs_card(ticket, &session, seq) {
            continue;
        }
        let Ok(status) = harness_status(ctx, &session).await else {
            continue; // the harness does not answer: the next sweep decides
        };
        if status.is_null() {
            if ticket.pills.contains(&starting_pill()) {
                if let Some((rule, project_id, root)) =
                    plan_for_claimed(&board, &config.dispatch, ticket)
                {
                    let resume = ticket.labels.iter().any(|l| l == RUN_LABEL);
                    send(
                        ctx,
                        &config.dispatch[rule],
                        ticket,
                        &project_id,
                        &root,
                        resume,
                    )
                    .await;
                }
            } else {
                let event = json!({ "session_id": session, "terminal": true, "status": "unknown to the harness" });
                on_final_turn(ctx, &event).await;
            }
        } else if !session_alive(&status) {
            let mut event = status.clone();
            event["session_id"] = json!(session);
            event["terminal"] = json!(true);
            on_final_turn(ctx, &event).await;
        } else if ticket.pills == vec![starting_pill()] {
            let started = UpdatePatch {
                pills: Some(Vec::new()),
                ..UpdatePatch::default()
            };
            let _ = update(
                ctx,
                &ticket.key,
                |t| runs_card(t, &session, seq) && t.pills == vec![starting_pill()],
                started,
            )
            .await;
        }
    }
    ctx.dispatch.poke();
}

/// The rule, project and root of a card the dispatcher claimed.
fn plan_for_claimed(
    board: &Board,
    rules: &[DispatchRule],
    ticket: &Ticket,
) -> Option<(usize, String, String)> {
    rules.iter().enumerate().find_map(|(index, rule)| {
        if !assigned_to_rule(ticket, rule) {
            return None;
        }
        rule.projects.iter().find_map(|p| {
            let id = resolve_project(board, &p.project)?;
            (ticket.project_id.as_deref() == Some(id))
                .then(|| (index, id.to_string(), p.root.clone()))
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Activity, Change, Project};

    fn project(id: &str, name: &str) -> Project {
        Project {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            lead: None,
            color: None,
            source_label: None,
            created_at: "t".into(),
            updated_at: "t".into(),
            archived_at: None,
        }
    }

    fn activity(seq: u64, actor: &str, kind: &str, changes: Vec<(&str, Value)>) -> Activity {
        Activity {
            id: format!("a{seq}"),
            ticket_id: "x".into(),
            seq,
            kind: kind.into(),
            actor: actor.into(),
            at: "t".into(),
            changes: Some(
                changes
                    .into_iter()
                    .map(|(field, to)| Change {
                        field: field.into(),
                        from: Value::Null,
                        to,
                    })
                    .collect(),
            ),
            comment_id: None,
        }
    }

    fn ticket(
        key: &str,
        order: u64,
        status: &str,
        assignee: &str,
        project: Option<&str>,
        actor: &str,
    ) -> Ticket {
        Ticket {
            id: format!("id-{key}"),
            key: key.into(),
            title: format!("title {key}"),
            description: String::new(),
            status: status.into(),
            priority: "medium".into(),
            assignee: Some(assignee.into()),
            labels: Vec::new(),
            pills: Vec::new(),
            parent_id: None,
            project_id: project.map(str::to_string),
            release_id: None,
            order,
            created_at: "t".into(),
            updated_at: "t".into(),
            deleted_at: None,
            created_by: actor.into(),
            comments: Vec::new(),
            activity: vec![activity(
                1,
                actor,
                "ticket.created",
                vec![("status", json!(status))],
            )],
        }
    }

    fn rule(max_running: u32) -> DispatchRule {
        DispatchRule {
            assignee: "deepwork".into(),
            agent: None,
            model: "claude-code/claude-sonnet-5-5".into(),
            args: String::new(),
            actors: default_actors(),
            max_total_tokens: Some(3_000_000),
            projects: vec![DispatchProject {
                project: "Tests".into(),
                root: "/repo".into(),
                max_running,
            }],
        }
    }

    fn board(tickets: Vec<Ticket>) -> Board {
        let mut board = Board::empty();
        board.projects.insert("p1".into(), project("p1", "Tests"));
        board.projects.insert("p2".into(), project("p2", "Other"));
        for t in tickets {
            board.tickets.insert(t.id.clone(), t);
        }
        board
    }

    fn keys(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .map(|a| match a {
                Action::Start { key, resume, .. } => {
                    format!("start {key}{}", if *resume { " resume" } else { "" })
                }
                Action::Queue { key } => format!("queue {key}"),
                Action::Unqueue { key } => format!("unqueue {key}"),
            })
            .collect()
    }

    #[test]
    fn starts_eligible_cards_and_queues_past_the_limit() {
        let b = board(vec![
            ticket("KAN-2", 2, "todo", "deepwork", Some("p1"), "user"),
            ticket("KAN-1", 1, "todo", "deepwork", Some("p1"), "user"),
        ]);
        assert_eq!(
            keys(&plan(&b, &[rule(1)])),
            vec!["start KAN-1", "queue KAN-2"]
        );
        assert_eq!(
            keys(&plan(&b, &[rule(2)])),
            vec!["start KAN-1", "start KAN-2"]
        );
    }

    #[test]
    fn a_running_card_takes_the_slot() {
        let b = board(vec![
            ticket("KAN-1", 1, "in_progress", "deepwork", Some("p1"), "user"),
            ticket("KAN-2", 2, "todo", "deepwork", Some("p1"), "user"),
        ]);
        assert_eq!(keys(&plan(&b, &[rule(1)])), vec!["queue KAN-2"]);
    }

    #[test]
    fn skips_subtasks_other_assignees_unmapped_projects_and_untrusted_actors() {
        let mut sub = ticket("KAN-3", 3, "todo", "deepwork", Some("p1"), "user");
        sub.parent_id = Some("id-KAN-9".into());
        let b = board(vec![
            sub,
            ticket("KAN-4", 4, "todo", "deepwork-impl", Some("p1"), "user"),
            ticket("KAN-5", 5, "todo", "deepwork", Some("p2"), "user"),
            ticket("KAN-6", 6, "todo", "deepwork", None, "user"),
            ticket("KAN-7", 7, "todo", "deepwork", Some("p1"), "some-agent"),
        ]);
        assert!(plan(&b, &[rule(5)]).is_empty());
    }

    #[test]
    fn the_newest_eligibility_write_decides_the_actor() {
        let mut t = ticket("KAN-1", 1, "backlog", "deepwork", Some("p1"), "some-agent");
        t.status = "todo".into();
        t.activity.push(activity(
            2,
            "user",
            "ticket.moved",
            vec![("status", json!("todo"))],
        ));
        let b = board(vec![t.clone()]);
        assert_eq!(keys(&plan(&b, &[rule(1)])), vec!["start KAN-1"]);
        // A later comment by someone else does not change who moved it.
        t.activity
            .push(activity(3, "other", "comment.created", vec![]));
        assert_eq!(trigger_actor(&t, "deepwork"), "user");
        // An agent moving it back to To do does not start it.
        t.activity.push(activity(
            4,
            "deepwork",
            "ticket.updated",
            vec![("status", json!("todo"))],
        ));
        assert!(plan(&board(vec![t]), &[rule(1)]).is_empty());
    }

    #[test]
    fn a_run_ticket_resumes_and_a_stale_queued_pill_clears() {
        let mut run = ticket("KAN-1", 1, "todo", "deepwork", Some("p1"), "user");
        run.labels.push("dw:run".into());
        let mut stale = ticket("KAN-2", 2, "todo", "someone", Some("p1"), "user");
        stale.pills.push(queued_pill());
        let b = board(vec![run, stale]);
        assert_eq!(
            keys(&plan(&b, &[rule(1)])),
            vec!["start KAN-1 resume", "unqueue KAN-2"]
        );
    }

    #[test]
    fn an_already_queued_card_is_not_rewritten() {
        let running = ticket("KAN-1", 1, "in_progress", "deepwork", Some("p1"), "user");
        let mut waiting = ticket("KAN-2", 2, "todo", "deepwork", Some("p1"), "user");
        waiting.pills.push(queued_pill());
        assert!(plan(&board(vec![running, waiting]), &[rule(1)]).is_empty());
    }

    #[test]
    fn only_writes_to_rule_cards_wake_the_dispatcher() {
        let rules = [rule(1)];
        let card = ticket("KAN-1", 1, "todo", "deepwork", Some("p1"), "user");
        let mut sub = ticket("KAN-2", 2, "todo", "deepwork-impl", Some("p1"), "user");
        sub.parent_id = Some("id-KAN-1".into());
        let other = ticket("KAN-3", 3, "todo", "someone", Some("p1"), "user");
        let mut queued = other.clone();
        queued.pills.push(queued_pill());
        assert!(relevant(&rules, [&card].into_iter()));
        assert!(!relevant(&rules, [&sub, &other].into_iter()));
        assert!(relevant(&rules, [&queued].into_iter()));
        assert!(!relevant(&[], [&card].into_iter()));
    }

    #[test]
    fn a_later_move_ends_the_claim_and_a_new_owner_keeps_the_card() {
        let mut t = ticket("KAN-3", 1, "in_progress", "deepwork", Some("p1"), "user");
        t.activity.push(activity(
            2,
            ACTOR,
            "ticket.updated",
            vec![("status", json!("in_progress"))],
        ));
        assert_eq!(claim_seq(&t), Some(2));
        assert!(runs_card(&t, "dispatch-kan-3-2", 2));
        t.description = "Goal: x\n## Run details\nOrchestrator session: console-abc".into();
        assert!(
            !runs_card(&t, "dispatch-kan-3-2", 2),
            "a chat resume took the run over"
        );
        t.description = "Orchestrator session: dispatch-kan-3-2".into();
        assert!(runs_card(&t, "dispatch-kan-3-2", 2));
        t.activity.push(activity(
            3,
            "user",
            "ticket.moved",
            vec![("status", json!("backlog"))],
        ));
        t.activity.push(activity(
            4,
            "user",
            "ticket.moved",
            vec![("status", json!("in_progress"))],
        ));
        assert_eq!(claim_seq(&t), None, "a person moved it back by hand");
        let event =
            json!({ "session_id": "dispatch-kan-3-2", "terminal": true, "status": "failed" });
        assert_eq!(stopped_reason(&board(vec![t]), &event), None);
    }

    #[test]
    fn a_rule_owns_the_cards_assigned_to_its_agent() {
        let mut r = rule(1);
        r.assignee = "dw-sonnet".into();
        r.agent = Some("deepwork".into());
        // The run ticket deepwork rewrote to `deepwork` still holds the slot.
        let running = ticket("KAN-1", 1, "in_progress", "deepwork", Some("p1"), "user");
        let waiting = ticket("KAN-2", 2, "todo", "dw-sonnet", Some("p1"), "user");
        assert_eq!(
            keys(&plan(&board(vec![running, waiting]), &[r.clone()])),
            vec!["queue KAN-2"]
        );
        // And dragging it back to To do still dispatches it.
        let mut back = ticket("KAN-1", 1, "todo", "deepwork", Some("p1"), "user");
        back.labels.push("dw:run".into());
        assert_eq!(
            keys(&plan(&board(vec![back]), &[r])),
            vec!["start KAN-1 resume"]
        );
    }

    #[test]
    fn session_alive_reads_running_and_parked() {
        assert!(session_alive(&json!({ "status": "running" })));
        assert!(session_alive(
            &json!({ "status": "completed", "expects_wake": true })
        ));
        assert!(!session_alive(
            &json!({ "status": "completed", "expects_wake": false })
        ));
        assert!(!session_alive(&json!({ "status": "failed" })));
    }

    #[test]
    fn session_ids_round_trip() {
        assert_eq!(session_id("KAN-12", 7), "dispatch-kan-12-7");
        assert_eq!(
            parse_session_id("dispatch-kan-12-7"),
            Some(("KAN-12".into(), 7))
        );
        assert_eq!(parse_session_id("console-1"), None);
        assert_eq!(parse_session_id("dispatch-kan-x"), None);
    }

    #[test]
    fn first_message_carries_the_deepwork_args() {
        assert_eq!(
            first_message("KAN-3", "p1", false, "finish=pr"),
            "KAN-3\nargs: project=p1 ask=board finish=pr"
        );
        assert_eq!(
            first_message("KAN-3", "p1", true, ""),
            "KAN-3\nargs: resume=KAN-3 project=p1 ask=board"
        );
    }

    #[test]
    fn send_request_pins_root_budget_and_idempotency() {
        let t = ticket("KAN-3", 1, "in_progress", "deepwork", Some("p1"), "user");
        let req = send_request(&rule(1), &t, "p1", "/repo", false, 4);
        assert_eq!(req["session_id"], "dispatch-kan-3-4");
        assert_eq!(req["idempotency_key"], "dispatch-kan-3-4");
        assert_eq!(req["options"]["metadata"]["fs_scope"]["root"], "/repo");
        assert_eq!(req["options"]["max_total_tokens"], 3_000_000);
        assert_eq!(req["options"]["agent"], "deepwork");
        assert_eq!(req["session"]["kind"], "automation");
    }

    #[test]
    fn a_final_turn_parks_only_its_own_claim() {
        let mut t = ticket("KAN-3", 1, "in_progress", "deepwork", Some("p1"), "user");
        t.activity.push(activity(
            2,
            ACTOR,
            "ticket.updated",
            vec![("status", json!("in_progress"))],
        ));
        let b = board(vec![t]);
        let ended = |session: &str, terminal: bool| json!({ "session_id": session, "terminal": terminal, "status": "failed", "result_error": "harness.budget_exceeded" });
        let (key, reason) = stopped_reason(&b, &ended("dispatch-kan-3-2", true)).unwrap();
        assert_eq!(key, "KAN-3");
        assert!(
            reason.contains("failed: harness.budget_exceeded"),
            "{reason}"
        );
        assert_eq!(stopped_reason(&b, &ended("dispatch-kan-3-2", false)), None);
        assert_eq!(
            stopped_reason(&b, &ended("dispatch-kan-3-1", true)),
            None,
            "an older claim"
        );
        let completed =
            json!({ "session_id": "dispatch-kan-3-2", "terminal": true, "status": "completed" });
        assert!(
            stopped_reason(&b, &completed)
                .unwrap()
                .1
                .ends_with("ended its run")
        );
    }

    #[test]
    fn normalize_drops_unusable_rules_and_projects() {
        let raw = json!([
            { "assignee": " deepwork ", "model": "m", "projects": [
                { "project": "Tests", "root": "/repo" },
                { "project": "Bad", "root": "relative/path" },
                { "project": "", "root": "/x" }
            ]},
            { "assignee": "", "model": "m", "projects": [{ "project": "T", "root": "/r" }] },
            { "assignee": "a", "model": "m", "projects": [] },
            "garbage"
        ]);
        let rules = normalize(Some(&raw));
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].assignee, "deepwork");
        assert_eq!(rules[0].projects.len(), 1);
        assert_eq!(rules[0].projects[0].max_running, 1);
        assert_eq!(rules[0].actors, vec!["user"]);
    }
}
