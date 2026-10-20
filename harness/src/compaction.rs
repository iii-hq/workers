//! Explicit conversation compaction: the `/compact` command sent through
//! `harness::send` by any client, and `harness::compact` for callers that
//! wait for the outcome (the console's model switch). Not to be confused with
//! [`crate::turn_compaction`], which slims finished turn records.
//!
//! The harness owns it so the command means the same thing from every client
//! (the mobile apps sent `/compact` to the model as a prompt) and every device
//! watching the session learns about it, not just the one that asked:
//!
//! - a `notice` row "compacting the conversation…" lands in the transcript
//!   when it starts — under the sender's `e_idem_` id, so its optimistic
//!   `/compact` row reconciles into it;
//! - a `/compact` send shows the session `working` / "compacting context"
//!   while it runs (unless a live turn owns the status line);
//! - it ends with one row: the `compaction` record, or a `notice` saying why
//!   there is none;
//! - `harness::compaction` fires on start and on completion;
//! - the `harness_context` snapshot is rewritten for the compacted window, so
//!   the `context` chip and `harness::status` stop showing the old one.
//!
//! The work runs on the server whatever happens to the requester's
//! connection. It reads the window exactly as the next turn assembles it
//! (`compaction_anchor` + `window::build`), never cuts a live turn's
//! unanswered call away from its result, and runs under the session's
//! context-manager lease, so it is safe beside a running turn.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::context::{CompactOutput, CompactParams, CountTokensParams};
use crate::clients::session::LoadedEntry;
use crate::clients::SessionClient;
use crate::context_snapshot::{self, SnapshotMessagesV1};
use crate::deps::Deps;
use crate::ids;
use crate::types::content::ContentBlock;
use crate::types::message::AgentMessage;

pub const COMMAND: &str = "/compact";
/// The session status reason while a compaction runs.
pub const STATUS_REASON: &str = "compacting context";

const NOTICE: &str = "notice";
const RECORD: &str = "compaction";
const STARTED: &str = "compacting the conversation…";
const ALREADY_RUNNING: &str =
    "compaction skipped — this conversation is already being compacted; its result will appear here.";
const LEASE_BUSY: &str = "compaction skipped — another compaction of this conversation is in progress; try /compact again when it finishes.";
const NOTHING_TO_COMPACT: &str =
    "nothing to compact yet — the conversation still fits word for word.";

/// A `/compact` command and its guidance for the summariser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub instructions: Option<String>,
}

/// `text` as the command: `/compact`, optionally followed by guidance. `None`
/// for prose (`/compaction`, or a `/compact` that does not lead the text).
pub fn parse(text: &str) -> Option<Command> {
    let rest = text.trim().strip_prefix(COMMAND)?;
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return None;
    }
    let instructions = rest.trim();
    Some(Command {
        instructions: (!instructions.is_empty()).then(|| instructions.to_string()),
    })
}

/// A user message that is the command: text blocks only — an attached file
/// or picture makes it a prompt.
pub fn parse_message(message: &AgentMessage) -> Option<Command> {
    let AgentMessage::User(user) = message else {
        return None;
    };
    if user
        .content
        .iter()
        .any(|block| !matches!(block, ContentBlock::Text { .. }))
    {
        return None;
    }
    parse(&ContentBlock::join_text(&user.content))
}

/// Sessions with a compaction running in this process. The status
/// projections consult it so a read of `harness::status` never flips the
/// session from "compacting context" back to `done` mid-run.
#[derive(Clone, Default)]
pub struct Compactions {
    running: Arc<Mutex<HashMap<String, String>>>,
}

impl Compactions {
    /// Claim `session_id` for compaction `id`; `None` when one already runs.
    pub fn begin(&self, session_id: &str, id: &str) -> Option<Running> {
        let mut running = self.lock();
        if running.contains_key(session_id) {
            return None;
        }
        running.insert(session_id.to_string(), id.to_string());
        Some(Running {
            compactions: self.clone(),
            session_id: session_id.to_string(),
        })
    }

