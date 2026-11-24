//! Sidechats (chat-projects phase 2, D14–D16): a side conversation forked
//! from a point of a conversation. It runs on the conversation's model,
//! effort and account (D2), so its first request reads the conversation's
//! prompt cache, and it writes nothing into the conversation.
//!
//! - `harness::side-chat::open` forks the conversation (`session::fork`),
//!   tags the fork, copies the conversation's turn record onto it as a
//!   finished turn — the record its first send continues from, so the
//!   system prompt, tools, skills and working folder are the conversation's —
//!   and writes the lock, `harness_side_chat/<sid>`.
//! - `harness::send` on a side chat enforces the lock ([`prepare_send`]),
//!   builds one for a side chat the ADE forked before this phase, and names
//!   the side chat after its first message.
//! - `close`, `keep` and `summarize`; a summary reply's entry id joins
//!   `side_chat_summaries` when its turn completes ([`record_summary_reply`]).
//! - `harness::delete-session-tree` deletes a conversation's side chats with
//!   it, and `harness::context-items` lists them ([`side_chats_of`]).

use std::collections::BTreeMap;

use iii_sdk::protocol::CallRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::clients::session::{MetaPatch, SessionClient, SessionSummary};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::{MessageInput, SendOptions, SendRequest};
use crate::types::message::AgentMessage;
use crate::types::model::ThinkingLevel;
use crate::types::turn::{
    FS_SCOPE_KEY, FS_SCOPE_ROOT_KEY, FunctionPolicy, TurnOptions, TurnRecord, TurnStatus,
};

pub const OPEN_ID: &str = "harness::side-chat::open";
pub const OPEN_DESC: &str = "Internal control-plane: open a side chat of a conversation, forked at one of its entries \
     (default: its last user or assistant message). The side chat runs on the conversation's \
     model, effort and account, and writes nothing into it. Refused for a side chat \
     (`nested_side_chat`) and for a conversation with no turn yet (`nothing_to_fork`).";

pub const CLOSE_ID: &str = "harness::side-chat::close";
pub const CLOSE_DESC: &str = "Internal control-plane: close a side chat. One in which nothing was sent is deleted \
     (`deleted: true`); one with messages is kept (`deleted: false`).";

pub const KEEP_ID: &str = "harness::side-chat::keep";
pub const KEEP_DESC: &str = "Internal control-plane: make a side chat an ordinary conversation, in its conversation's \
     project: its link to the conversation and its lock go.";

pub const SUMMARIZE_ID: &str = "harness::side-chat::summarize";
pub const SUMMARIZE_DESC: &str = "Internal control-plane: ask a side chat for a summary for its conversation (a turn with a \
     fixed prompt). When the reply completes, its entry id is appended to the side chat's \
     `side_chat_summaries`.";

/// Harness state scope of the full lock, keyed by the side chat's id.
pub const LOCK_SCOPE: &str = "harness_side_chat";
/// A side chat's title until its first message names it.
pub const TITLE: &str = "Side chat";
/// A derived title's length, as the clients cut theirs.
pub const TITLE_MAX_CHARS: usize = 60;
/// What `summarize` sends (spec §2.2, exactly).
pub const SUMMARY_PROMPT: &str = "Summarize what we concluded in this side conversation for the main \
     conversation, in at most five short lines. Plain text, no preamble.";
/// Summary replies kept in `side_chat_summaries`, newest last.
pub const SUMMARIES_KEPT: usize = 20;

/// Session metadata: the conversation a side chat belongs to.
pub const SIDE_CHAT_OF: &str = "side_chat_of";
/// Session metadata: the side chat's own copy of the entry it was forked at
/// (its last copied message), as the ADE writes it.
pub const SIDE_CHAT_FROM: &str = "side_chat_from";
/// Session metadata: the passages of the conversation the side chat asked
/// about, `[{message_id, text, agent?}]`, newest last.
pub const SIDE_CHAT_QUOTES: &str = "side_chat_quotes";
/// Session metadata: the lock for display, `{model, provider?,
/// thinking_level?, account?}`; the full lock is in [`LOCK_SCOPE`].
pub const SIDE_CHAT_LOCK: &str = "side_chat_lock";
/// Session metadata: entry ids of summary replies, newest last.
pub const SIDE_CHAT_SUMMARIES: &str = "side_chat_summaries";
/// Session metadata: where the side chat was forked, `{entry_id,
/// message_count}` — the conversation's entry (for the mark under it in the
/// conversation) and how many messages the fork copied (so a pane counts
/// only the side chat's own).
pub const SIDE_CHAT_FORK_POINT: &str = "side_chat_fork_point";
/// Turn metadata (`options.metadata`) of a `summarize` turn.
pub const SUMMARY_TURN_KEY: &str = "side_chat_summary";

