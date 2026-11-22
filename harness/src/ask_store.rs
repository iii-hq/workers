//! Durable human questions. Reservations and completion use the harness's
//! private CAS accessor; delivery reuses `harness::send`'s idempotency key.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::ask::{AskAbout, AskClass, AskExpiry, AskQuestion, AskRequest};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::message::AgentMessage;

pub const SCOPE: &str = "harness_ask";
pub const ANSWER_ID: &str = "harness::ask::answer";
pub const WITHDRAW_ID: &str = "harness::ask::withdraw";
pub const LIST_ID: &str = "harness::ask::list";

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Answered,
    Dismissed,
    Withdrawn,
    ExpiredDefault,
    NotDelivered,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Chat,
    Tower,
    Kanban,
    Expiry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnswerBy {
    pub channel: Channel,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Answer {
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Entry {
    pub session_id: String,
    pub question_id: String,
    pub turn_id: String,
    /// Asking agent profile id, when the session has a frozen profile.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asked_by: Option<String>,
    pub questions: Vec<AskQuestion>,
    pub about: AskAbout,
    pub asked_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_in_min: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    pub on_expiry: AskExpiry,
    pub state: AskState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answers: Option<Vec<Answer>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub by: Option<AnswerBy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<i64>,
}

impl Entry {
    fn key(&self) -> String {
        format!("{}/{}", self.session_id, self.question_id)
    }

    fn is_open(&self) -> bool {
        matches!(self.state, AskState::Open | AskState::Expired)
    }

    fn already_answered(&self) -> HarnessError {
        let channel = self.by.as_ref().map(|by| match by.channel {
            Channel::Chat => "chat",
            Channel::Tower => "tower",
            Channel::Kanban => "kanban",
            Channel::Expiry => "expiry",
        });
        HarnessError::InvalidRequest(match channel {
            Some(channel) => format!("already answered by {channel}"),
            None if self.state == AskState::Withdrawn => "question withdrawn by the asker".into(),
            None => "already answered".into(),
        })
    }
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
// Like other function inputs, tolerate metadata the engine injects into a
// worker-to-worker call (notably _caller_worker_id, on kanban's forward).
pub struct AnswerRequest {
    pub session_id: String,
    pub question_id: String,
    pub answers: Option<Vec<Answer>>,
    #[serde(default)]
    pub dismissed: bool,
    pub by: AnswerBy,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct WithdrawRequest {
    /// Stamped by the harness for an agent call; operators supply it explicitly.
    #[serde(default)]
    pub session_id: String,
    pub question_id: String,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ListRequest {
    pub session_id: Option<String>,
    pub state: Option<AskState>,
    pub limit: Option<usize>,
    /// The last key returned by the previous page (exclusive).
    pub cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ListResponse {
    pub entries: Vec<Entry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

fn invalid(message: impl Into<String>) -> HarnessError {
    HarnessError::InvalidRequest(message.into())
}

async fn get(deps: &Deps, session: &str, question: &str) -> Result<Entry, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let value =
        crate::state::state_get(&deps.iii, SCOPE, &format!("{session}/{question}"), timeout)
            .await?;
    if value.is_null() {
        return Err(invalid("ask entry not found"));
    }
    serde_json::from_value(value).map_err(|e| HarnessError::State(format!("ask entry: {e}")))
}

// ponytail: scan the private scope; index by session if retained history makes status reads expensive.
async fn all(deps: &Deps) -> Result<Vec<Entry>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let rows: Vec<Value> = crate::state::list_values(&deps.iii, SCOPE, timeout).await?;
    rows.into_iter()
        // Before durable asks, this scope held arrays of the last 32 chat
        // settlements, keyed by session. Those rows still serve old cards.
        .filter(|row| !row.is_array())
        .map(|row| {
            serde_json::from_value(row).map_err(|e| HarnessError::State(format!("ask entry: {e}")))
        })
        .collect()
}

async fn cas(deps: &Deps, before: Option<&Entry>, after: &Entry) -> Result<bool, HarnessError> {
    let expected = before
        .map(serde_json::to_value)
        .transpose()
        .map_err(|e| HarnessError::State(e.to_string()))?;
    let value = serde_json::to_value(after).map_err(|e| HarnessError::State(e.to_string()))?;
    Ok(crate::state::cas_value(
        &deps.iii,
        SCOPE,
        &after.key(),
        expected,
        value,
        deps.cfg().await.session_timeout_ms,
    )
    .await?
    .is_none())
}

pub async fn accept(
    deps: &Deps,
    session_id: &str,
    turn_id: &str,
    question_id: &str,
    req: &AskRequest,
) -> Result<(), HarnessError> {
    let metadata = deps.session().await.metadata_of(session_id).await?;
    let asked_by = metadata
        .as_ref()
        .and_then(|meta| meta.get("agent_profile"))
        .and_then(|profile| profile.get("id"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    let mut about = req.about.clone();
    if about.ticket.is_none() {
        about.ticket = metadata.as_ref().and_then(|meta| {
            meta.get("kanban_ticket")
                .and_then(Value::as_str)
                .map(str::to_owned)
        });
    }
    let asked_at = AgentMessage::now_ms();
    let expires_at = req
        .expires_in_min
        .map(|minutes| {
            minutes
                .checked_mul(60_000)
                .and_then(|ms| i64::try_from(ms).ok())
                .and_then(|ms| asked_at.checked_add(ms))
                .ok_or_else(|| invalid("expires_in_min is too large"))
        })
        .transpose()?;
    let entry = Entry {
        session_id: session_id.into(),
        question_id: question_id.into(),
        turn_id: turn_id.into(),
        asked_by,
        questions: req.questions.clone(),
        about,
        asked_at,
        expires_in_min: req.expires_in_min,
        expires_at,
        on_expiry: req.on_expiry.clone(),
        state: AskState::Open,
        answers: None,
        answer_text: None,
        by: None,
        outcome: None,
        settled_at: None,
    };
    if cas(deps, None, &entry).await? {
        arm(deps, &entry);
        deps.events.emit_ask(crate::events::ON_ASK_FN, &entry).await;
    } else {
        arm(deps, &get(deps, session_id, question_id).await?);
    }
    Ok(())
}

/// Exactly the chat card's `formatAnswer`: question/option order, trimmed
/// free text last on a multi-select, and `(skipped)` on skipped questions.
fn answer_text(entry: &Entry, answers: &[Answer]) -> Result<String, HarnessError> {
    if answers.len() != entry.questions.len() {
        return Err(invalid(
            "answers must have one entry per question in question order",
        ));
    }
    entry
        .questions
        .iter()
        .zip(answers)
        .map(|(q, a)| {
            if a.header != q.header {
                return Err(invalid(format!("answer header must be {:?}", q.header)));
            }
            let other = a.other.as_deref().unwrap_or("").trim();
            let picks = a.choices.as_deref().unwrap_or_default();
            if a.skipped == Some(true) {
                if a.choice.is_some() || a.choices.is_some() || a.other.is_some() {
                    return Err(invalid("skipped cannot be combined with an answer"));
                }
                return Ok(format!("{}: (skipped)", q.header));
            }
            if a.choice.is_some() && (q.multi_select || a.choices.is_some() || !other.is_empty()) {
                return Err(invalid(
                    "choice is only for a single-select answer without other",
                ));
            }
            if !q.multi_select && a.choices.is_some() {
                return Err(invalid("choices is only for a multi-select question"));
            }
            let labels: Vec<&str> = a
                .choice
                .iter()
                .map(String::as_str)
                .chain(picks.iter().map(String::as_str))
                .collect();
            if labels
                .iter()
                .any(|label| !q.options.iter().any(|o| o.label == *label))
                || labels
                    .iter()
                    .enumerate()
                    .any(|(i, label)| labels[..i].contains(label))
            {
                return Err(invalid(
                    "answer choices must be distinct existing option labels",
                ));
            }
            let mut parts: Vec<&str> = q
                .options
                .iter()
                .filter(|o| labels.contains(&o.label.as_str()))
                .map(|o| o.label.as_str())
                .collect();
            if !other.is_empty() {
                parts.push(other);
            }
            if parts.is_empty() {
                return Err(invalid("each question needs an answer or skipped: true"));
            }
            Ok(format!("{}: {}", q.header, parts.join(", ")))
        })
        .collect::<Result<Vec<_>, _>>()
        .map(|lines| lines.join("\n"))
}

const DISMISS_TEXT: &str = "I dismissed your questions without answering them. Go on with your best judgement; ask again later only if you really must.";

fn reserve(entry: &Entry, req: AnswerRequest) -> Result<Entry, HarnessError> {
    if !entry.is_open() {
        return Err(entry.already_answered());
    }
    if req.dismissed == req.answers.is_some() {
        return Err(invalid("provide answers or dismissed: true, exclusively"));
    }
    if req.dismissed
        && !matches!(
            entry.about.class,
            AskClass::Scope | AskClass::Routine | AskClass::RunHealth
        )
    {
        return Err(invalid(
            "dismissed is only allowed for scope, routine or run_health questions",
        ));
    }
    let text = if req.dismissed {
        DISMISS_TEXT.into()
    } else {
        answer_text(entry, req.answers.as_deref().unwrap_or_default())?
    };
    Ok(Entry {
        state: AskState::Answering,
        answers: req.answers,
        answer_text: Some(text),
        by: Some(req.by),
        outcome: Some(if req.dismissed {
            Outcome::Dismissed
        } else {
            Outcome::Answered
        }),
        ..entry.clone()
    })
}

pub async fn answer(deps: &Deps, req: AnswerRequest) -> Result<Entry, HarnessError> {
    let before = get(deps, &req.session_id, &req.question_id).await?;
    let next = reserve(&before, req)?;
    if !cas(deps, Some(&before), &next).await? {
        return Err(get(deps, &before.session_id, &before.question_id)
            .await?
            .already_answered());
    }
    deps.ask_timers
        .cancel(&format!("ask-expiry:{}", next.key()));
    deliver(deps, next).await
}

/// An `answering` entry has everything needed for crash replay. A missing
/// outcome denotes the block-expiry notice, which reopens as `expired`.
async fn deliver(deps: &Deps, entry: Entry) -> Result<Entry, HarnessError> {
    let result = deliver_once(deps, entry.clone()).await;
    if result.is_err() {
        retry(deps, &entry);
    }
    result
}

async fn deliver_once(deps: &Deps, entry: Entry) -> Result<Entry, HarnessError> {
    let block = entry.outcome.is_none();
    let mut exists = deps.session().await.exists(&entry.session_id).await?;
    if exists {
        let sent = deps.iii.call(iii_sdk::CallRequest {
            function_id: crate::functions::SEND_ID.into(),
            payload: json!({ "session_id": entry.session_id, "message": entry.answer_text,
                "idempotency_key": format!("{}:{}", if block { "ask-expiry" } else { "ask" }, entry.key()),
                "options": { "require_existing_session": true } }),
            action: None, timeout_ms: Some(deps.cfg().await.session_timeout_ms),
        }).await;
        if let Err(error) = sent {
            exists = deps.session().await.exists(&entry.session_id).await?;
            if exists {
                return Err(HarnessError::Dependency(format!(
                    "harness::send ask: {error}"
                )));
            }
        }
    }
    let mut closed = entry.clone();
    closed.state = if block {
        AskState::Expired
    } else if entry.outcome == Some(Outcome::Dismissed) {
        AskState::Dismissed
    } else {
        AskState::Answered
    };
    if !exists {
        closed.outcome = Some(Outcome::NotDelivered);
    }
    if !block {
        closed.settled_at = Some(AgentMessage::now_ms());
    }
    if cas(deps, Some(&entry), &closed).await? {
        if !block {
            deps.events
                .emit_ask(crate::events::ON_ASK_SETTLED_FN, &closed)
                .await;
        }
        Ok(closed)
    } else {
        get(deps, &entry.session_id, &entry.question_id).await
    }
}

pub async fn withdraw(deps: &Deps, req: WithdrawRequest) -> Result<Entry, HarnessError> {
    let before = get(deps, &req.session_id, &req.question_id).await?;
    if !before.is_open() {
        return Err(before.already_answered());
    }
    let next = Entry {
        state: AskState::Withdrawn,
        answers: None,
        answer_text: None,
        by: None,
        outcome: Some(Outcome::Withdrawn),
        settled_at: Some(AgentMessage::now_ms()),
        ..before.clone()
    };
    if !cas(deps, Some(&before), &next).await? {
        return Err(get(deps, &req.session_id, &req.question_id)
            .await?
            .already_answered());
    }
    deps.ask_timers
        .cancel(&format!("ask-expiry:{}", next.key()));
    deps.events
        .emit_ask(crate::events::ON_ASK_SETTLED_FN, &next)
        .await;
    Ok(next)
}

pub async fn list(deps: &Deps, req: ListRequest) -> Result<ListResponse, HarnessError> {
    let limit = req.limit.unwrap_or(100);
    if !(1..=200).contains(&limit) {
        return Err(invalid("limit must be 1-200"));
    }
    let mut entries: Vec<_> = all(deps)
        .await?
        .into_iter()
        .filter(|entry| {
            req.session_id
                .as_ref()
                .is_none_or(|s| *s == entry.session_id)
                && req.state.is_none_or(|state| state == entry.state)
                && req
                    .cursor
                    .as_ref()
                    .is_none_or(|cursor| entry.key() > *cursor)
        })
        .collect();
    entries.sort_by_key(Entry::key);
    let more = entries.len() > limit;
    entries.truncate(limit);
    let cursor = more.then(|| entries.last().unwrap().key());
    Ok(ListResponse { entries, cursor })
}

pub async fn open_count(deps: &Deps, session_id: &str) -> Result<usize, HarnessError> {
    Ok(all(deps)
        .await?
        .iter()
        .filter(|entry| entry.session_id == session_id && entry.is_open())
        .count())
}

pub async fn purge(deps: &Deps, session_id: &str) -> Result<(), HarnessError> {
    for entry in all(deps)
        .await?
        .into_iter()
        .filter(|entry| entry.session_id == session_id)
    {
        deps.ask_timers
            .cancel(&format!("ask-expiry:{}", entry.key()));
        deps.ask_timers
            .cancel(&format!("ask-retry:{}", entry.key()));
        // Retry a racing reserve/close so session deletion never leaves a row.
        loop {
            let value = crate::state::state_get(
                &deps.iii,
                SCOPE,
                &entry.key(),
                deps.cfg().await.session_timeout_ms,
            )
            .await?;
            if value.is_null()
                || crate::state::cas_value(
                    &deps.iii,
                    SCOPE,
                    &entry.key(),
                    Some(value),
                    Value::Null,
                    deps.cfg().await.session_timeout_ms,
                )
                .await?
                .is_none()
            {
                break;
            }
        }
    }
    Ok(())
}

fn expiry_reservation(entry: &Entry) -> Result<Entry, HarnessError> {
    let minutes = entry
        .expires_in_min
        .ok_or_else(|| invalid("ask has no expiry"))?;
    match &entry.on_expiry {
        AskExpiry::Answer { answer } => {
            let mut next = reserve(
                entry,
                AnswerRequest {
                    session_id: entry.session_id.clone(),
                    question_id: entry.question_id.clone(),
                    answers: Some(vec![Answer {
                        header: entry.questions[0].header.clone(),
                        choice: Some(answer.clone()),
                        choices: None,
                        other: None,
                        skipped: None,
                    }]),
                    dismissed: false,
                    by: AnswerBy {
                        channel: Channel::Expiry,
                        user: None,
                    },
                },
            )?;
            next.outcome = Some(Outcome::ExpiredDefault);
            next.answer_text.as_mut().unwrap().push_str(&format!(
                " (no answer within {minutes} min; default applied)"
            ));
            Ok(next)
        }
        AskExpiry::Block(_) => Ok(Entry {
            state: AskState::Answering,
            outcome: None,
            by: Some(AnswerBy {
                channel: Channel::Expiry,
                user: None,
            }),
            answer_text: Some(format!(
                "No answer within {minutes} min to: {}. Do not decide it yourself. If you cannot go on without it, finish blocked; the answer will arrive as a message.",
                entry
                    .questions
                    .iter()
                    .map(|q| q.header.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )),
            ..entry.clone()
        }),
    }
}

async fn expire(deps: &Deps, session: &str, question: &str, due: i64) -> Result<(), HarnessError> {
    let entry = get(deps, session, question).await?;
    if entry.state != AskState::Open || entry.expires_at.is_none_or(|at| at > due) {
        return Ok(());
    }
    let next = expiry_reservation(&entry)?;
    if cas(deps, Some(&entry), &next).await? {
        deliver_once(deps, next).await?;
    }
    Ok(())
}

fn retry(deps: &Deps, entry: &Entry) {
    let key = format!("ask-retry:{}", entry.key());
    if deps.ask_timers.is_armed(&key) {
        return;
    }
    let worker = deps.clone();
    let entry = entry.clone();
    deps.ask_timers
        .arm(key, AgentMessage::now_ms() + 1_000, async move {
            for attempt in 0..5 {
                let current = match get(&worker, &entry.session_id, &entry.question_id).await {
                    Ok(current) => current,
                    Err(HarnessError::InvalidRequest(_)) => return, // purged
                    Err(error) => {
                        tracing::warn!(%error, "ask retry read failed");
                        if attempt < 4 {
                            tokio::time::sleep(std::time::Duration::from_secs(2 << attempt)).await;
                        }
                        continue;
                    }
                };
                let result = if current.state == AskState::Answering {
                    deliver_once(&worker, current).await.map(|_| ())
                } else if current.state == AskState::Open
                    && let Some(due) = current.expires_at
                {
                    expire(&worker, &current.session_id, &current.question_id, due).await
                } else {
                    return;
                };
                match result {
                    Ok(()) => return,
                    Err(error) => tracing::warn!(%error, "ask delivery retry failed"),
                }
                if attempt < 4 {
                    tokio::time::sleep(std::time::Duration::from_secs(2 << attempt)).await;
                }
            }
        });
}

fn arm(deps: &Deps, entry: &Entry) {
    if entry.state != AskState::Open {
        return;
    }
    let Some(due) = entry.expires_at else {
        return;
    };
    if deps
        .ask_timers
        .is_armed(&format!("ask-expiry:{}", entry.key()))
    {
        return;
    }
    // ponytail: in-process deadlines, re-armed from state at boot; use a
    // durable scheduler if expiry must run while every harness is down.
    let worker = deps.clone();
    let entry = entry.clone();
    deps.ask_timers
        .arm(format!("ask-expiry:{}", entry.key()), due, async move {
            if let Err(error) = expire(&worker, &entry.session_id, &entry.question_id, due).await {
                tracing::warn!(%error, "ask expiry failed; retrying");
                retry(&worker, &entry);
            }
        });
}

pub async fn recover(deps: &Deps) -> Result<(), HarnessError> {
    for entry in all(deps).await? {
        if !deps.session().await.exists(&entry.session_id).await? {
            purge(deps, &entry.session_id).await?;
        } else if entry.state == AskState::Answering {
            deliver(deps, entry).await?;
        } else {
            arm(deps, &entry);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