    pub fn is_running(&self, session_id: &str) -> bool {
        self.lock().contains_key(session_id)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, String>> {
        self.running.lock().unwrap_or_else(|p| p.into_inner())
    }
}

/// A claimed session, released on drop (also when the task panics).
pub struct Running {
    compactions: Compactions,
    session_id: String,
}

impl Drop for Running {
    fn drop(&mut self) {
        self.compactions.lock().remove(&self.session_id);
    }
}

/// One compaction to run.
#[derive(Debug, Clone)]
pub struct Request {
    pub session_id: String,
    /// The model that writes the summary (router-resolved limits).
    pub model: String,
    pub provider: Option<String>,
    pub instructions: Option<String>,
    /// The entry the start notice takes: the sender's `e_idem_` id, so its
    /// optimistic `/compact` row becomes the notice on every device.
    pub entry_id: Option<String>,
    /// Show the session as `working` / "compacting context" while it runs.
    /// A caller that waits for the outcome behind its own progress UI (the
    /// console's model switch) leaves the status alone: a `working` session
    /// reads as busy there and would void the switch it compacts for.
    pub mark_working: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Summarised; the `compaction` record anchors every later turn.
    Ok,
    /// Another compaction of the session was running.
    Busy,
    /// Nothing old enough to summarise.
    Empty,
    /// The summariser failed or the result could not be saved.
    Failed,
}

/// How an explicit compaction ended — also what `harness::compaction`
/// carries on completion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outcome {
    pub session_id: String,
    pub compaction_id: String,
    pub status: Status,
    /// The outcome as every client shows it.
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens_after: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// The transcript rows it wrote, in order: the start notice (absent when
    /// it never started) and the outcome row.
    pub entry_ids: Vec<String>,
}

/// A summary `context::compact` produced, ready to persist.
#[derive(Debug, Clone, PartialEq)]
struct Summary {
    text: String,
    tail_start_entry_id: Option<String>,
    tokens_before: u64,
    tokens_after: Option<u64>,
    /// The anchor's summary this one updates.
    previous: Option<String>,
    /// The last entry the window was read up to.
    read_up_to: Option<String>,
}

/// What the summarisation step produced.
#[derive(Debug, Clone, PartialEq)]
enum Summarised {
    Done(Summary),
    Busy,
    Empty,
    Failed(String),
}

/// Run one compaction to completion. Never fails: whatever goes wrong becomes
/// the outcome every client is told about.
pub async fn run(deps: &Deps, req: Request, compaction_id: String) -> Outcome {
    let session = deps.session().await;
    let outcome_entry = ids::compaction_outcome_entry_id(&compaction_id);
    let Some(running) = deps.compactions.begin(&req.session_id, &compaction_id) else {
        // The sender's own row (if any) carries the answer.
        let entry = req.entry_id.clone().unwrap_or(outcome_entry);
        let outcome = outcome(&req, &compaction_id, Status::Busy, ALREADY_RUNNING.into());
        return finish(deps, &session, outcome, Vec::new(), entry).await;
    };
    let started_entry = req
        .entry_id
        .clone()
        .unwrap_or_else(|| ids::compaction_started_entry_id(&compaction_id));
    if let Err(error) = session
        .append_custom(
            &req.session_id,
            NOTICE,
            notice_data(&compaction_id, "compacting", STARTED, "info"),
            &started_entry,
            Some(&origin(&compaction_id)),
        )
        .await
    {
        drop(running);
        let message = failed(&format!("could not write to the conversation: {error}"));
        let outcome = outcome(&req, &compaction_id, Status::Failed, message);
        deps.events
            .emit_compaction(&req.session_id, event_payload("completed", &outcome))
            .await;
        return outcome;
    }

    let cfg = deps.cfg().await;
    // Whether a turn is live decides which unanswered calls must stay in the
    // tail; without knowing, a cut could orphan a result.
    let live_turn =
        match crate::state::get_turn_unhydrated(&deps.iii, &req.session_id, cfg.session_timeout_ms)
            .await
        {
            Ok(record) => record
                .filter(|record| !record.status.is_terminal())
                .map(|record| record.turn_id),
            Err(error) => {
                drop(running);
                let message = failed(&format!("could not read the session's turn: {error}"));
                let outcome = outcome(&req, &compaction_id, Status::Failed, message);
                return finish(deps, &session, outcome, vec![started_entry], outcome_entry).await;
            }
        };
    // A live turn owns the status line (its own phases); the notice and the
    // event still reach every client.
    let owns_status = req.mark_working
        && live_turn.is_none()
        && session
            .set_status(&req.session_id, "working", Some(STATUS_REASON))
            .await
            .is_ok();
    let started = outcome(&req, &compaction_id, Status::Ok, STARTED.into());
    deps.events
        .emit_compaction(&req.session_id, event_payload("started", &started))
        .await;

    let summarised = summarise(deps, &session, &req, live_turn.as_deref()).await;
    let outcome = persist(
        deps,
        &session,
        &req,
        &compaction_id,
        &outcome_entry,
        summarised,
    )
    .await;
    drop(running);
    if owns_status {
        if let Err(error) = crate::session_status::repair(deps, &req.session_id).await {
            tracing::warn!(session_id = %req.session_id, %error, "session status restore after compaction failed");
        }
    }
    if outcome.status != Status::Ok {
        return finish(deps, &session, outcome, vec![started_entry], outcome_entry).await;
    }
    let outcome = Outcome {
        entry_ids: vec![started_entry, outcome_entry],
        ..outcome
    };
    deps.events
        .emit_compaction(&req.session_id, event_payload("completed", &outcome))
        .await;
    outcome
}