/// Keys the harness writes on a spawned session. A fork copies them and a
/// side chat must not keep them: deleting it would then notify the parent,
/// and its approvals would route to the parent (the ADE's `SPAWN_LINK_KEYS`).
const SPAWN_LINK_KEYS: [&str; 7] = [
    "parent_session_id",
    "parent_turn_id",
    "function_call_id",
    "parent_function_call_id",
    "depth",
    "spawned_by",
    "root_session_id",
];

/// Keys that name or describe the conversation itself. A side chat has its
/// own title and description, so it does not inherit who wrote them.
const OWN_NAME_KEYS: [&str; 6] = [
    "title_manual",
    "title_by",
    "description_by",
    "description_at_ms",
    "description_manual",
    "description_history",
];

const LOCKED_MESSAGE: &str =
    "A side chat keeps its conversation's model and effort, to reuse its cache.";

// ── wire shapes ──

/// A passage of the conversation the side chat asks about.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SideChatQuote {
    /// The conversation's message the passage came from (its entry id).
    pub message_id: String,
    pub text: String,
    /// Who wrote the message, for the quote card's label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SideChatOpenRequest {
    /// The conversation to open a side chat of.
    pub session_id: String,
    /// The conversation's entry to fork at. Default: its last user or
    /// assistant message (a reply still streaming is skipped).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_id: Option<String>,
    /// A passage the side chat asks about, recorded in `side_chat_quotes`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quote: Option<SideChatQuote>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SideChatOpenResponse {
    /// The side chat's session id.
    pub session_id: String,
    /// The side chat's session record after it was tagged (as `session::get`
    /// answers it in `meta`).
    pub meta: Value,
}

/// `close`, `keep` and `summarize` name the side chat.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SideChatRequest {
    /// The side chat's session id.
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SideChatCloseResponse {
    /// True when the side chat was empty and its deletion was accepted.
    pub deleted: bool,
    /// The `harness::delete-session-tree` operation that deletes it, when
    /// `deleted` (read it with `harness::delete-session-tree-status`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SideChatKeepResponse {
    /// The conversation's session record after the write.
    pub meta: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SideChatSummarizeResponse {
    /// The summary turn.
    pub turn_id: String,
}

/// The full lock (`harness_side_chat/<sid>`): the conversation's turn options
/// a side chat runs on, which `harness::send` enforces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct SideChatLock {
    pub session_id: String,
    pub parent_session_id: String,
    /// The conversation's entry the side chat was forked at; absent for a
    /// side chat forked before phase 2.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_entry_id: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// Provider-native options, the account among them (phase 4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_options: Option<BTreeMap<String, Value>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub functions: Option<FunctionPolicy>,
    /// The skills filter (`None`: every skill).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills: Option<Vec<String>>,
    /// `{root}`, the working folder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fs_scope: Option<Value>,
    pub created_at_ms: i64,
    /// `open`, or `lazy` for a side chat forked before phase 2, locked at
    /// its next send to the conversation's options then.
    pub source: String,
}

impl SideChatLock {
    fn from_options(
        session_id: &str,
        parent_session_id: &str,
        parent_entry_id: Option<String>,
        options: &TurnOptions,
        source: &str,
    ) -> Self {
        Self {
            session_id: session_id.to_string(),
            parent_session_id: parent_session_id.to_string(),
            parent_entry_id,
            model: options.model.clone(),
            provider: options.provider.clone(),
            thinking_level: options.thinking_level,
            provider_options: normalized_provider_options(options.provider_options.as_ref()),
            functions: options.functions.clone(),
            skills: normalized_skills(
                options
                    .skill_context
                    .as_ref()
                    .and_then(|context| context.filter.as_ref()),
            ),
            fs_scope: options
                .filesystem_root()
                .map(|root| json!({ FS_SCOPE_ROOT_KEY: root })),
            created_at_ms: AgentMessage::now_ms(),
            source: source.to_string(),
        }
    }

    /// The working folder the side chat is locked to.
    pub fn root(&self) -> Option<&str> {
        self.fs_scope
            .as_ref()
            .and_then(|scope| scope.get(FS_SCOPE_ROOT_KEY))
            .and_then(Value::as_str)
    }

    /// The account the turn runs on, when its provider options name one
    /// (`provider_options[<provider>].account`, phase 4).
    fn account(&self) -> Option<&str> {
        let options = self.provider_options.as_ref()?;
        self.provider
            .as_deref()
            .and_then(|provider| options.get(provider))
            .and_then(|o| o.get("account"))
            .or_else(|| options.values().find_map(|o| o.get("account")))
            .and_then(Value::as_str)
    }

    /// `side_chat_lock`, the session metadata clients display.
    pub fn summary(&self) -> Value {
        let mut summary = json!({ "model": self.model });
        if let Some(provider) = &self.provider {
            summary["provider"] = json!(provider);
        }
        if let Some(level) = self.thinking_level {
            summary["thinking_level"] = json!(level);
        }
        if let Some(account) = self.account() {
            summary["account"] = json!(account);
        }
        summary
    }
}

