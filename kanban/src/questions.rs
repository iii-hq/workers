//! Append-only question/answer comments. Harness entries are authoritative;
//! the board mirrors them and retains the decision log after sessions disappear.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, Capture, RegisterFunction, Source};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::board::{self, CommentResult};
use crate::events::{self, CommentEvent};
use crate::functions::{self, Ctx};
use crate::store::{Answer, Board, Comment, Question, Ticket};

pub const ON_ASK: &str = "kanban::question::on-ask";
pub const ON_SETTLED: &str = "kanban::question::on-ask-settled";
const RECONCILE_RETRY: std::time::Duration = std::time::Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub recommended: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskQuestion {
    pub header: String,
    pub question: String,
    #[serde(default)]
    pub multi_select: bool,
    #[serde(default)]
    pub options: Vec<AskOption>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AskAbout {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ticket: Option<String>,
    #[serde(default)]
    pub blocks: Vec<String>,
    #[serde(default = "scope")]
    pub class: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

fn scope() -> String {
    "scope".into()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AnswerBy {
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AnswerOutcome {
    Answered,
    Dismissed,
    Withdrawn,
    ExpiredDefault,
    NotDelivered,
}

impl AnswerOutcome {
    fn state(self) -> &'static str {
        match self {
            Self::Answered => "answered",
            Self::Dismissed => "dismissed",
            Self::Withdrawn => "withdrawn",
            Self::ExpiredDefault => "expired_default",
            Self::NotDelivered => "not_delivered",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskState {
    Open,
    Answering,
    Answered,
    Dismissed,
    Withdrawn,
    Expired,
}

/// The whole stored entry carried by both harness events and its list API.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct AskEntry {
    #[serde(flatten)]
    pub question: Question,
    pub state: AskState,
    #[serde(default)]
    pub answer_text: Option<String>,
    #[serde(default)]
    pub by: Option<AnswerBy>,
    #[serde(default)]
    pub outcome: Option<AnswerOutcome>,
    #[serde(default)]
    pub settled_at: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct AskPage {
    entries: Vec<AskEntry>,
    #[serde(default)]
    cursor: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct Delivery {
    input: AskEntry,
}

fn reply<'a>(ticket: &'a Ticket, comment: &Comment) -> Option<&'a Comment> {
    ticket
        .comments
        .iter()
        .find(|reply| reply.parent_id.as_deref() == Some(&comment.id) && reply.answer.is_some())
}

pub fn open_questions(ticket: &Ticket) -> u64 {
    ticket
        .comments
        .iter()
        .filter(|comment| comment.question.is_some() && reply(ticket, comment).is_none())
        .count() as u64
}

fn question_body(question: &Question) -> String {
    question
        .questions
        .iter()
        .map(|q| {
            let mut text = format!("⛔ Needs you — {}: {}", q.header, q.question);
            if let Some(why) = question.about.as_ref().and_then(|about| about.why.as_ref()) {
                text.push_str(&format!("\n{why}"));
            }
            for option in &q.options {
                text.push_str(&format!("\n- {}", option.label));
                if let Some(description) = option.description.as_ref().filter(|s| !s.is_empty()) {
                    text.push_str(&format!(": {description}"));
                }
                if option.recommended {
                    text.push_str(" (recommended)");
                }
            }
            text
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn append(
    ticket: &mut Ticket,
    question: Option<Question>,
    answer: Option<Answer>,
    parent_id: Option<String>,
    recorded_by: Option<String>,
    created_at: &str,
    at: &str,
) -> CommentResult {
    let (body, author) = match (&question, &answer) {
        (Some(question), _) => (
            question_body(question),
            recorded_by
                .clone()
                .unwrap_or_else(|| question.session_id.clone()),
        ),
        (_, Some(answer)) => (
            answer.answer_text.clone(),
            answer
                .by
                .as_ref()
                .map(|by| by.user.clone().unwrap_or_else(|| by.channel.clone()))
                .unwrap_or_else(|| "harness".into()),
        ),
        _ => unreachable!("question comments carry a question or an answer"),
    };
    let comment = Comment {
        id: board::new_id(),
        ticket_id: ticket.id.clone(),
        parent_id,
        body,
        author,
        created_at: created_at.to_string(),
        question,
        answer,
        recorded_by,
    };
    board::append_comment(ticket, comment, at)
}

fn timestamp(ms: i64) -> Result<String, String> {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|at| at.to_rfc3339_opts(chrono::SecondsFormat::Millis, true))
        .ok_or_else(|| format!("invalid harness question timestamp: {ms}"))
}

/// Resolve by the explicit card first, otherwise by session metadata. An
/// existing pair stays on its original card even if the session is relinked.
pub fn mirror(
    board: &mut Board,
    entry: &AskEntry,
    session_ticket: Option<&str>,
    at: &str,
) -> Result<Vec<CommentResult>, String> {
    let question = &entry.question;
    let existing = board.tickets.values().find_map(|ticket| {
        ticket
            .comments
            .iter()
            .find(|comment| {
                comment.recorded_by.is_none()
                    && comment.question.as_ref().is_some_and(|q| {
                        q.session_id == question.session_id && q.question_id == question.question_id
                    })
            })
            .map(|comment| (ticket.id.clone(), comment.id.clone()))
    });
    let (ticket_id, existing_comment) = match existing {
        Some((ticket, comment)) => (ticket, Some(comment)),
        None => {
            let reference = question
                .about
                .as_ref()
                .and_then(|about| about.ticket.as_deref())
                .or(session_ticket);
            let Some(ticket) = reference.and_then(|reference| board::find_ticket(board, reference))
            else {
                return Ok(Vec::new());
            };
            (ticket.id.clone(), None)
        }
    };
    let answer = match entry.state {
        AskState::Answered | AskState::Dismissed | AskState::Withdrawn => {
            let outcome = entry.outcome.ok_or("settled ask is missing outcome")?;
            // Withdrawal has no human answer or attribution in the harness store.
            let (answer_text, by) = if outcome == AnswerOutcome::Withdrawn {
                (
                    entry
                        .answer_text
                        .clone()
                        .unwrap_or_else(|| "Withdrawn by the asker.".into()),
                    entry.by.clone(),
                )
            } else {
                (
                    entry
                        .answer_text
                        .clone()
                        .ok_or("settled ask is missing answer_text")?,
                    Some(entry.by.clone().ok_or("settled ask is missing by")?),
                )
            };
            Some(Answer {
                answer_text,
                by,
                outcome,
            })
        }
        AskState::Open | AskState::Answering | AskState::Expired => None,
    };
    let ticket = board
        .tickets
        .get_mut(&ticket_id)
        .expect("resolved ticket exists");
    let mut added = Vec::new();
    let comment_id = match existing_comment {
        Some(id) => id,
        None => {
            let asked_at = timestamp(question.asked_at)?;
            let result = append(
                ticket,
                Some(question.clone()),
                None,
                None,
                None,
                &asked_at,
                at,
            );
            let id = result.comment.id.clone();
            added.push(result);
            id
        }
    };
    if let Some(answer) = answer
        && !ticket
            .comments
            .iter()
            .any(|c| c.parent_id.as_deref() == Some(&comment_id) && c.answer.is_some())
    {
        let settled_at = entry
            .settled_at
            .map(timestamp)
            .transpose()?
            .unwrap_or_else(|| at.to_string());
        added.push(append(
            ticket,
            None,
            Some(answer),
            Some(comment_id),
            None,
            &settled_at,
            at,
        ));
    }
    Ok(added)
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecordInput {
    /// Ticket uuid or key.
    pub ticket_id: String,
    pub questions: Vec<AskQuestion>,
    pub answer_text: String,
    /// Profile that made this material choice without asking a person.
    pub recorded_by: String,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub about: Option<AskAbout>,
}

pub fn record(
    board: &mut Board,
    input: RecordInput,
    at: &str,
) -> Result<Vec<CommentResult>, String> {
    if input.recorded_by.trim().is_empty() || input.answer_text.trim().is_empty() {
        return Err("recorded_by and answer_text are required".into());
    }
    if input.questions.is_empty()
        || input
            .questions
            .iter()
            .any(|q| q.header.trim().is_empty() || q.question.trim().is_empty())
    {
        return Err("questions need a header and question".into());
    }
    let ticket = board::find_ticket_mut(board, &input.ticket_id)
        .ok_or_else(|| format!("ticket not found: {}", input.ticket_id))?;
    if ticket.deleted_at.is_some() {
        return Err("cannot record a question on a deleted ticket".into());
    }
    let profile = input.recorded_by.trim().to_string();
    let question = Question {
        session_id: input
            .session_id
            .unwrap_or_else(|| format!("recorded:{profile}")),
        question_id: board::new_id(),
        questions: input.questions,
        about: input.about,
        asked_at: chrono::DateTime::parse_from_rfc3339(at)
            .expect("board write timestamp")
            .timestamp_millis(),
    };
    let root = append(
        ticket,
        Some(question),
        None,
        None,
        Some(profile.clone()),
        at,
        at,
    );
    let answer = append(
        ticket,
        None,
        Some(Answer {
            answer_text: input.answer_text,
            by: Some(AnswerBy {
                channel: "agent".into(),
                user: Some(profile),
            }),
            outcome: AnswerOutcome::Answered,
        }),
        Some(root.comment.id.clone()),
        None,
        at,
        at,
    );
    Ok(vec![root, answer])
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListInput {
    /// open, answered, dismissed, withdrawn, expired_default, not_delivered or recorded.
    #[serde(default)]
    pub state: Option<String>,
    /// Project id or name; empty string selects no project.
    #[serde(default)]
    pub project: Option<String>,
    /// Release id or name; empty string selects no release.
    #[serde(default)]
    pub release: Option<String>,
    /// Default 100, maximum 500.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Continue after this question comment id from the preceding page.
    #[serde(default)]
    pub cursor: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct QuestionView {
    pub comment: Comment,
    pub state: String,
    pub answer: Option<Comment>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct QuestionList {
    pub questions: Vec<QuestionView>,
    pub count: usize,
    pub cursor: Option<String>,
}

pub fn list(board: &Board, input: &ListInput) -> Result<QuestionList, String> {
    let filter = board::TicketFilter {
        project_id: input.project.clone(),
        release_id: input.release.clone(),
        include_deleted: true,
        ..Default::default()
    };
    let mut questions: Vec<QuestionView> = board::list_tickets(board, &filter)
        .into_iter()
        .flat_map(|ticket| {
            ticket
                .comments
                .iter()
                .filter(|c| c.question.is_some())
                .map(move |comment| {
                    let answer = reply(ticket, comment);
                    let state = if comment.recorded_by.is_some() {
                        "recorded"
                    } else {
                        answer
                            .and_then(|c| c.answer.as_ref())
                            .map_or("open", |answer| answer.outcome.state())
                    };
                    QuestionView {
                        comment: comment.clone(),
                        state: state.into(),
                        answer: answer.cloned(),
                    }
                })
        })
        .collect();
    questions.sort_by(|a, b| {
        b.comment
            .created_at
            .cmp(&a.comment.created_at)
            .then_with(|| b.comment.id.cmp(&a.comment.id))
    });
    let start = match &input.cursor {
        Some(cursor) => {
            questions
                .iter()
                .position(|row| &row.comment.id == cursor)
                .ok_or("invalid question cursor")?
                + 1
        }
        None => 0,
    };
    let mut rows: Vec<_> = questions
        .into_iter()
        .skip(start)
        .filter(|row| input.state.as_ref().is_none_or(|state| state == &row.state))
        .collect();
    let limit = input.limit.unwrap_or(100).clamp(1, 500);
    let more = rows.len() > limit;
    rows.truncate(limit);
    Ok(QuestionList {
        cursor: more.then(|| rows.last().expect("positive limit").comment.id.clone()),
        count: rows.len(),
        questions: rows,
    })
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AnswerInput {
    pub ticket_id: String,
    pub comment_id: String,
    /// Structured harness answers: header and choice, choices, other or skipped.
    #[serde(default)]
    pub answers: Option<Vec<StructuredAnswer>>,
    #[serde(default)]
    pub dismissed: bool,
    pub user: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct StructuredAnswer {
    pub header: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub other: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped: Option<bool>,
}

pub fn answer_request(board: &Board, input: &AnswerInput) -> Result<Value, String> {
    if input.user.trim().is_empty()
        || input.dismissed == input.answers.is_some()
        || input.answers.as_ref().is_some_and(Vec::is_empty)
    {
        return Err("user and exactly one of answers or dismissed are required".into());
    }
    let ticket = board::find_ticket(board, &input.ticket_id)
        .ok_or_else(|| format!("ticket not found: {}", input.ticket_id))?;
    let comment = ticket
        .comments
        .iter()
        .find(|c| c.id == input.comment_id)
        .ok_or("question comment not found on this ticket")?;
    let question = comment
        .question
        .as_ref()
        .ok_or("comment is not a question")?;
    if comment.recorded_by.is_some() {
        return Err("a recorded choice is not a harness question".into());
    }
    let mut request = json!({"session_id":question.session_id,"question_id":question.question_id,"by":{"channel":"kanban","user":input.user}});
    if input.dismissed {
        request["dismissed"] = json!(true);
    } else {
        request["answers"] = json!(input.answers);
    }
    Ok(request)
}

async fn call(ctx: &Ctx, function_id: &str, payload: Value) -> Result<Value, Error> {
    ctx.iii
        .call(CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
}

/// Persist a complete pair atomically, then announce each new comment in order.
async fn persist(
    ctx: &Ctx,
    run: impl FnOnce(&mut Board, &str) -> Result<Vec<CommentResult>, String>,
) -> Result<Vec<Comment>, Error> {
    let (comments, deliveries) = ctx
        .store
        .update(|board, config| {
            let at = board::next_at(board, chrono::Utc::now().timestamp_millis());
            let added = run(board, &at)?;
            let deliveries: Vec<_> = added
                .iter()
                .map(|result| {
                    let view = board::ticket_view(
                        board,
                        &result.ticket,
                        &config.done_status,
                        config.canceled_status.as_deref(),
                    );
                    let changes = functions::change_events(
                        board,
                        "comment.created",
                        std::slice::from_ref(&view),
                        Some(&result.comment),
                        &at,
                    );
                    let comment = CommentEvent {
                        event: "comment.created".into(),
                        ticket_id: view.ticket.id.clone(),
                        ticket_key: view.ticket.key.clone(),
                        ticket: view.into(),
                        comment: result.comment.clone(),
                        author: result.comment.author.clone(),
                        at: at.clone(),
                    };
                    (changes, comment)
                })
                .collect();
            Ok((
                added.into_iter().map(|result| result.comment).collect(),
                deliveries,
            ))
        })
        .await
        .map_err(Error::Handler)?;
    for (changes, comment) in deliveries {
        for change in changes {
            events::emit_change(&ctx.iii, &change).await;
        }
        events::emit_comment(&ctx.iii, &comment).await;
    }
    Ok(comments)
}

async fn mirror_entry(ctx: &Ctx, entry: AskEntry) -> Result<(), Error> {
    let board = ctx.store.read().await.map_err(Error::Handler)?;
    let already_mirrored = board.tickets.values().any(|ticket| {
        ticket.comments.iter().any(|comment| {
            comment.recorded_by.is_none()
                && comment.question.as_ref().is_some_and(|question| {
                    question.session_id == entry.question.session_id
                        && question.question_id == entry.question.question_id
                })
        })
    });
    let session_ticket = if !already_mirrored
        && entry
            .question
            .about
            .as_ref()
            .and_then(|about| about.ticket.as_ref())
            .is_none()
    {
        let session = call(
            ctx,
            "session::get",
            json!({"session_id":entry.question.session_id}),
        )
        .await?;
        session["meta"]["metadata"]["kanban_ticket"]
            .as_str()
            .map(String::from)
    } else {
        None
    };
    persist(ctx, |board, at| {
        mirror(board, &entry, session_ticket.as_deref(), at)
    })
    .await?;
    Ok(())
}

/// Boot reconciliation, retried while the harness is unavailable. Paging includes
/// closed entries so missed settlements also repair the append-only log.
pub async fn reconcile(ctx: &Ctx) -> Result<(), Error> {
    let mut cursor = None;
    loop {
        let value = call(
            ctx,
            "harness::ask::list",
            json!({"limit":100,"cursor":cursor}),
        )
        .await?;
        let page: AskPage = serde_json::from_value(value)
            .map_err(|e| Error::Handler(format!("harness ask list: {e}")))?;
        for entry in page.entries {
            mirror_entry(ctx, entry).await?;
        }
        match page.cursor {
            Some(next) if cursor.as_ref() != Some(&next) => cursor = Some(next),
            None => return Ok(()),
            Some(_) => {
                return Err(Error::Handler(
                    "harness ask list repeated its cursor".into(),
                ));
            }
        }
    }
}

pub fn register(ctx: &Arc<Ctx>) {
    for (source, target) in [
        ("harness::on-ask", ON_ASK),
        ("harness::on-ask-settled", ON_SETTLED),
    ] {
        let handler_ctx = ctx.clone();
        ctx.iii.register_function(
            target,
            RegisterFunction::new_async(move |delivery: Delivery| {
                let ctx = handler_ctx.clone();
                async move {
                    mirror_entry(&ctx, delivery.input).await?;
                    Ok::<_, Error>(json!({"ok":true}))
                }
            })
            .description("Internal: mirror a harness question or settlement onto its card.")
            .metadata(json!({"internal":true})),
        );
        // The SDK retains the observer while the source worker is unavailable.
        ctx.iii
            .observe(Source::function(source).capture([Capture::Input]), target);
    }
}

pub fn spawn_reconcile(ctx: Arc<Ctx>) {
    tokio::spawn(async move {
        loop {
            match reconcile(&ctx).await {
                Ok(()) => break,
                Err(error) => {
                    tracing::warn!(%error, "kanban question reconciliation failed; will retry")
                }
            }
            tokio::time::sleep(RECONCILE_RETRY).await;
        }
    });
}

pub async fn answer(ctx: Arc<Ctx>, input: AnswerInput) -> Result<Value, Error> {
    let board = ctx.store.read().await.map_err(Error::Handler)?;
    let request = answer_request(&board, &input).map_err(Error::Handler)?;
    // The harness reserves and closes the ask. Only its event writes the reply.
    call(&ctx, "harness::ask::answer", request).await
}

pub async fn record_question(ctx: Arc<Ctx>, input: RecordInput) -> Result<QuestionView, Error> {
    let comments = persist(&ctx, |board, at| record(board, input, at)).await?;
    Ok(QuestionView {
        comment: comments[0].clone(),
        state: "recorded".into(),
        answer: Some(comments[1].clone()),
    })
}

pub async fn list_questions(ctx: Arc<Ctx>, input: ListInput) -> Result<QuestionList, Error> {
    list(&ctx.store.read().await.map_err(Error::Handler)?, &input).map_err(Error::Handler)
}