/// Write the notice for an outcome that has no `compaction` record, then
/// tell the live subscribers.
async fn finish(
    deps: &Deps,
    session: &SessionClient,
    mut outcome: Outcome,
    mut written: Vec<String>,
    entry: String,
) -> Outcome {
    let tone = if outcome.status == Status::Empty {
        "info"
    } else {
        "warn"
    };
    let reason = match outcome.status {
        Status::Ok => "compacted",
        Status::Busy => "compaction_busy",
        Status::Empty => "compaction_empty",
        Status::Failed => "compaction_failed",
    };
    let mut data = notice_data(&outcome.compaction_id, reason, &outcome.message, tone);
    data["status"] = json!(outcome.status);
    match session
        .append_custom(
            &outcome.session_id,
            NOTICE,
            data,
            &entry,
            Some(&origin(&outcome.compaction_id)),
        )
        .await
    {
        Ok(()) => written.push(entry),
        Err(error) => {
            tracing::warn!(session_id = %outcome.session_id, %error, "compaction outcome notice not written")
        }
    }
    outcome.entry_ids = written;
    deps.events
        .emit_compaction(&outcome.session_id, event_payload("completed", &outcome))
        .await;
    outcome
}

/// Summarise the head of the window the next turn would assemble.
async fn summarise(
    deps: &Deps,
    session: &SessionClient,
    req: &Request,
    live_turn: Option<&str>,
) -> Summarised {
    let entries = match session.messages(&req.session_id, true).await {
        Ok(entries) => entries,
        Err(error) => {
            return Summarised::Failed(format!("could not read the conversation: {error}"))
        }
    };
    let anchor = crate::turn_loop::compaction_anchor(&req.session_id, &entries);
    let candidate = crate::window::build(&entries, anchor.window_start, None, None).candidate;
    let settled = settled_prefix(&candidate, live_turn);
    if settled == 0 {
        return Summarised::Empty;
    }
    let messages = candidate[..settled]
        .iter()
        .map(|(_, message)| serde_json::to_value(message).unwrap_or(Value::Null))
        .collect();
    let response = deps
        .context()
        .await
        .compact(CompactParams {
            messages,
            model_id: req.model.clone(),
            provider: req.provider.clone(),
            previous_summary: anchor.summary.clone(),
            instructions: req.instructions.clone(),
            lease_key: req.session_id.clone(),
        })
        .await;
    match response {
        Ok(CompactOutput::Ok {
            summary,
            tail_start_index,
            tokens_before,
            tokens_after,
        }) => Summarised::Done(Summary {
            text: summary,
            tail_start_entry_id: tail_entry(&candidate, settled, tail_start_index),
            tokens_before,
            tokens_after,
            previous: anchor.summary,
            read_up_to: entries.last().map(|entry| entry.entry_id.clone()),
        }),
        Ok(CompactOutput::Busy) => Summarised::Busy,
        Ok(CompactOutput::Empty) => Summarised::Empty,
        Ok(CompactOutput::Overflow) => Summarised::Failed(format!(
            "the summariser could not produce a summary with {}; try again, or switch to a model with a larger context window",
            req.model
        )),
        Err(error) => Summarised::Failed(error),
    }
}