// ── small rules ──

fn not_found(what: &str) -> HarnessError {
    HarnessError::rejected("not_found", format!("{what} does not exist"), json!({}))
}

fn not_side_chat(session_id: &str) -> HarnessError {
    HarnessError::rejected(
        "not_side_chat",
        format!("session {session_id} is not a side chat"),
        json!({}),
    )
}

fn nothing_to_fork(session_id: &str) -> HarnessError {
    HarnessError::rejected(
        "nothing_to_fork",
        "A side chat sees the conversation up to a reply, and this one has no turn yet.",
        json!({ "session_id": session_id }),
    )
}

/// The conversation a side chat belongs to, from its metadata.
pub fn side_chat_parent(metadata: &Map<String, Value>) -> Option<&str> {
    metadata
        .get(SIDE_CHAT_OF)
        .and_then(Value::as_str)
        .filter(|parent| !parent.is_empty())
}

/// `{}` is no options (the router reads both the same).
fn normalized_provider_options(
    options: Option<&BTreeMap<String, Value>>,
) -> Option<BTreeMap<String, Value>> {
    options.filter(|map| !map.is_empty()).cloned()
}

/// An empty skills list is every skill, like none.
fn normalized_skills(skills: Option<&Vec<String>>) -> Option<Vec<String>> {
    skills.filter(|list| !list.is_empty()).cloned()
}

/// The working folder a send names: `Some(root)` when its metadata names
/// `fs_scope` (`Some(None)` for an explicit clear), `None` when it does not.
fn requested_root(options: &SendOptions) -> Option<Option<&str>> {
    let scope = options.metadata.as_ref()?.as_object()?.get(FS_SCOPE_KEY)?;
    Some(scope.get(FS_SCOPE_ROOT_KEY).and_then(Value::as_str))
}

/// The derived title of a first message: whitespace collapsed, cut at
/// [`TITLE_MAX_CHARS`] with an ellipsis (the clients' `deriveTitle`).
pub fn derive_title(text: &str) -> Option<String> {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if clean.is_empty() {
        return None;
    }
    if clean.chars().count() <= TITLE_MAX_CHARS {
        return Some(clean);
    }
    let cut: String = clean.chars().take(TITLE_MAX_CHARS).collect();
    Some(format!("{}…", cut.trim_end()))
}

fn message_text(message: &MessageInput) -> Option<String> {
    match message {
        MessageInput::Text(text) => Some(text.clone()),
        MessageInput::Message(message) => match message.as_ref() {
            AgentMessage::User(user) => Some(crate::types::content::ContentBlock::join_text(
                &user.content,
            )),
            _ => None,
        },
    }
}

/// The turn record a side chat starts from: the conversation's, finished,
/// under the side chat's id and a turn id of its own. Its first send
/// continues from it as from any previous turn — the same system prompt,
/// skills, policy, working folder and language — so the request it sends
/// is the conversation's. Nothing that belongs to the conversation's
/// running work travels: no calls, no parent, no wakes, no budget.
pub fn seeded_record(parent: &TurnRecord, session_id: &str) -> TurnRecord {
    let mut record = parent.clone();
    let now = AgentMessage::now_ms();
    record.session_id = session_id.to_string();
    record.turn_id = crate::ids::new_turn_id();
    record.status = TurnStatus::Completed;
    record.step = 0;
    record.turn_count = 0;
    record.recall_context = None;
    record.recall_sent = false;
    record.message_preview = None;
    record.abort = false;
    record.exchange_finished = false;
    record.continues = false;
    record.owes_completion = false;
    record.took_wakes.clear();
    record.watermark_entry_id = None;
    record.stream_request_id = None;
    record.calls.clear();
    record.parent = None;
    record.display_parent_session_id = None;
    record.failed_calls.clear();
    record.context_snapshot = None;
    record.result = None;
    record.result_error = None;
    record.stop_reason = None;
    record.failure = None;
    record.validation_retries = 0;
    record.transient_resumes = 0;
    record.ask_step = None;
    record.ask_seen_step = None;
    record.created_at = now;
    record.updated_at = now;
    // A side chat is a root of its own: unmetered unless it says otherwise.
    record.options.budget_root_session_id = None;
    record.options.max_total_tokens = None;
    record.options.max_cost_usd = None;
    // Only the working folder survives from the turn's own metadata.
    record.options.metadata = record
        .options
        .filesystem_root()
        .map(|root| json!({ FS_SCOPE_KEY: { FS_SCOPE_ROOT_KEY: root } }));
    record
}

// ── the lock in state ──

pub async fn load_lock(
    deps: &Deps,
    session_id: &str,
) -> Result<Option<SideChatLock>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let value = crate::state::state_get(&deps.iii, LOCK_SCOPE, session_id, timeout).await?;
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|e| HarnessError::State(format!("side chat lock {session_id}: {e}")))
}

