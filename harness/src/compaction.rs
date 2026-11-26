//! Conversation compaction: the `/compact` command sent through
//! `harness::send` by any client, `harness::compact` for callers that wait
//! for the outcome (the console's model switch), and the automatic
//! end-of-turn compaction ([`crate::autocompact`], `trigger: "auto"`). Not to
//! be confused with [`crate::turn_compaction`], which slims finished turn
//! records.
//!
//! The harness owns it so the command means the same thing from every client
//! (the mobile apps sent `/compact` to the model as a prompt) and every device
//! watching the session learns about it, not just the one that asked:
//!
//! - a `notice` row "compacting the conversation…" (an automatic one:
//!   "Auto-compacting at 160k…") lands in the transcript when it starts —
//!   under the sender's `e_idem_` id, so its optimistic `/compact` row
//!   reconciles into it;
//! - a `/compact` send shows the session `working` / "compacting context"
//!   while it runs (unless a live turn owns the status line);
//! - it ends with one row: the `compaction` record, or a `notice` saying why
//!   there is none;
//! - `harness::on-compaction` fires on start and on completion;
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
use serde_json::{Value, json};

use crate::autocompact::Rule;
use crate::clients::SessionClient;
use crate::clients::context::{CompactFailure, CompactOutput, CompactParams, CountTokensParams};
use crate::clients::session::LoadedEntry;
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
const ALREADY_RUNNING: &str = "compaction skipped — this conversation is already being compacted; its result will appear here.";
const LEASE_BUSY: &str = "compaction skipped — another compaction of this conversation is in progress; try /compact again when it finishes.";
const NOTHING_TO_COMPACT: &str =
    "nothing to compact yet — the conversation still fits word for word.";
const AUTO_FAILED_TAIL: &str = "The conversation goes on and compacts at the model's limit.";
/// A usage limit starts no back-off: the next turn's end tries again (D162).
const AUTO_FAILED_LIMIT_TAIL: &str =
    "The conversation goes on and compacts again after its next turn.";
/// A Stop starts no back-off either, but it abandons only the harness's
/// wait: context-manager finishes the summary it was asked for and holds the
/// conversation's lease until then, so a turn that ends meanwhile finds it
/// busy. A later turn's end compacts at the threshold again.
const AUTO_STOPPED: &str =
    "Auto-compact stopped. The conversation goes on and compacts again after a later turn.";
/// How often an automatic compaction checks for a Stop that landed on
/// another harness instance (the in-process signal covers this one).
const STOP_POLL: std::time::Duration = std::time::Duration::from_secs(2);

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

/// What started a compaction (chat-projects contract C).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Trigger {
    /// `/compact` or `harness::compact`.
    Command,
    /// The end of a turn past the autocompact threshold.
    Auto,
    /// A turn's assembly, at the autocompact budget.
    Inline,
    /// A turn's assembly, at the model's own limit (as before autocompact).
    Limit,
}

impl Trigger {
    /// The record's `source`, kept for older readers.
    pub fn source(self) -> &'static str {
        match self {
            Trigger::Command => "command",
            Trigger::Auto => "auto",
            Trigger::Inline => "auto_inline",
            Trigger::Limit => "limit",
        }
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
    /// `command` or `auto`.
    pub trigger: Trigger,
    /// The threshold an automatic compaction crossed.
    pub threshold_tokens: Option<u64>,
    /// The rule that set it.
    pub rule: Option<Rule>,
    /// Forwarded to the summariser's `router::chat`: the conversation's
    /// Claude Code account (D161).
    pub provider_options: Option<Value>,
    /// The Claude Code account the summary runs on, to name it in a usage
    /// limit's notice.
    pub account: Option<String>,
    /// The whole context before (the caller measured it); `None` reads the
    /// stored snapshot.
    pub context_before: Option<u64>,
    /// The turn whose Stop abandons the summary (an end-of-turn compaction).
    pub stop_turn: Option<String>,
}

impl Request {
    /// A `/compact` or `harness::compact` request.
    pub fn command(session_id: String, model: String, provider: Option<String>) -> Self {
        Self {
            session_id,
            model,
            provider,
            instructions: None,
            entry_id: None,
            mark_working: false,
            trigger: Trigger::Command,
            threshold_tokens: None,
            rule: None,
            provider_options: None,
            account: None,
            context_before: None,
            stop_turn: None,
        }
    }