/// Persist a summary as the `compaction` record (and refresh the context
/// snapshot), or turn the other results into their outcome.
async fn persist(
    deps: &Deps,
    session: &SessionClient,
    req: &Request,
    compaction_id: &str,
    entry: &str,
    summarised: Summarised,
) -> Outcome {
    let summary = match summarised {
        Summarised::Done(summary) => summary,
        Summarised::Busy => return outcome(req, compaction_id, Status::Busy, LEASE_BUSY.into()),
        Summarised::Empty => {
            return outcome(req, compaction_id, Status::Empty, NOTHING_TO_COMPACT.into())
        }
        Summarised::Failed(reason) => {
            return outcome(req, compaction_id, Status::Failed, failed(&reason))
        }
    };
    // Everything read was summarised: a turn that appended meanwhile must
    // stay in the window, so the tail opens at its first entry.
    let tail_start_entry_id = match &summary.tail_start_entry_id {
        Some(id) => Some(id.clone()),
        None => match session.messages(&req.session_id, true).await {
            Ok(entries) => first_after(&entries, summary.read_up_to.as_deref()),
            Err(error) => {
                return outcome(
                    req,
                    compaction_id,
                    Status::Failed,
                    failed(&format!("could not read the conversation: {error}")),
                )
            }
        },
    };
    let data = json!({
        "summary": summary.text,
        "tail_start_entry_id": tail_start_entry_id,
        "tokens_before": summary.tokens_before,
        "tokens_after": summary.tokens_after,
        "summarized_head_tokens": summary.tokens_before,
        "timestamp": AgentMessage::now_ms(),
        "compaction_id": compaction_id,
        "model": req.model,
        "source": "command",
    });
    if let Err(error) = session
        .append_custom(
            &req.session_id,
            RECORD,
            data,
            entry,
            Some(&origin(compaction_id)),
        )
        .await
    {
        return outcome(
            req,
            compaction_id,
            Status::Failed,
            failed(&format!("the summary could not be saved: {error}")),
        );
    }
    if let Err(error) = refresh_snapshot(deps, session, &req.session_id, &summary).await {
        tracing::warn!(session_id = %req.session_id, %error, "context snapshot not refreshed after compaction");
    }
    let message = format!("compacted {} tokens", grouped(summary.tokens_before));
    Outcome {
        tokens_before: Some(summary.tokens_before),
        tokens_after: summary.tokens_after,
        summary: Some(summary.text),
        ..outcome(req, compaction_id, Status::Ok, message)
    }
}