async fn save_lock(deps: &Deps, lock: &SideChatLock) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let value = serde_json::to_value(lock)
        .map_err(|e| HarnessError::State(format!("side chat lock serialize: {e}")))?;
    crate::state::state_set(&deps.iii, LOCK_SCOPE, &lock.session_id, value, timeout).await
}

/// Forget a side chat's lock (`keep`, and deletion).
pub async fn delete_lock(deps: &Deps, session_id: &str) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    crate::state::state_delete(&deps.iii, LOCK_SCOPE, session_id, timeout).await
}

// ── reads ──

/// The side chats of `session_id`: the sessions whose `side_chat_of` names
/// it (`session::list` filtered on that key), in the store's order.
pub async fn side_chats_of(
    session: &SessionClient,
    session_id: &str,
) -> Result<Vec<SessionSummary>, HarnessError> {
    Ok(session
        .list_summaries(Some(json!({ SIDE_CHAT_OF: session_id })))
        .await?
        .into_iter()
        .filter(|chat| chat.meta_str(SIDE_CHAT_OF) == Some(session_id))
        .collect())
}

/// How many messages the fork copied, when the side chat records it.
pub fn forked_message_count(summary: &SessionSummary) -> Option<u64> {
    summary
        .metadata
        .as_ref()?
        .get(SIDE_CHAT_FORK_POINT)?
        .get("message_count")?
        .as_u64()
}

/// The conversation's entry a side chat was forked at, when recorded.
pub fn fork_entry_id(summary: &SessionSummary) -> Option<String> {
    summary
        .metadata
        .as_ref()?
        .get(SIDE_CHAT_FORK_POINT)?
        .get("entry_id")?
        .as_str()
        .map(str::to_string)
}

/// The messages added in the side chat after the fork (all of them when the
/// fork point is not recorded: a side chat forked before phase 2).
pub fn own_message_count(summary: &SessionSummary) -> u64 {
    summary
        .message_count
        .saturating_sub(forked_message_count(summary).unwrap_or(0))
}

/// Whether anything was sent in the side chat since the fork. A side chat
/// forked before phase 2 has no recorded count: its transcript is read for
/// a message after `side_chat_from`, and without that anchor it counts as
/// sent (it is never discarded on a guess).
async fn sent_in(session: &SessionClient, summary: &SessionSummary) -> Result<bool, HarnessError> {
    if let Some(base) = forked_message_count(summary) {
        return Ok(summary.message_count > base);
    }
    let Some(from) = summary.meta_str(SIDE_CHAT_FROM) else {
        return Ok(true);
    };
    let entries = session.messages_strict(&summary.session_id).await?;
    Ok(
        match entries.iter().position(|entry| entry.entry_id == from) {
            Some(at) => entries[at + 1..]
                .iter()
                .any(|entry| entry.message.is_some()),
            None => true,
        },
    )
}