    /// Run the summary on the conversation's Claude Code account, resolved
    /// now (D161): `/compact` and `harness::compact` have no turn whose
    /// account they could take.
    pub async fn on_conversation_account(mut self, deps: &Deps) -> Self {
        let session = deps.session().await;
        if let Some(account) = crate::accounts::compaction_account(
            deps,
            &session,
            &self.session_id,
            self.provider.as_deref(),
            &self.model,
        )
        .await
        {
            self.provider_options = Some(account.provider_options);
            self.account = Some(account.id);
        }
        self
    }

    fn is_auto(&self) -> bool {
        self.trigger != Trigger::Command
    }
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
    /// A Stop abandoned an automatic compaction: no record was written.
    Cancelled,
}

/// How a compaction ended — also what `harness::on-compaction` carries on
/// completion.
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
    /// What started it.
    pub trigger: Trigger,
    /// [`Trigger::source`], for readers older than `trigger`.
    pub source: String,
    /// The threshold an automatic compaction crossed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold_tokens: Option<u64>,
    /// The rule that set the threshold.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rule: Option<Rule>,
    /// The whole context's tokens before and after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_before: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_after: Option<u64>,
    /// Why the summariser failed, when context-manager said (1.3.0).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<CompactFailure>,
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
    /// Why, as a reason; and context-manager's failure when it gave one.
    Failed(String, Option<CompactFailure>),
    Cancelled,
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
    let started_text = started_message(&req);
    let mut started_data = notice_data(&compaction_id, "compacting", &started_text, "info");
    annotate(&mut started_data, &req);
    if let Err(error) = session
        .append_custom(
            &req.session_id,
            NOTICE,
            started_data,
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
    let started = outcome(&req, &compaction_id, Status::Ok, started_text);
    deps.events
        .emit_compaction(&req.session_id, event_payload("started", &started))
        .await;

    // The whole context before: what the caller measured, else the stored
    // snapshot (plus its last output, which the next request carries).
    let context_before = match req.context_before {
        Some(tokens) => Some(tokens),
        None => context_snapshot::get(&deps.iii, &req.session_id, cfg.session_timeout_ms)
            .await
            .ok()
            .flatten()
            .map(|snapshot| crate::autocompact::measure(&snapshot)),
    };
    let summarise = summarise(deps, &session, &req, live_turn.as_deref());
    let summarised = match &req.stop_turn {
        Some(turn_id) => tokio::select! {
            summarised = summarise => summarised,
            () = stopped(deps, &req.session_id, turn_id) => Summarised::Cancelled,
        },
        None => summarise.await,
    };
    let outcome = persist(
        deps,
        &session,
        &req,
        &compaction_id,
        &outcome_entry,
        summarised,
        context_before,
    )
    .await;
    drop(running);
    if owns_status && let Err(error) = crate::session_status::repair(deps, &req.session_id).await {
        tracing::warn!(session_id = %req.session_id, %error, "session status restore after compaction failed");
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

/// Resolves once `turn_id` is stopped: this process's cancel signal, or the
/// abort flag a Stop on another instance wrote (polled), or the turn gone.
async fn stopped(deps: &Deps, session_id: &str, turn_id: &str) {
    let mut signal = deps.cancels.watch(turn_id);
    let timeout = deps.cfg().await.session_timeout_ms;
    loop {
        if *signal.borrow() {
            return;
        }
        tokio::select! {
            changed = signal.changed() => {
                if changed.is_err() {
                    // The signal was cleared: only the store can tell now.
                    tokio::time::sleep(STOP_POLL).await;
                }
            }
            () = tokio::time::sleep(STOP_POLL) => {}
        }
        if let Ok(record) = crate::state::get_turn_unhydrated(&deps.iii, session_id, timeout).await
            && record.is_none_or(|record| record.turn_id != turn_id || record.abort)
        {
            return;
        }
    }
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
    let tone = if matches!(outcome.status, Status::Empty | Status::Cancelled) {
        "info"
    } else {
        "warn"
    };
    let reason = match outcome.status {
        Status::Ok => "compacted",
        Status::Busy => "compaction_busy",
        Status::Empty => "compaction_empty",
        Status::Failed => "compaction_failed",
        Status::Cancelled => "compaction_cancelled",
    };
    let mut data = notice_data(&outcome.compaction_id, reason, &outcome.message, tone);
    data["status"] = json!(outcome.status);
    data["trigger"] = json!(outcome.trigger);
    data["source"] = json!(outcome.source);
    if let Some(threshold) = outcome.threshold_tokens {
        data["threshold_tokens"] = json!(threshold);
    }
    if let Some(rule) = &outcome.rule {
        data["rule"] = json!(rule);
    }
    if let Some(failure) = &outcome.failure {
        data["failure"] = json!(failure);
    }
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
            return Summarised::Failed(format!("could not read the conversation: {error}"), None);
        }
    };
    let anchor = crate::turn_loop::compaction_anchor(&req.session_id, &entries);
    let candidate =
        crate::window::build(&req.session_id, &entries, anchor.window_start, None, None).candidate;
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
            provider_options: req.provider_options.clone(),
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
        Ok(CompactOutput::Overflow { failure }) => {
            Summarised::Failed(failure_reason(deps, req, failure.as_ref()).await, failure)
        }
        Err(error) => {
            let failure = CompactFailure::from_error(&error);
            let reason = match &failure {
                Some(failure) => failure_reason(deps, req, Some(failure)).await,
                None => error,
            };
            Summarised::Failed(reason, failure)
        }
    }
}