/// Rewrite the session's context snapshot for the compacted window: the last
/// generate step's categories with the new summary and the verbatim tail.
async fn refresh_snapshot(
    deps: &Deps,
    session: &SessionClient,
    session_id: &str,
    summary: &Summary,
) -> Result<(), String> {
    let cfg = deps.cfg().await;
    let Some(last) = context_snapshot::get(&deps.iii, session_id, cfg.session_timeout_ms)
        .await
        .map_err(|e| e.to_string())?
    else {
        // No generate step yet: the chip has nothing to correct.
        return Ok(());
    };
    let entries = session
        .messages(session_id, true)
        .await
        .map_err(|e| e.to_string())?;
    let anchor = crate::turn_loop::compaction_anchor(session_id, &entries);
    let window = crate::window::build(&entries, anchor.window_start, None, None);
    let context = deps.context().await;
    let count = |messages: Vec<Value>, system_prompt: &str| {
        context.count_tokens(CountTokensParams {
            messages,
            model_id: last.model.clone(),
            provider: last.provider.clone(),
            system_prompt: Some(system_prompt.to_string()),
            tools: Vec::new(),
        })
    };
    let counted = count(
        window
            .candidate
            .iter()
            .map(|(_, message)| serde_json::to_value(message).unwrap_or(Value::Null))
            .collect(),
        &summary.text,
    )
    .await?;
    // A context-manager without the by-role split cannot tell the summary
    // from the messages; leave the snapshot to the next generate step.
    let Some(by_role) = counted.by_role else {
        return Ok(());
    };
    let messages = SnapshotMessagesV1::from(by_role);
    let summary_tokens = counted.tokens.saturating_sub(messages.total());
    let previous_summary_tokens = match &summary.previous {
        Some(text) if last.compacted => count(Vec::new(), text).await.map(|c| c.tokens)?,
        _ => 0,
    };
    let next = context_snapshot::after_compaction(
        &last,
        messages,
        previous_summary_tokens,
        summary_tokens,
        summary.tokens_before,
        AgentMessage::now_ms(),
    );
    context_snapshot::put(&deps.iii, &next, cfg.session_timeout_ms)
        .await
        .map_err(|e| e.to_string())
}

/// How much of the window is settled: everything before the first reply of
/// the live turn that still waits on a call's result. A cut past it would
/// summarise the call while its result lands later in the kept tail — an
/// orphaned result providers reject. Calls of finished turns never get a
/// result (the turn loop repairs them), so only the live turn's count.
fn settled_prefix(candidate: &[(String, AgentMessage)], live_turn: Option<&str>) -> usize {
    let Some(turn) = live_turn else {
        return candidate.len();
    };
    let prefix = format!("e_{turn}_");
    candidate
        .iter()
        .enumerate()
        .position(|(index, (id, message))| {
            let AgentMessage::Assistant(reply) = message else {
                return false;
            };
            if !id.starts_with(&prefix) {
                return false;
            }
            let answered: HashSet<&str> = candidate[index + 1..]
                .iter()
                .filter_map(|(_, later)| match later {
                    AgentMessage::FunctionResult(result) => Some(result.function_call_id.as_str()),
                    _ => None,
                })
                .collect();
            reply.content.iter().any(|block| {
                matches!(block, ContentBlock::FunctionCall { id, .. } if !answered.contains(id.as_str()))
            })
        })
        .unwrap_or(candidate.len())
}

/// The entry the verbatim tail opens at, from `context::compact`'s index into
/// the settled prefix it was sent. Everything summarised leaves the tail at
/// the first unsettled message, or nothing (`null`) when all was settled.
fn tail_entry(
    candidate: &[(String, AgentMessage)],
    settled: usize,
    tail_start_index: Option<usize>,
) -> Option<String> {
    let index = tail_start_index.filter(|&i| i < settled).unwrap_or(settled);
    candidate.get(index).map(|(id, _)| id.clone())
}

/// The first entry logged after `read_up_to` (the window's last read entry).
fn first_after(entries: &[LoadedEntry], read_up_to: Option<&str>) -> Option<String> {
    let last = read_up_to?;
    let at = entries.iter().position(|entry| entry.entry_id == last)?;
    entries.get(at + 1).map(|entry| entry.entry_id.clone())
}

fn outcome(req: &Request, compaction_id: &str, status: Status, message: String) -> Outcome {
    Outcome {
        session_id: req.session_id.clone(),
        compaction_id: compaction_id.to_string(),
        status,
        message,
        tokens_before: None,
        tokens_after: None,
        summary: None,
        entry_ids: Vec::new(),
    }
}

fn failed(reason: &str) -> String {
    format!("compaction failed — {reason}")
}

fn notice_data(compaction_id: &str, reason: &str, message: &str, tone: &str) -> Value {
    json!({
        "reason": reason,
        "message": message,
        "tone": tone,
        "compaction_id": compaction_id,
    })
}