/// The entry `open` forks at by default: the last user or assistant message
/// (the ADE's `lastSpokenEntry`), skipping the reply a running turn is still
/// streaming.
async fn last_spoken_entry(
    session: &SessionClient,
    session_id: &str,
    streaming: Option<&str>,
) -> Result<Option<String>, HarnessError> {
    let tail = session
        .call_raw(
            "session::messages-tail",
            json!({ "session_id": session_id, "limit": 50, "include_custom": false,
                    "include_image_data": false }),
        )
        .await?;
    Ok(tail
        .get("messages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .rev()
        .filter(|item| item["entry_id"].as_str() != streaming)
        .find(|item| matches!(item["message"]["role"].as_str(), Some("user" | "assistant")))
        .and_then(|item| item["entry_id"].as_str())
        .map(str::to_string))
}

// ── open ──

pub async fn open(
    deps: &Deps,
    req: SideChatOpenRequest,
) -> Result<SideChatOpenResponse, HarnessError> {
    let parent_id = req.session_id.trim();
    if parent_id.is_empty() {
        return Err(HarnessError::InvalidRequest(
            "session_id must not be empty".into(),
        ));
    }
    let session = deps.session().await;
    let parent = session
        .summary(parent_id)
        .await?
        .ok_or_else(|| not_found(&format!("session {parent_id}")))?;
    if parent
        .metadata
        .as_ref()
        .and_then(side_chat_parent)
        .is_some()
    {
        return Err(HarnessError::rejected(
            "nested_side_chat",
            "This is already a side chat. Ask in it directly.",
            json!({ "session_id": parent_id }),
        ));
    }
    crate::functions::delete_session_tree::ensure_live(deps, parent_id).await?;
    let timeout = deps.cfg().await.session_timeout_ms;
    let Some(record) = crate::state::get_turn(&deps.iii, parent_id, timeout).await? else {
        return Err(nothing_to_fork(parent_id));
    };
    let streaming = (!record.status.is_terminal())
        .then(|| crate::ids::assistant_entry_id(&record.turn_id, record.step));
    let entry_id = match req
        .entry_id
        .as_deref()
        .map(str::trim)
        .filter(|e| !e.is_empty())
    {
        Some(entry_id) => entry_id.to_string(),
        None => last_spoken_entry(&session, parent_id, streaming.as_deref())
            .await?
            .ok_or_else(|| nothing_to_fork(parent_id))?,
    };
    let forked = deps
        .iii
        .call(CallRequest {
            function_id: "session::fork".into(),
            payload: json!({ "session_id": parent_id, "entry_id": entry_id, "title": TITLE }),
            action: None,
            timeout_ms: Some(timeout),
        })
        .await
        .map_err(|error| match error {
            iii_sdk::Error::Remote { code, .. } if code.contains("not_found") => {
                not_found(&format!("entry {entry_id} of session {parent_id}"))
            }
            error => HarnessError::Dependency(format!("session::fork: {error}")),
        })?;
    let side_chat_id = forked
        .get("session_id")
        .and_then(Value::as_str)
        .ok_or_else(|| HarnessError::Dependency("session::fork returned no session_id".into()))?
        .to_string();
    let copied = forked["meta"]["message_count"].as_u64().unwrap_or(0);
    let tagged = tag(
        deps,
        &session,
        &record,
        &side_chat_id,
        &entry_id,
        copied,
        req.quote,
    )
    .await;
    match tagged {
        Ok(meta) => Ok(SideChatOpenResponse {
            session_id: side_chat_id,
            meta,
        }),
        Err(error) => {
            // Untagged, the fork would show up as an ordinary conversation.
            discard_untagged(deps, &session, &side_chat_id).await;
            Err(error)
        }
    }
}

/// Lock and tag a fresh fork; returns its session record.
async fn tag(
    deps: &Deps,
    session: &SessionClient,
    parent: &TurnRecord,
    side_chat_id: &str,
    entry_id: &str,
    copied: u64,
    quote: Option<SideChatQuote>,
) -> Result<Value, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    // The fork's copy of the entry it was cut at: its last copied message.
    let from = session
        .call_raw(
            "session::messages-tail",
            json!({ "session_id": side_chat_id, "limit": 1, "include_custom": false,
                    "include_image_data": false }),
        )
        .await?
        .get("messages")
        .and_then(Value::as_array)
        .and_then(|messages| messages.last())
        .and_then(|item| item["entry_id"].as_str())
        .map(str::to_string);
    let lock = SideChatLock::from_options(
        side_chat_id,
        &parent.session_id,
        Some(entry_id.to_string()),
        &parent.options,
        "open",
    );
    crate::state::put_turn(&deps.iii, &seeded_record(parent, side_chat_id), timeout).await?;
    save_lock(deps, &lock).await?;
    let mut patch = MetaPatch {
        description: Some(String::new()),
        ..MetaPatch::default()
    };
    patch
        .set
        .insert(SIDE_CHAT_OF.into(), json!(parent.session_id));
    if let Some(from) = from {
        patch.set.insert(SIDE_CHAT_FROM.into(), json!(from));
    }
    patch.set.insert(SIDE_CHAT_LOCK.into(), lock.summary());
    patch.set.insert(
        SIDE_CHAT_FORK_POINT.into(),
        json!({ "entry_id": entry_id, "message_count": copied }),
    );
    patch.unset = SPAWN_LINK_KEYS
        .iter()
        .chain(OWN_NAME_KEYS.iter())
        .chain([SIDE_CHAT_SUMMARIES].iter())
        .map(|key| key.to_string())
        .collect();
    // A kept side chat that is now a conversation still carries its own
    // quotes and summaries; a side chat of it starts without them.
    match quote {
        Some(quote) => {
            patch.set.insert(SIDE_CHAT_QUOTES.into(), json!([quote]));
        }
        None => patch.unset.push(SIDE_CHAT_QUOTES.into()),
    }
    session.patch_meta(side_chat_id, &patch).await
}

/// Best effort: a fork `open` could not tag goes, with what it wrote.
async fn discard_untagged(deps: &Deps, session: &SessionClient, side_chat_id: &str) {
    let timeout = deps.cfg().await.session_timeout_ms;
    if let Err(error) = session
        .call_raw("session::delete", json!({ "session_id": side_chat_id }))
        .await
    {
        tracing::warn!(session_id = side_chat_id, %error, "could not delete an untagged side chat fork");
    }
    let _ = crate::state::delete_turn(&deps.iii, side_chat_id, timeout).await;
    let _ = delete_lock(deps, side_chat_id).await;
}

// ── the lock on send ──

/// What `harness::send` does after delivering a side chat's message.
#[derive(Debug, Default)]
pub(crate) struct SideChatSend {
    /// The title its first message gives it.
    pub retitle: Option<String>,
}