/// Why the summariser failed, in words (D162), with the account's name
/// read from tmux for a usage limit.
async fn failure_reason(deps: &Deps, req: &Request, failure: Option<&CompactFailure>) -> String {
    let name = match (failure, &req.account) {
        (Some(failure), Some(id)) if failure.is_usage_limit() => {
            Some(crate::accounts::account_name(deps, id).await)
        }
        _ => None,
    };
    failure_words(req, failure, name.as_deref())
}

/// Why the summariser failed: the cause context-manager named, and "a
/// larger context window" only when the input did not fit. An older
/// context-manager names none: today's words. A usage limit names the
/// account (`name`, from tmux) and no clock time: the engine's time zone
/// is not the reader's.
fn failure_words(req: &Request, failure: Option<&CompactFailure>, name: Option<&str>) -> String {
    let Some(failure) = failure else {
        return too_large(&req.model);
    };
    if failure.is_usage_limit() {
        let who = name
            .or(req.account.as_deref())
            .unwrap_or("The Claude Code account");
        return format!("{who} reached its usage limit");
    }
    if failure.kind == "input_too_large" {
        return too_large(&req.model);
    }
    // Router messages come verbatim, sometimes with a final period.
    let message = failure.message.trim().trim_end_matches('.').trim_end();
    if message.is_empty() {
        format!(
            "the summariser could not produce a summary with {}",
            req.model
        )
    } else {
        message.to_string()
    }
}