fn origin(compaction_id: &str) -> Value {
    json!({ "compaction_id": compaction_id })
}

/// The `harness::compaction` payload for `phase` (`started` / `completed`).
pub fn event_payload(phase: &str, outcome: &Outcome) -> Value {
    let mut payload = json!({
        "session_id": outcome.session_id,
        "compaction_id": outcome.compaction_id,
        "phase": phase,
        "timestamp": AgentMessage::now_ms(),
    });
    if phase == "completed" {
        payload["status"] = json!(outcome.status);
        payload["message"] = json!(outcome.message);
        payload["entry_ids"] = json!(outcome.entry_ids);
        if let Some(tokens) = outcome.tokens_before {
            payload["tokens_before"] = json!(tokens);
        }
        if let Some(tokens) = outcome.tokens_after {
            payload["tokens_after"] = json!(tokens);
        }
    }
    payload
}

/// `1234567` → `1,234,567`, as the clients render the marker.
fn grouped(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clients::session::LoadedCustom;
    use crate::types::message::empty_assistant;

    fn reply(text: &str) -> AgentMessage {
        let mut a = empty_assistant("p", "m");
        a.content = vec![ContentBlock::text(text)];
        AgentMessage::Assistant(a)
    }

    fn calls(ids: &[&str]) -> AgentMessage {
        let mut a = empty_assistant("p", "m");
        a.content = ids
            .iter()
            .map(|id| ContentBlock::FunctionCall {
                id: (*id).into(),
                function_id: "harness::spawn".into(),
                arguments: json!({}),
            })
            .collect();
        AgentMessage::Assistant(a)
    }

    fn result(call_id: &str) -> AgentMessage {
        serde_json::from_value(json!({
            "role": "function_result", "function_call_id": call_id,
            "function_id": "harness::spawn", "content": [], "details": {}, "is_error": false, "timestamp": 1
        }))
        .unwrap()
    }

    fn window(rows: Vec<(&str, AgentMessage)>) -> Vec<(String, AgentMessage)> {
        rows.into_iter()
            .map(|(id, m)| (id.to_string(), m))
            .collect()
    }

    #[test]
    fn the_command_is_the_whole_message_with_optional_guidance() {
        assert_eq!(parse("/compact"), Some(Command { instructions: None }));
        assert_eq!(parse("  /compact \n"), Some(Command { instructions: None }));
        assert_eq!(
            parse("/compact keep the ticket ids\nand the decisions"),
            Some(Command {
                instructions: Some("keep the ticket ids\nand the decisions".into())
            })
        );
        assert_eq!(parse("/compaction"), None);
        assert_eq!(parse("/compactkeep"), None);
        assert_eq!(parse("please /compact"), None);
        assert_eq!(parse("/Compact"), None);
        assert_eq!(parse(""), None);
    }

    #[test]
    fn only_a_text_only_user_message_is_the_command() {
        assert!(parse_message(&AgentMessage::user_text("/compact")).is_some());
        let with_file: AgentMessage = serde_json::from_value(json!({
            "role": "user", "timestamp": 1, "content": [
                { "type": "text", "text": "/compact" },
                { "type": "file", "attachment_id": "a", "name": "n", "mime": "text/plain", "size": 1 }
            ]
        }))
        .unwrap();
        assert!(parse_message(&with_file).is_none());
        assert!(parse_message(&reply("/compact")).is_none());
    }

    #[test]
    fn one_compaction_per_session_at_a_time() {
        let compactions = Compactions::default();
        let first = compactions.begin("s1", "c1").expect("free");
        assert!(compactions.is_running("s1"));
        assert!(compactions.begin("s1", "c2").is_none());
        assert!(compactions.begin("s2", "c3").is_some());
        drop(first);
        assert!(!compactions.is_running("s1"));
        assert!(compactions.begin("s1", "c4").is_some());
    }

    #[test]
    fn a_finished_turn_settles_the_whole_window() {
        // An interrupted call never gets a result; the turn loop repairs it.
        let w = window(vec![
            ("u1", AgentMessage::user_text("go")),
            ("e_t_old_0_assistant", calls(&["dead"])),
            ("a2", reply("done")),
        ]);
        assert_eq!(settled_prefix(&w, None), 3);
        assert_eq!(settled_prefix(&w, Some("t_new")), 3);
    }

    #[test]
    fn the_live_turns_unanswered_call_bounds_what_can_be_summarised() {
        let w = window(vec![
            ("u1", AgentMessage::user_text("coordinate")),
            ("e_t_live_0_assistant", calls(&["c1"])),
            ("e_t_live_c1", result("c1")),
            ("e_t_live_1_assistant", calls(&["c2", "c3"])),
            ("e_t_live_c2", result("c2")),
            ("n1", AgentMessage::user_text("[notification] child done")),
        ]);
        // c3 still waits for its result: the cut must stay before its reply.
        assert_eq!(settled_prefix(&w, Some("t_live")), 3);
    }

    #[test]
    fn a_result_for_an_earlier_call_with_the_same_id_does_not_settle_a_later_one() {
        let w = window(vec![
            ("e_t_live_c1", result("c1")),
            ("e_t_live_0_assistant", calls(&["c1"])),
        ]);
        assert_eq!(settled_prefix(&w, Some("t_live")), 1);
    }

    #[test]
    fn the_tail_maps_back_to_entry_ids() {
        let w = window(vec![
            ("u1", AgentMessage::user_text("a")),
            ("a1", reply("b")),
            ("u2", AgentMessage::user_text("c")),
            ("e_t_live_0_assistant", calls(&["c1"])),
        ]);
        assert_eq!(tail_entry(&w, 3, Some(2)).as_deref(), Some("u2"));
        // Everything sent was summarised: the tail opens at the unsettled reply.
        assert_eq!(
            tail_entry(&w, 3, None).as_deref(),
            Some("e_t_live_0_assistant")
        );
        // An index past what was sent is treated the same way.
        assert_eq!(
            tail_entry(&w, 3, Some(9)).as_deref(),
            Some("e_t_live_0_assistant")
        );
        // All settled and all summarised: nothing verbatim.
        assert_eq!(tail_entry(&w, 4, None), None);
    }

    #[test]
    fn entries_appended_during_the_summary_stay_in_the_window() {
        let entry = |id: &str| LoadedEntry {
            entry_id: id.into(),
            message: None,
            custom: Some(LoadedCustom {
                custom_type: "notice".into(),
                data: Value::Null,
            }),
        };
        let entries = vec![entry("a"), entry("b"), entry("c")];
        assert_eq!(first_after(&entries, Some("a")).as_deref(), Some("b"));
        assert_eq!(first_after(&entries, Some("c")), None);
        assert_eq!(first_after(&entries, Some("gone")), None);
        assert_eq!(first_after(&entries, None), None);
    }

    #[test]
    fn the_completion_event_carries_the_outcome() {
        let outcome = Outcome {
            session_id: "s1".into(),
            compaction_id: "cmp_1".into(),
            status: Status::Ok,
            message: "compacted 471,486 tokens".into(),
            tokens_before: Some(471_486),
            tokens_after: Some(9_000),
            summary: Some("S".into()),
            entry_ids: vec!["e_idem_m1".into(), "e_cmp_1_outcome".into()],
        };
        let started = event_payload("started", &outcome);
        assert_eq!(started["phase"], "started");
        assert!(started.get("status").is_none());
        let done = event_payload("completed", &outcome);
        assert_eq!(done["status"], "ok");
        assert_eq!(done["message"], "compacted 471,486 tokens");
        assert_eq!(done["tokens_before"], 471_486);
        assert_eq!(done["entry_ids"], json!(["e_idem_m1", "e_cmp_1_outcome"]));
        assert!(
            done.get("summary").is_none(),
            "the summary rides the record, not the event"
        );
    }

    #[test]
    fn token_counts_are_grouped_like_the_clients_render_them() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(1_000), "1,000");
        assert_eq!(grouped(471_486), "471,486");
        assert_eq!(grouped(1_234_567), "1,234,567");
    }
}