/// Enforce a side chat's lock on a send: a request naming another value
/// than the lock's fails `side_chat_locked`; the fields it leaves out run on
/// the lock's values, written into the request when the session's last turn
/// ran on others (a side chat locked after the fact). A side chat forked
/// before phase 2 is locked here, to its conversation's options now.
/// `None` for a session that is not a side chat.
pub(crate) async fn prepare_send(
    deps: &Deps,
    session_id: &str,
    metadata: &Map<String, Value>,
    req: &mut SendRequest,
    caller_holds_session_lock: bool,
) -> Result<Option<SideChatSend>, HarnessError> {
    let Some(parent) = side_chat_parent(metadata) else {
        return Ok(None);
    };
    let timeout = deps.cfg().await.session_timeout_ms;
    let lock = match load_lock(deps, session_id).await? {
        Some(lock) => Some(lock),
        None => {
            let _guard = if caller_holds_session_lock {
                None
            } else {
                Some(deps.locks.guard(session_id).await)
            };
            lazy_lock(deps, session_id, parent).await?
        }
    };
    if let Some(lock) = &lock {
        check_lock(lock, req)?;
        let prev = crate::state::get_turn_unhydrated(&deps.iii, session_id, timeout).await?;
        fill_from_lock(lock, prev.as_ref(), req);
    }
    let summary_turn = req
        .options
        .as_ref()
        .and_then(|o| o.metadata.as_ref())
        .and_then(|m| m.get(SUMMARY_TURN_KEY))
        == Some(&json!(true));
    let mut out = SideChatSend::default();
    if !summary_turn && metadata.get("title_manual") != Some(&json!(true)) {
        let title = deps
            .session()
            .await
            .summary(session_id)
            .await?
            .map(|summary| summary.title);
        if title.as_deref() == Some(TITLE) {
            out.retitle = message_text(&req.message).as_deref().and_then(derive_title);
        }
    }
    Ok(Some(out))
}

/// Lock a side chat forked before phase 2 to its conversation's options now,
/// and give it the conversation's turn record to start from when it has no
/// turn of its own. `None` when the conversation has no turn record (the
/// side chat then runs unlocked, as before).
async fn lazy_lock(
    deps: &Deps,
    session_id: &str,
    parent: &str,
) -> Result<Option<SideChatLock>, HarnessError> {
    if let Some(lock) = load_lock(deps, session_id).await? {
        return Ok(Some(lock));
    }
    let timeout = deps.cfg().await.session_timeout_ms;
    let Some(record) = crate::state::get_turn(&deps.iii, parent, timeout).await? else {
        tracing::warn!(
            session_id,
            parent,
            "side chat's conversation has no turn record; not locked"
        );
        return Ok(None);
    };
    let lock = SideChatLock::from_options(session_id, parent, None, &record.options, "lazy");
    if crate::state::get_turn_unhydrated(&deps.iii, session_id, timeout)
        .await?
        .is_none()
    {
        crate::state::put_turn(&deps.iii, &seeded_record(&record, session_id), timeout).await?;
    }
    save_lock(deps, &lock).await?;
    let display = MetaPatch::set(SIDE_CHAT_LOCK, lock.summary()).quiet();
    if let Err(error) = deps.session().await.patch_meta(session_id, &display).await {
        tracing::warn!(session_id, %error, "could not show the side chat's lock");
    }
    Ok(Some(lock))
}

fn locked(field: &str, sent: Value, lock: Value) -> HarnessError {
    HarnessError::rejected(
        "side_chat_locked",
        LOCKED_MESSAGE,
        json!({ "field": field, "sent": sent, "locked": lock }),
    )
}

/// A send that names a different value than the lock's for the model, the
/// provider, the effort (`thinking_level`, `provider_options`, `reasoning`),
/// the functions, the skills or the working folder fails. So does one that
/// names a prompt field or an agent: the side chat keeps the conversation's
/// system prompt. Naming the lock's own value is accepted.
pub fn check_lock(lock: &SideChatLock, req: &SendRequest) -> Result<(), HarnessError> {
    if let Some(model) = req.model.as_deref()
        && model != lock.model
    {
        return Err(locked("model", json!(model), json!(lock.model)));
    }
    if let Some(provider) = req.provider.as_deref()
        && Some(provider) != lock.provider.as_deref()
    {
        return Err(locked("provider", json!(provider), json!(lock.provider)));
    }
    let Some(options) = req.options.as_ref() else {
        return Ok(());
    };
    if let Some(level) = options.thinking_level
        && Some(level) != lock.thinking_level
    {
        return Err(locked(
            "thinking_level",
            json!(level),
            json!(lock.thinking_level),
        ));
    }
    if let Some(sent) = options.provider_options.as_ref()
        && normalized_provider_options(Some(sent)) != lock.provider_options
    {
        return Err(locked(
            "provider_options",
            json!(sent),
            json!(lock.provider_options),
        ));
    }
    if let Some(reasoning) = options.reasoning {
        return Err(locked(
            "reasoning",
            json!(reasoning),
            json!(lock.thinking_level),
        ));
    }
    if let Some(functions) = options.functions.as_ref()
        && Some(functions) != lock.functions.as_ref()
    {
        return Err(locked("functions", json!(functions), json!(lock.functions)));
    }
    if let Some(skills) = options.skills.as_ref()
        && normalized_skills(Some(skills)) != lock.skills
    {
        return Err(locked("skills", json!(skills), json!(lock.skills)));
    }
    if let Some(root) = requested_root(options)
        && root != lock.root()
    {
        return Err(locked("fs_scope", json!(root), json!(lock.root())));
    }
    for (field, named) in [
        ("agent", options.agent.is_some()),
        ("system_prompt", options.system_prompt.is_some()),
        (
            "system_prompt_strategy",
            options.system_prompt_strategy.is_some(),
        ),
    ] {
        if named {
            return Err(locked(field, json!(true), Value::Null));
        }
    }
    Ok(())
}