/// Today's words for a summary that does not fit the summariser: the only
/// failure that names the window.
fn too_large(model: &str) -> String {
    format!(
        "the summariser could not produce a summary with {model}; try again, or switch to a model with a larger context window"
    )
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
    context_before: Option<u64>,
) -> Outcome {
    let with_before = |outcome: Outcome| Outcome {
        context_before,
        ..outcome
    };
    let summary = match summarised {
        Summarised::Done(summary) => summary,
        Summarised::Busy => {
            return with_before(outcome(req, compaction_id, Status::Busy, busy_message(req)));
        }
        Summarised::Empty => {
            return with_before(outcome(
                req,
                compaction_id,
                Status::Empty,
                NOTHING_TO_COMPACT.into(),
            ));
        }
        Summarised::Failed(reason, failure) => {
            let message = failed_message(req, &reason, failure.as_ref());
            return Outcome {
                failure,
                ..with_before(outcome(req, compaction_id, Status::Failed, message))
            };
        }
        Summarised::Cancelled => {
            return with_before(outcome(
                req,
                compaction_id,
                Status::Cancelled,
                AUTO_STOPPED.into(),
            ));
        }
    };
    let failure = |reason: String| {
        with_before(outcome(
            req,
            compaction_id,
            Status::Failed,
            failed_message(req, &reason, None),
        ))
    };
    let entries = match session.messages(&req.session_id, true).await {
        Ok(entries) => entries,
        Err(error) => return failure(format!("could not read the conversation: {error}")),
    };
    // Everything read was summarised: a turn that appended meanwhile must
    // stay in the window, so the tail opens at its first entry.
    let tail_start_entry_id = summary
        .tail_start_entry_id
        .clone()
        .or_else(|| first_after(&entries, summary.read_up_to.as_deref()));
    // The window the next turn assembles once the record lands, counted
    // before it is written so the record can say how large it is.
    let snapshot = match refreshed_snapshot(
        deps,
        &req.session_id,
        &entries,
        tail_start_entry_id.as_deref(),
        &summary,
    )
    .await
    {
        Ok(snapshot) => snapshot,
        Err(error) => {
            tracing::warn!(session_id = %req.session_id, %error, "context snapshot not refreshed after compaction");
            None
        }
    };
    let context_after = snapshot.as_ref().map(|snapshot| snapshot.total);
    let mut data = json!({
        "summary": summary.text,
        "tail_start_entry_id": tail_start_entry_id,
        "tokens_before": summary.tokens_before,
        "tokens_after": summary.tokens_after,
        "summarized_head_tokens": summary.tokens_before,
        "timestamp": AgentMessage::now_ms(),
        "compaction_id": compaction_id,
        "model": req.model,
        "source": req.trigger.source(),
        "trigger": req.trigger,
    });
    if let Some(threshold) = req.threshold_tokens {
        data["threshold_tokens"] = json!(threshold);
    }
    if let Some(rule) = &req.rule {
        data["rule"] = json!(rule);
    }
    if let Some(before) = context_before {
        data["context_before"] = json!(before);
    }
    if let Some(after) = context_after {
        data["context_after"] = json!(after);
    }
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
        return failure(format!("the summary could not be saved: {error}"));
    }
    if let Some(next) = &snapshot {
        let timeout = deps.cfg().await.session_timeout_ms;
        if let Err(error) = context_snapshot::put(&deps.iii, next, timeout).await {
            tracing::warn!(session_id = %req.session_id, %error, "context snapshot not stored after compaction");
        }
    }
    let message = match req.trigger {
        Trigger::Command => format!("compacted {} tokens", grouped(summary.tokens_before)),
        trigger => marker(
            trigger,
            req.threshold_tokens,
            context_before,
            context_after,
            req.rule.as_ref(),
        ),
    };
    Outcome {
        tokens_before: Some(summary.tokens_before),
        tokens_after: summary.tokens_after,
        summary: Some(summary.text),
        context_before,
        context_after,
        ..outcome(req, compaction_id, Status::Ok, message)
    }
}

/// The session's context snapshot for the compacted window, on top of the
/// last generate step's: its categories with the new summary and the
/// verbatim tail from `tail_start_entry_id` (none: everything was
/// summarised). `None` before any generate step, and from a context-manager
/// without the by-role split (the next generate step corrects it).
async fn refreshed_snapshot(
    deps: &Deps,
    session_id: &str,
    entries: &[LoadedEntry],
    tail_start_entry_id: Option<&str>,
    summary: &Summary,
) -> Result<Option<context_snapshot::ContextSnapshotV1>, String> {
    let cfg = deps.cfg().await;
    let Some(last) = context_snapshot::get(&deps.iii, session_id, cfg.session_timeout_ms)
        .await
        .map_err(|e| e.to_string())?
    else {
        // No generate step yet: the chip has nothing to correct.
        return Ok(None);
    };
    // Where the window opens once the record is the latest on the path, as
    // `compaction_anchor` will read it.
    let window_start = match tail_start_entry_id {
        Some(tail) => entries
            .iter()
            .position(|entry| entry.entry_id == tail)
            .unwrap_or(entries.len()),
        None => entries.len(),
    };
    let mut window = crate::window::build(session_id, entries, window_start, None, None);
    // Count what the next step sends: it strips thinking logged before the
    // record, and every entry read here was logged before it.
    if crate::window::strips_thinking_at_compaction(&last.model) {
        crate::window::strip_thinking_logged_before(&mut window, entries);
    }
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
        return Ok(None);
    };
    let messages = SnapshotMessagesV1::from(by_role);
    let summary_tokens = counted.tokens.saturating_sub(messages.total());
    let previous_summary_tokens = match &summary.previous {
        Some(text) if last.compacted => count(Vec::new(), text).await.map(|c| c.tokens)?,
        _ => 0,
    };
    Ok(Some(context_snapshot::after_compaction(
        &last,
        messages,
        previous_summary_tokens,
        summary_tokens,
        summary.tokens_before,
        AgentMessage::now_ms(),
    )))
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
        trigger: req.trigger,
        source: req.trigger.source().to_string(),
        threshold_tokens: req.threshold_tokens,
        rule: req.rule.clone(),
        context_before: None,
        context_after: None,
        failure: None,
    }
}

/// The outcome of a compaction `context::assemble` ran inside a turn
/// (`inline` or `limit`): its record was written with the step, and only its
/// completed phase is emitted (contract C).
#[allow(clippy::too_many_arguments)]
pub fn inline_outcome(
    session_id: &str,
    compaction_id: &str,
    trigger: Trigger,
    threshold_tokens: Option<u64>,
    rule: Option<Rule>,
    tokens_before: Option<u64>,
    context_before: Option<u64>,
    context_after: Option<u64>,
    entry_ids: Vec<String>,
) -> Outcome {
    Outcome {
        session_id: session_id.to_string(),
        compaction_id: compaction_id.to_string(),
        status: Status::Ok,
        message: marker(
            trigger,
            threshold_tokens,
            context_before,
            context_after,
            rule.as_ref(),
        ),
        tokens_before,
        tokens_after: context_after,
        summary: None,
        entry_ids,
        trigger,
        source: trigger.source().to_string(),
        threshold_tokens,
        rule,
        context_before,
        context_after,
        failure: None,
    }
}

/// The start notice: "compacting the conversation…", or for an automatic
/// compaction "Auto-compacting at 160k…" (D163).
fn started_message(req: &Request) -> String {
    match (req.trigger, req.threshold_tokens) {
        (Trigger::Command, _) => STARTED.into(),
        (_, Some(threshold)) => format!(
            "Auto-compacting at {}…",
            crate::autocompact::short_tokens(threshold)
        ),
        (_, None) => "Auto-compacting…".into(),
    }
}

fn busy_message(req: &Request) -> String {
    if req.is_auto() {
        "Auto-compact skipped: another compaction of this conversation is in progress.".into()
    } else {
        LEASE_BUSY.into()
    }
}

fn failed_message(req: &Request, reason: &str, failure: Option<&CompactFailure>) -> String {
    if !req.is_auto() {
        return failed(reason);
    }
    let tail = if failure.is_some_and(CompactFailure::is_usage_limit) {
        AUTO_FAILED_LIMIT_TAIL
    } else {
        AUTO_FAILED_TAIL
    };
    format!("Auto-compact failed: {reason}. {tail}")
}