/// Write the lock's values into the fields a send leaves out, where the
/// session's last turn ran on others. A side chat `open` made starts from
/// the conversation's own record, so this writes nothing there; it matters
/// for a side chat locked after the fact.
pub fn fill_from_lock(lock: &SideChatLock, prev: Option<&TurnRecord>, req: &mut SendRequest) {
    let prev = prev.map(|record| (&record.options, record.status.is_terminal()));
    if req.model.is_none()
        && prev.is_none_or(|(o, _)| o.model != lock.model || o.provider != lock.provider)
    {
        req.model = Some(lock.model.clone());
        if req.provider.is_none() {
            req.provider = lock.provider.clone();
        }
    }
    let options = req.options.get_or_insert_with(SendOptions::default);
    if options.thinking_level.is_none()
        && options.provider_options.is_none()
        && options.reasoning.is_none()
        && prev.is_none_or(|(o, _)| {
            o.thinking_level != lock.thinking_level
                || normalized_provider_options(o.provider_options.as_ref()) != lock.provider_options
        })
    {
        options.thinking_level = lock.thinking_level;
        // `{}` is the explicit "no provider options", never the provider default.
        options.provider_options = Some(lock.provider_options.clone().unwrap_or_default());
    }
    if options.functions.is_none() && prev.is_none_or(|(o, _)| o.functions != lock.functions) {
        options.functions = Some(lock.functions.clone().unwrap_or_default());
    }
    if requested_root(options).is_none()
        && let Some(root) = lock.root()
        && prev.is_none_or(|(o, _)| o.filesystem_root() != Some(root))
    {
        let metadata = options
            .metadata
            .get_or_insert_with(|| Value::Object(Map::new()));
        if let Some(map) = metadata.as_object_mut() {
            map.insert(FS_SCOPE_KEY.into(), json!({ FS_SCOPE_ROOT_KEY: root }));
        }
    }
    // The skills filter changes only between turns, on a session that has a
    // skills index (`harness::send`'s own rule).
    if options.skills.is_none()
        && let Some((o, terminal)) = prev
        && terminal
        && let Some(context) = o.skill_context.as_ref()
        && normalized_skills(context.filter.as_ref()) != lock.skills
    {
        options.skills = Some(lock.skills.clone().unwrap_or_default());
    }
}

/// After the side chat's first message landed: name it after the message
/// (best effort; the send already succeeded).
pub(crate) async fn after_send(deps: &Deps, session_id: &str, prepared: SideChatSend) {
    let Some(title) = prepared.retitle else {
        return;
    };
    let patch = MetaPatch {
        title: Some(title),
        ..MetaPatch::default()
    };
    if let Err(error) = deps.session().await.patch_meta(session_id, &patch).await {
        tracing::warn!(session_id, %error, "could not name the side chat after its first message");
    }
}

// ── close, keep, summarize ──

async fn side_chat(
    session: &SessionClient,
    session_id: &str,
) -> Result<(SessionSummary, String), HarnessError> {
    let summary = session
        .summary(session_id)
        .await?
        .ok_or_else(|| not_found(&format!("session {session_id}")))?;
    let parent = summary
        .metadata
        .as_ref()
        .and_then(side_chat_parent)
        .map(str::to_string)
        .ok_or_else(|| not_side_chat(session_id))?;
    Ok((summary, parent))
}

pub async fn close(
    deps: &Deps,
    req: SideChatRequest,
) -> Result<SideChatCloseResponse, HarnessError> {
    let session = deps.session().await;
    let (summary, _) = side_chat(&session, &req.session_id).await?;
    if sent_in(&session, &summary).await? {
        return Ok(SideChatCloseResponse {
            deleted: false,
            operation_id: None,
        });
    }
    // The durable deletion: a tombstone first, so a late send cannot
    // recreate it, then the session and everything the harness kept for it.
    let snapshot = crate::functions::delete_session_tree::handle(
        deps,
        crate::functions::delete_session_tree::DeleteRequest {
            session_id: req.session_id.clone(),
        },
    )
    .await?;
    if snapshot.status == crate::functions::delete_session_tree::DeletionStatus::Failed {
        return Err(HarnessError::Dependency(format!(
            "side chat {} not deleted: {}",
            req.session_id,
            snapshot.error.unwrap_or_default()
        )));
    }
    Ok(SideChatCloseResponse {
        deleted: true,
        operation_id: Some(snapshot.operation_id),
    })
}