/// The marker every client shows for an automatic compaction (D163):
/// "Auto-compacted at 160k · 161k → 34k tokens · acme-web rule".
pub fn marker(
    trigger: Trigger,
    threshold: Option<u64>,
    before: Option<u64>,
    after: Option<u64>,
    rule: Option<&Rule>,
) -> String {
    use crate::autocompact::short_tokens;
    let sizes = match (before, after) {
        (Some(before), Some(after)) => {
            format!(
                " · {} → {} tokens",
                short_tokens(before),
                short_tokens(after)
            )
        }
        _ => String::new(),
    };
    let at = threshold
        .map(|threshold| format!(" at {}", short_tokens(threshold)))
        .unwrap_or_default();
    let rule = rule
        .filter(|_| trigger != Trigger::Limit)
        .map(|rule| format!(" · {}", rule.name()))
        .unwrap_or_default();
    match trigger {
        Trigger::Limit => format!("Compacted at the model's limit{sizes}"),
        Trigger::Inline => format!("Auto-compacted during the turn{at}{sizes}{rule}"),
        Trigger::Auto => format!("Auto-compacted{at}{sizes}{rule}"),
        Trigger::Command => format!("Compacted{sizes}"),
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

/// The fields an automatic compaction's start notice carries.
fn annotate(data: &mut Value, req: &Request) {
    if !req.is_auto() {
        return;
    }
    data["trigger"] = json!(req.trigger);
    data["source"] = json!(req.trigger.source());
    if let Some(threshold) = req.threshold_tokens {
        data["threshold_tokens"] = json!(threshold);
    }
    if let Some(rule) = &req.rule {
        data["rule"] = json!(rule);
    }
}

fn origin(compaction_id: &str) -> Value {
    json!({ "compaction_id": compaction_id })
}

/// The `harness::on-compaction` payload for `phase` (`started` / `completed`).
pub fn event_payload(phase: &str, outcome: &Outcome) -> Value {
    let mut payload = json!({
        "session_id": outcome.session_id,
        "compaction_id": outcome.compaction_id,
        "phase": phase,
        "timestamp": AgentMessage::now_ms(),
        "trigger": outcome.trigger,
        "source": outcome.source,
    });
    if let Some(threshold) = outcome.threshold_tokens {
        payload["threshold_tokens"] = json!(threshold);
    }
    if let Some(rule) = &outcome.rule {
        payload["rule"] = json!(rule);
    }
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
        if let Some(tokens) = outcome.context_before {
            payload["context_before"] = json!(tokens);
        }
        if let Some(tokens) = outcome.context_after {
            payload["context_after"] = json!(tokens);
        }
        if let Some(failure) = &outcome.failure {
            payload["failure"] = json!(failure);
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
            trigger: Trigger::Command,
            source: "command".into(),
            threshold_tokens: None,
            rule: None,
            context_before: Some(480_000),
            context_after: Some(12_000),
            failure: None,
        };
        let started = event_payload("started", &outcome);
        assert_eq!(started["phase"], "started");
        assert_eq!(started["trigger"], "command");
        assert_eq!(started["source"], "command");
        assert!(started.get("status").is_none());
        assert!(started.get("context_before").is_none());
        let done = event_payload("completed", &outcome);
        assert_eq!(done["status"], "ok");
        assert_eq!(done["message"], "compacted 471,486 tokens");
        assert_eq!(done["tokens_before"], 471_486);
        assert_eq!(done["context_before"], 480_000);
        assert_eq!(done["context_after"], 12_000);
        assert_eq!(done["entry_ids"], json!(["e_idem_m1", "e_cmp_1_outcome"]));
        assert!(done.get("threshold_tokens").is_none());
        assert!(done.get("rule").is_none());
        assert!(
            done.get("summary").is_none(),
            "the summary rides the record, not the event"
        );
    }

    fn acme_rule() -> Rule {
        Rule {
            scope: crate::autocompact::RuleScope::Project,
            project_id: Some("prj_1".into()),
            project_name: Some("acme-web".into()),
        }
    }

    fn auto_request() -> Request {
        Request {
            trigger: Trigger::Auto,
            threshold_tokens: Some(160_000),
            rule: Some(acme_rule()),
            account: Some("work".into()),
            ..Request::command("s1".into(), "claude-code/opus".into(), None)
        }
    }

    /// Contract C: the record's and the event's `trigger`, `source`,
    /// `threshold_tokens`, `rule` and the context sizes.
    #[test]
    fn an_automatic_outcome_carries_the_contract_c_fields() {
        let req = auto_request();
        let started = outcome(&req, "cmp_1", Status::Ok, started_message(&req));
        let payload = event_payload("started", &started);
        assert_eq!(
            payload,
            json!({
                "session_id": "s1",
                "compaction_id": "cmp_1",
                "phase": "started",
                "timestamp": payload["timestamp"],
                "trigger": "auto",
                "source": "auto",
                "threshold_tokens": 160_000,
                "rule": { "scope": "project", "project_id": "prj_1", "project_name": "acme-web" },
            })
        );
        let failure = CompactFailure {
            kind: "timeout".into(),
            message: "The summary request timed out.".into(),
            error_details: None,
        };
        let failed = Outcome {
            failure: Some(failure.clone()),
            context_before: Some(161_000),
            ..outcome(&req, "cmp_1", Status::Failed, "x".into())
        };
        let done = event_payload("completed", &failed);
        assert_eq!(done["status"], "failed");
        assert_eq!(done["failure"], json!(failure));
        assert_eq!(done["context_before"], 161_000);
        assert!(done.get("context_after").is_none());
    }

    #[test]
    fn the_notices_name_the_trigger_the_threshold_and_the_rule() {
        let req = auto_request();
        assert_eq!(started_message(&req), "Auto-compacting at 160k…");
        assert_eq!(
            started_message(&Request::command("s1".into(), "m".into(), None)),
            STARTED
        );
        let rule = acme_rule();
        assert_eq!(
            marker(
                Trigger::Auto,
                Some(160_000),
                Some(161_000),
                Some(34_000),
                Some(&rule)
            ),
            "Auto-compacted at 160k · 161k → 34k tokens · acme-web rule"
        );
        assert_eq!(
            marker(
                Trigger::Inline,
                Some(160_000),
                Some(181_000),
                Some(30_000),
                Some(&Rule::global())
            ),
            "Auto-compacted during the turn at 160k · 181k → 30k tokens · global rule"
        );
        assert_eq!(
            marker(Trigger::Limit, None, Some(116_000), Some(30_000), None),
            "Compacted at the model's limit · 116k → 30k tokens"
        );
        assert_eq!(
            marker(
                Trigger::Auto,
                Some(600_000),
                Some(612_000),
                Some(41_000),
                Some(&Rule::session())
            ),
            "Auto-compacted at 600k · 612k → 41k tokens · this conversation's rule"
        );
        assert_eq!(Trigger::Inline.source(), "auto_inline");
        assert_eq!(Trigger::Limit.source(), "limit");
    }

    /// D162 and the spec revision: "a larger context window" only for an
    /// input too large; a usage limit names the account and no clock time,
    /// and the conversation compacts again after its next turn.
    #[test]
    fn a_failed_auto_compaction_says_why_by_its_cause() {
        let req = auto_request();
        let usage = CompactFailure {
            kind: "usage_limit".into(),
            message: "This Claude Code account reached its usage limit.".into(),
            error_details: Some(crate::types::message::ErrorDetails {
                usage_limit: true,
                window: Some("five_hour".into()),
                resets_at_ms: Some(1_700_000_000_000),
            }),
        };
        assert_eq!(
            failed_message(&req, "Personal reached its usage limit", Some(&usage)),
            "Auto-compact failed: Personal reached its usage limit. The conversation goes on and compacts again after its next turn."
        );
        let other = CompactFailure {
            kind: "rate_limited".into(),
            message: "Too many requests.".into(),
            error_details: None,
        };
        assert_eq!(
            failed_message(&req, "Too many requests", Some(&other)),
            "Auto-compact failed: Too many requests. The conversation goes on and compacts at the model's limit."
        );
        // `/compact` keeps its own words.
        let command = Request::command("s1".into(), "m".into(), None);
        assert_eq!(
            failed_message(&command, "Too many requests", Some(&other)),
            "compaction failed — Too many requests"
        );
        assert!(too_large("m").contains("larger context window"));
    }

    #[test]
    fn the_cause_comes_from_context_managers_failure() {
        let req = auto_request();
        let failure = |kind: &str, message: &str, usage_limit: bool| CompactFailure {
            kind: kind.into(),
            message: message.into(),
            error_details: usage_limit.then(|| crate::types::message::ErrorDetails {
                usage_limit: true,
                window: Some("week".into()),
                resets_at_ms: Some(1_700_000_000_000),
            }),
        };
        let limit = failure("usage_limit", "limit", true);
        assert_eq!(
            failure_words(&req, Some(&limit), Some("Personal")),
            "Personal reached its usage limit"
        );
        // The name could not be read: the id.
        assert_eq!(
            failure_words(&req, Some(&limit), None),
            "work reached its usage limit"
        );
        // A usage limit reported under `rate_limited` with its details.
        assert_eq!(
            failure_words(
                &req,
                Some(&failure("rate_limited", "x", true)),
                Some("Personal")
            ),
            "Personal reached its usage limit"
        );
        assert_eq!(
            failure_words(
                &req,
                Some(&failure("input_too_large", "too big", false)),
                None
            ),
            "the summariser could not produce a summary with claude-code/opus; try again, or switch to a model with a larger context window"
        );
        assert_eq!(
            failure_words(
                &req,
                Some(&failure("timeout", "The request timed out.", false)),
                None
            ),
            "The request timed out"
        );
        assert_eq!(
            failure_words(&req, Some(&failure("other", "  ", false)), None),
            "the summariser could not produce a summary with claude-code/opus"
        );
        // An older context-manager: today's words.
        assert_eq!(
            failure_words(&req, None, None),
            too_large("claude-code/opus")
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