pub async fn keep(deps: &Deps, req: SideChatRequest) -> Result<SideChatKeepResponse, HarnessError> {
    let session = deps.session().await;
    let (_, parent) = side_chat(&session, &req.session_id).await?;
    let parent = session.summary(&parent).await?;
    // Membership writes keep `updated_at` (D50): the kept conversation sorts
    // by its own last activity.
    let mut patch = MetaPatch::default().quiet();
    patch.unset = [
        SIDE_CHAT_OF,
        SIDE_CHAT_FROM,
        SIDE_CHAT_LOCK,
        SIDE_CHAT_FORK_POINT,
    ]
    .iter()
    .map(|key| key.to_string())
    .collect();
    if let Some(parent) = parent {
        match parent.meta_str(crate::projects::METADATA_KEY) {
            Some(project) => {
                patch
                    .set
                    .insert(crate::projects::METADATA_KEY.into(), json!(project));
            }
            None => patch.unset.push(crate::projects::METADATA_KEY.into()),
        }
    }
    let meta = session.patch_meta(&req.session_id, &patch).await?;
    delete_lock(deps, &req.session_id).await?;
    Ok(SideChatKeepResponse { meta })
}

pub async fn summarize(
    deps: &Deps,
    req: SideChatRequest,
) -> Result<SideChatSummarizeResponse, HarnessError> {
    let session = deps.session().await;
    side_chat(&session, &req.session_id).await?;
    let timeout = deps.cfg().await.session_timeout_ms;
    if crate::state::get_turn_unhydrated(&deps.iii, &req.session_id, timeout)
        .await?
        .is_some_and(|record| !record.status.is_terminal())
    {
        // Merged into the running turn, the prompt would not be a turn of
        // its own and its reply would never be marked as the summary.
        return Err(HarnessError::rejected(
            "side_chat_busy",
            "Wait for the side chat's reply, then ask for a summary.",
            json!({ "session_id": req.session_id }),
        ));
    }
    let sent = crate::functions::send::handle(
        deps,
        SendRequest {
            session_id: Some(req.session_id.clone()),
            message: MessageInput::Text(SUMMARY_PROMPT.to_string()),
            model: None,
            provider: None,
            idempotency_key: None,
            session: None,
            options: Some(SendOptions {
                metadata: Some(json!({ SUMMARY_TURN_KEY: true })),
                ..SendOptions::default()
            }),
        },
    )
    .await?;
    Ok(SideChatSummarizeResponse {
        turn_id: sent.turn_id,
    })
}

/// At the end of a `summarize` turn: append its reply's entry id (the last
/// assistant message the turn wrote) to `side_chat_summaries`, keeping the
/// newest [`SUMMARIES_KEPT`]. Best effort and idempotent on a redelivered
/// step; any other turn is left alone.
pub async fn record_summary_reply(session: &SessionClient, record: &TurnRecord) {
    let summary_turn = record
        .options
        .metadata
        .as_ref()
        .and_then(|m| m.get(SUMMARY_TURN_KEY))
        == Some(&json!(true));
    if !summary_turn {
        return;
    }
    if let Err(error) = append_summary(session, record).await {
        tracing::warn!(session_id = %record.session_id, %error, "could not record the side chat's summary reply");
    }
}

async fn append_summary(session: &SessionClient, record: &TurnRecord) -> Result<(), HarnessError> {
    let prefix = format!("e_{}_", record.turn_id);
    let entries = session.messages_strict(&record.session_id).await?;
    let Some(reply) = entries.iter().rev().find(|entry| {
        entry.entry_id.starts_with(&prefix)
            && matches!(entry.message, Some(AgentMessage::Assistant(_)))
    }) else {
        return Ok(());
    };
    let Some(summary) = session.summary(&record.session_id).await? else {
        return Ok(());
    };
    let mut ids: Vec<String> = summary
        .metadata
        .as_ref()
        .and_then(|m| m.get(SIDE_CHAT_SUMMARIES))
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_string)
        .collect();
    if ids.contains(&reply.entry_id) {
        return Ok(());
    }
    ids.push(reply.entry_id.clone());
    let skip = ids.len().saturating_sub(SUMMARIES_KEPT);
    let ids: Vec<String> = ids.into_iter().skip(skip).collect();
    session
        .patch_meta(
            &record.session_id,
            &MetaPatch::set(SIDE_CHAT_SUMMARIES, json!(ids)),
        )
        .await
        .map(drop)
}

#[cfg(test)]
mod tests;
