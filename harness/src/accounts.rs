//! Claude Code accounts in the Chat (chat-projects phase 4), all in one
//! module (D30's seam):
//!
//! - **Resolution (D139).** The account a `claude-code` turn runs on, decided
//!   once when the turn starts and frozen on it: the session's
//!   `metadata.claude_account`, else its project's pinned `account`, else the
//!   account tmux makes global now. Always a concrete id, written into
//!   `provider_options["claude-code"].account` after every other writer of
//!   the options; a client's own `account` never reaches the provider.
//! - **`harness::retry` (D142).** Re-runs a session's failed last turn from
//!   its transcript, on a newly resolved account.
//! - **The usage ledger (D133).** `claude-code` models carry no prices, so
//!   their turns carry no cost: the harness counts their tokens by model,
//!   per account, project and day.
//! - **`harness::metrics::account-split`.** Each tmux account's weekly
//!   percentage split by project, priced from the ledger and from what
//!   tmux measured outside the Chat.
//!
//! tmux owns the accounts (`tmux::accounts::list`, `tmux::usage::read`,
//! `tmux::usage::claude_outside`). With no tmux worker on the engine, no
//! account is set and the provider keeps its own single sign-in.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicI64, Ordering};

use iii_sdk::protocol::CallRequest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::clients::SessionClient;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::projects::Project;
use crate::types::message::{AgentMessage, AssistantMessage};
use crate::types::turn::{ParentLink, TurnOptions, TurnRecord, TurnStatus};

/// The provider whose turns run on a tmux account.
pub const PROVIDER: &str = "claude-code";
/// The provider option naming the account (`provider_options["claude-code"].account`).
pub const ACCOUNT_OPTION: &str = "account";
/// Session metadata a client writes (`session::patch-meta`) to run a
/// conversation on one account: an account id, or [`GLOBAL`].
pub const SESSION_KEY: &str = "claude_account";
/// [`SESSION_KEY`]'s value for "follow the global account".
pub const GLOBAL: &str = "global";

pub const UNKNOWN_ACCOUNT_CODE: &str = "harness.unknown_account";
pub const USAGE_LIMIT_CODE: &str = "llm.usage_limit";

const ACCOUNTS_LIST_ID: &str = "tmux::accounts::list";
const USAGE_READ_ID: &str = "tmux::usage::read";
const OUTSIDE_ID: &str = "tmux::usage::claude_outside";
const ROUTER_MODEL_GET_ID: &str = "router::models::get";
/// The catalog whose list prices price the ledger (`anthropic/<model>`).
const PRICE_PROVIDER: &str = "anthropic";

const TMUX_TIMEOUT_MS: u64 = 10_000;
/// Reading the week's transcripts outside the Chat can take a while.
const OUTSIDE_TIMEOUT_MS: u64 = 30_000;
const CATALOG_TIMEOUT_MS: u64 = 5_000;

const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;
const WEEK_MS: i64 = 7 * DAY_MS;

// ── the turn's account ──

/// Why a turn runs on its account (contract D).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AccountSource {
    /// The conversation names it (`metadata.claude_account`).
    Session,
    /// The conversation's project is pinned to it.
    Project,
    /// It is the global account (nothing pins the conversation, or it
    /// names `"global"`).
    Global,
}

/// `account {id, source}`: what the verbose `harness::status` and the turn
/// events show.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AccountRef {
    /// The tmux account id.
    pub id: String,
    pub source: AccountSource,
}

/// The account a turn runs on, as its record keeps it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnAccount {
    /// The tmux account id.
    pub id: String,
    pub source: AccountSource,
    /// The project the turn's tokens count under in the usage ledger: the
    /// session's when the turn started (a sub-agent's is its parent's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
}

impl TurnAccount {
    pub fn public(&self) -> AccountRef {
        AccountRef {
            id: self.id.clone(),
            source: self.source,
        }
    }
}

/// Whether a turn with these options runs on the `claude-code` provider.
pub fn runs_on_claude_code(options: &TurnOptions) -> bool {
    match options.provider.as_deref() {
        Some(provider) => provider == PROVIDER,
        None => options
            .model
            .strip_prefix(PROVIDER)
            .is_some_and(|rest| rest.starts_with('/')),
    }
}

/// The account `provider_options["claude-code"]` names.
pub fn account_option(options: Option<&BTreeMap<String, Value>>) -> Option<&str> {
    options?
        .get(PROVIDER)?
        .get(ACCOUNT_OPTION)?
        .as_str()
        .filter(|id| !id.is_empty())
}

/// Write the account into `provider_options["claude-code"]`, replacing any.
pub fn set_account_option(options: &mut Option<BTreeMap<String, Value>>, id: &str) {
    let namespace = options
        .get_or_insert_with(BTreeMap::new)
        .entry(PROVIDER.to_string())
        .or_insert_with(|| json!({}));
    if !namespace.is_object() {
        *namespace = json!({});
    }
    if let Some(namespace) = namespace.as_object_mut() {
        namespace.insert(ACCOUNT_OPTION.into(), json!(id));
    }
}

/// Drop `provider_options["claude-code"].account`, and the namespace when it
/// held nothing else. Whether anything was dropped.
pub fn strip_account_option(options: &mut BTreeMap<String, Value>) -> bool {
    let Some(namespace) = options.get_mut(PROVIDER).and_then(Value::as_object_mut) else {
        return false;
    };
    if namespace.remove(ACCOUNT_OPTION).is_none() {
        return false;
    }
    if namespace.is_empty() {
        options.remove(PROVIDER);
    }
    true
}

/// The options without the account, for comparing two sets of options
/// (a side chat's lock, D80): empty namespaces say nothing, and no options
/// at all is `None`.
pub fn without_account(
    options: Option<&BTreeMap<String, Value>>,
) -> Option<BTreeMap<String, Value>> {
    let mut options = options.cloned()?;
    strip_account_option(&mut options);
    options.retain(|_, value| value.as_object().is_none_or(|object| !object.is_empty()));
    (!options.is_empty()).then_some(options)
}

// ── tmux ──

/// `tmux::accounts::list`'s answer, and `tmux::usage::read`'s `claude`:
/// only what the harness reads (field names from `tmux/src/model.rs`).
#[derive(Debug, Clone, Default, Deserialize)]
struct TmuxClaude {
    /// A `claude` binary or `~/.claude` exists.
    #[serde(default)]
    available: bool,
    /// The global account's id.
    #[serde(default)]
    global: String,
    #[serde(default)]
    accounts: Vec<TmuxAccount>,
}

#[derive(Debug, Clone, Deserialize)]
struct TmuxAccount {
    id: String,
    #[serde(default)]
    usage: Option<TmuxUsage>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TmuxUsage {
    #[serde(default)]
    week: Option<TmuxLimit>,
}

#[derive(Debug, Clone, Default, Deserialize)]
struct TmuxLimit {
    #[serde(default)]
    used_percent: f64,
    #[serde(default)]
    resets_at_ms: Option<u64>,
    /// The window reset after the reading: `used_percent` is stale.
    #[serde(default)]
    reset_since_read: bool,
}

impl TmuxClaude {
    fn has(&self, id: &str) -> bool {
        self.accounts.iter().any(|account| account.id == id)
    }
}

/// Call a tmux function; `Ok(None)` when the engine has no such function
/// (no tmux worker, or one that predates it).
async fn call_optional(
    deps: &Deps,
    function_id: &str,
    payload: Value,
    timeout_ms: u64,
) -> Result<Option<Value>, String> {
    let result = deps
        .iii
        .call(CallRequest {
            function_id: function_id.into(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await;
    match result {
        Ok(value) => Ok(Some(value)),
        Err(error) if is_not_registered(&error) => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

fn is_not_registered(error: &iii_sdk::errors::Error) -> bool {
    if let iii_sdk::errors::Error::Remote { code, .. } = error
        && (code == "function_not_found" || code == "NOT_FOUND")
    {
        return true;
    }
    error.to_string().contains("function_not_found")
}

/// tmux's accounts, or `None` when there is no tmux to run a turn on one
/// (absent, unreadable, or no Claude Code on its computer).
async fn tmux_accounts(deps: &Deps) -> Option<TmuxClaude> {
    match call_optional(deps, ACCOUNTS_LIST_ID, json!({}), TMUX_TIMEOUT_MS).await {
        Ok(Some(value)) => match serde_json::from_value::<TmuxClaude>(value) {
            Ok(list) if list.available => Some(list),
            Ok(_) => None,
            Err(error) => {
                tracing::warn!(%error, "tmux::accounts::list answered an unknown shape; no account set");
                None
            }
        },
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(%error, "tmux::accounts::list failed; the turn runs with no account set");
            None
        }
    }
}

// ── resolution ──

/// What resolution decided for one turn.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolved {
    /// No account: no tmux to run on one.
    None,
    Account(TurnAccount),
    /// The pin names an account tmux no longer has.
    Unknown(TurnAccount),
}

/// What [`resolve_turn`] did to the record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnResolution {
    /// Nothing changed.
    Unchanged,
    /// The record changed (its account, or its provider options): store it.
    Changed,
    /// The turn is pinned to an account that no longer exists: fail it with
    /// [`UNKNOWN_ACCOUNT_CODE`]. `record.account` names that account.
    Unknown,
}

/// The pin a session's metadata and its project make, and the project.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Pin {
    account: Option<(String, AccountSource)>,
    project_id: Option<String>,
}

fn project_id_of(metadata: &Map<String, Value>) -> Option<&str> {
    metadata
        .get(crate::projects::METADATA_KEY)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && *id != crate::projects::NONE)
}

/// The project's pinned account id, when `account` is `{mode: "pinned", account_id}`.
pub fn pinned_account(account: &Value) -> Option<&str> {
    (account.get("mode").and_then(Value::as_str) == Some("pinned"))
        .then(|| account.get("account_id").and_then(Value::as_str))
        .flatten()
        .filter(|id| !id.is_empty())
}

/// The order of D139: the session's own account, else its project's pin.
/// `"global"` in the session follows the global even in a pinned project.
fn pin_of(metadata: &Map<String, Value>, projects: &[Project]) -> Pin {
    let project =
        project_id_of(metadata).and_then(|id| projects.iter().find(|project| project.id == id));
    let project_id = project.map(|project| project.id.clone());
    let session = metadata
        .get(SESSION_KEY)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let account = match session {
        Some(GLOBAL) => None,
        Some(id) => Some((id.to_string(), AccountSource::Session)),
        None => project
            .and_then(|project| pinned_account(&project.account))
            .map(|id| (id.to_string(), AccountSource::Project)),
    };
    Pin {
        account,
        project_id,
    }
}

fn pick(list: &TmuxClaude, pin: Pin) -> Resolved {
    match pin.account {
        Some((id, source)) => {
            let account = TurnAccount {
                id,
                source,
                project_id: pin.project_id,
            };
            if list.has(&account.id) {
                Resolved::Account(account)
            } else {
                Resolved::Unknown(account)
            }
        }
        None if list.global.is_empty() => Resolved::None,
        None => Resolved::Account(TurnAccount {
            id: list.global.clone(),
            source: AccountSource::Global,
            project_id: pin.project_id,
        }),
    }
}

/// The projects, for a session that names one (best effort: an unreadable
/// store pins nothing).
async fn projects_for(deps: &Deps, metadata: &Map<String, Value>) -> Vec<Project> {
    if project_id_of(metadata).is_none() {
        return Vec::new();
    }
    let path = deps.cfg().await.projects_file_path.clone();
    match deps.projects.list(&path).await {
        Ok(document) => document.projects,
        Err(error) => {
            tracing::warn!(%error, "projects unreadable; the turn's account ignores its project");
            Vec::new()
        }
    }
}

async fn resolve_session(deps: &Deps, metadata: &Map<String, Value>) -> Resolved {
    let Some(list) = tmux_accounts(deps).await else {
        return Resolved::None;
    };
    let projects = projects_for(deps, metadata).await;
    pick(&list, pin_of(metadata, &projects))
}

/// A sub-agent runs on its parent's account (it inherited the parent's
/// provider options, the account among them): only the source and the
/// project are read back from the parent's record.
async fn inherited(deps: &Deps, parent: &ParentLink, id: &str) -> TurnAccount {
    let timeout = deps.cfg().await.session_timeout_ms;
    let parent_account = crate::state::get_turn_unhydrated(&deps.iii, &parent.session_id, timeout)
        .await
        .ok()
        .flatten()
        .and_then(|record| record.account)
        .filter(|account| account.id == id);
    parent_account.unwrap_or_else(|| TurnAccount {
        id: id.to_string(),
        source: AccountSource::Session,
        project_id: None,
    })
}

/// A side chat runs on its lock's account (D80): it skips resolution. A lock
/// made before phase 4 has none; the side chat's next turn takes the
/// conversation's resolution then and stores it in the lock.
async fn side_chat_account(
    deps: &Deps,
    session: &SessionClient,
    session_id: &str,
    parent: &str,
    metadata: &Map<String, Value>,
) -> Result<Resolved, HarnessError> {
    let Some(mut lock) = crate::side_chat::load_lock(deps, session_id).await? else {
        // Unlocked (its conversation had no turn), it runs as a conversation.
        return Ok(resolve_session(deps, metadata).await);
    };
    let Some(list) = tmux_accounts(deps).await else {
        return Ok(Resolved::None);
    };
    let locked = lock.account.clone().or_else(|| {
        lock.account_id().map(|id| TurnAccount {
            id: id.to_string(),
            source: AccountSource::Session,
            project_id: project_id_of(metadata).map(str::to_string),
        })
    });
    if let Some(account) = locked {
        return Ok(if list.has(&account.id) {
            Resolved::Account(account)
        } else {
            Resolved::Unknown(account)
        });
    }
    let parent_metadata = session.metadata_of(parent).await?.unwrap_or_default();
    let projects = projects_for(deps, &parent_metadata).await;
    let resolved = pick(&list, pin_of(&parent_metadata, &projects));
    if let Resolved::Account(account) = &resolved {
        lock.account = Some(account.clone());
        set_account_option(&mut lock.provider_options, &account.id);
        crate::side_chat::relock(deps, &lock).await?;
    }
    Ok(resolved)
}

/// Resolve the account of a turn that is starting (its first step), once:
/// a record that has one keeps it. Writes the account into the turn's
/// provider options and `record.account`; a turn that is not on
/// `claude-code`, or has no tmux to run on, gets none (an inherited one is
/// dropped).
pub async fn resolve_turn(
    deps: &Deps,
    session: &SessionClient,
    record: &mut TurnRecord,
) -> Result<TurnResolution, HarnessError> {
    if record.account.is_some() {
        return Ok(TurnResolution::Unchanged);
    }
    let options = &mut record.options;
    if !runs_on_claude_code(options) {
        let stripped = options
            .provider_options
            .as_mut()
            .is_some_and(strip_account_option);
        return Ok(if stripped {
            TurnResolution::Changed
        } else {
            TurnResolution::Unchanged
        });
    }
    let inherited_id = account_option(options.provider_options.as_ref()).map(str::to_string);
    let resolved = match (record.parent.as_ref(), inherited_id) {
        (Some(parent), Some(id)) => Resolved::Account(inherited(deps, parent, &id).await),
        _ => {
            let metadata = session
                .metadata_of(&record.session_id)
                .await?
                .unwrap_or_default();
            match crate::side_chat::side_chat_parent(&metadata) {
                Some(parent) => {
                    side_chat_account(deps, session, &record.session_id, parent, &metadata).await?
                }
                None => resolve_session(deps, &metadata).await,
            }
        }
    };
    let options = &mut record.options;
    let stripped = options
        .provider_options
        .as_mut()
        .is_some_and(strip_account_option);
    match resolved {
        Resolved::None => Ok(if stripped {
            TurnResolution::Changed
        } else {
            TurnResolution::Unchanged
        }),
        Resolved::Account(account) => {
            set_account_option(&mut options.provider_options, &account.id);
            record.account = Some(account);
            Ok(TurnResolution::Changed)
        }
        Resolved::Unknown(account) => {
            record.account = Some(account);
            Ok(TurnResolution::Unknown)
        }
    }
}

/// The public message of a turn pinned to an account that is gone.
pub fn unknown_account_message(id: &str) -> String {
    format!("The Claude Code account {id} no longer exists.")
}

// ── harness::retry ──

pub const RETRY_ID: &str = "harness::retry";
pub const RETRY_DESC: &str = "Re-run a session's last turn, which must have failed (e.g. on a Claude \
     Code account's usage limit), from its transcript: no new user message, the failed turn's \
     options and a newly resolved Claude Code account (chat-projects D142). When the failed turn left \
     partial output, the turn resumes from it. Refused with `nothing_to_retry` when the last turn did \
     not fail, and `busy` while a turn runs. Returns the new turn's id.";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RetryResponse {
    /// The turn that re-runs the failed one.
    pub turn_id: String,
}

fn nothing_to_retry(session_id: &str) -> HarnessError {
    HarnessError::rejected(
        "nothing_to_retry",
        "There is nothing to retry: the conversation's last turn did not fail.",
        json!({ "session_id": session_id }),
    )
}

fn busy(session_id: &str) -> HarnessError {
    HarnessError::rejected(
        "busy",
        "The conversation is still working. Try again when it stops.",
        json!({ "session_id": session_id }),
    )
}

/// What a retry decides from the session's last turn.
fn retry_gate(record: Option<&TurnRecord>, session_id: &str) -> Result<(), HarnessError> {
    let Some(record) = record else {
        return Err(nothing_to_retry(session_id));
    };
    if !record.status.is_terminal() || record.continues || record.owes_completion {
        return Err(busy(session_id));
    }
    if record.status != TurnStatus::Failed {
        return Err(nothing_to_retry(session_id));
    }
    Ok(())
}

/// `harness::retry`: a new turn of the session from its transcript, as the
/// transient resume runs one (no new user message; the resume nudge when
/// the failed turn left partial output), seeded with the failed turn's
/// options and lineage. Its first step resolves the account anew.
pub async fn retry(deps: &Deps, req: RetryRequest) -> Result<RetryResponse, HarnessError> {
    let session_id = req.session_id.as_str();
    crate::functions::delete_session_tree::ensure_live(deps, session_id).await?;
    let cfg = deps.cfg().await;
    let session = deps.session().await;
    if let Some(metadata) = session.metadata_of(session_id).await? {
        crate::functions::send::ensure_writable(&metadata, session_id)?;
    }
    // The turn's first step takes the same lock: the nudge lands before it reads.
    let _guard = deps.locks.guard(session_id).await;
    let failed = crate::state::get_turn(&deps.iii, session_id, cfg.session_timeout_ms).await?;
    retry_gate(failed.as_ref(), session_id)?;
    let failed = failed.expect("the gate refuses a missing turn");
    let mut options = failed.options.clone();
    if let Some(provider_options) = options.provider_options.as_mut() {
        strip_account_option(provider_options);
    }
    let lineage = crate::functions::send::TurnLineage::continuing(&failed);
    let started = crate::functions::send::seed_new(
        deps,
        &cfg,
        session_id,
        options,
        Some(&failed),
        failed.message_preview.clone(),
        &lineage,
    )
    .await?;
    if failed.result.is_some() {
        let reason = failed.result_error.as_deref().unwrap_or("the turn failed");
        crate::turn_loop::append_resume_nudge(&session, session_id, &started.turn_id, 1, reason)
            .await?;
    }
    Ok(RetryResponse {
        turn_id: started.turn_id,
    })
}

// ── the usage ledger (D133) ──

/// One document per account, project and UTC day (`<day>:<account>:<project>`).
const LEDGER_SCOPE: &str = "harness_account_usage";
/// Days a ledger document is kept.
const LEDGER_DAYS: i64 = 35;
const NO_PROJECT_KEY: &str = "none";

/// Tokens of one model.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    #[serde(default)]
    pub input: u64,
    #[serde(default)]
    pub output: u64,
    #[serde(default)]
    pub cache_read: u64,
    #[serde(default)]
    pub cache_write: u64,
}

impl Tokens {
    fn add(&mut self, other: Tokens) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.cache_read = self.cache_read.saturating_add(other.cache_read);
        self.cache_write = self.cache_write.saturating_add(other.cache_write);
    }

    fn is_empty(&self) -> bool {
        *self == Tokens::default()
    }
}

/// A day of one account's Chat tokens in one project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct LedgerDay {
    account: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    /// Days since the unix epoch (UTC).
    day: i64,
    /// Hour of the day (UTC, 0-23) → model → tokens. Hours, so the week
    /// that starts at its reset, mid-day, is cut close to it.
    #[serde(default)]
    hours: BTreeMap<u8, BTreeMap<String, Tokens>>,
}

impl LedgerDay {
    fn key(&self) -> String {
        ledger_key(self.day, &self.account, self.project_id.as_deref())
    }
}

fn ledger_key(day: i64, account: &str, project_id: Option<&str>) -> String {
    format!("{day}:{account}:{}", project_id.unwrap_or(NO_PROJECT_KEY))
}

/// The Anthropic id a model's tokens are priced under: no `claude-code/`
/// (or other) namespace, no `[1m]` context suffix, no date.
pub fn anthropic_model(model: &str) -> String {
    let bare = model.rsplit('/').next().unwrap_or(model);
    let bare = bare.split('[').next().unwrap_or(bare).trim();
    base_id(bare).to_string()
}

/// A model id without its `-YYYYMMDD` release date.
fn base_id(id: &str) -> &str {
    if id.len() > 9 {
        let (head, tail) = id.split_at(id.len() - 9);
        if tail.starts_with('-') && tail[1..].chars().all(|c| c.is_ascii_digit()) {
            return head;
        }
    }
    id
}

fn usage_tokens(message: &AssistantMessage) -> Option<Tokens> {
    let usage = message.usage.as_ref()?;
    let tokens = Tokens {
        input: usage.input.unwrap_or(0),
        output: usage.output.unwrap_or(0),
        cache_read: usage.cache_read.unwrap_or(0),
        cache_write: usage.cache_write.unwrap_or(0),
    };
    (!tokens.is_empty()).then_some(tokens)
}

fn add_to_ledger(doc: &mut LedgerDay, at_ms: i64, model: &str, tokens: Tokens) {
    let hour = (at_ms.rem_euclid(DAY_MS) / HOUR_MS) as u8;
    doc.hours
        .entry(hour)
        .or_default()
        .entry(anthropic_model(model))
        .or_default()
        .add(tokens);
}

/// The last day the ledger pruned in this process.
static PRUNED_DAY: AtomicI64 = AtomicI64::new(i64::MIN);

/// Count a generation's tokens under the turn's account and project. Best
/// effort: accounting never fails a turn.
pub async fn record_usage(deps: &Deps, record: &TurnRecord, message: &AssistantMessage) {
    let Some(account) = record.account.as_ref() else {
        return;
    };
    let Some(tokens) = usage_tokens(message) else {
        return;
    };
    let model = if message.model.is_empty() {
        record.options.model.as_str()
    } else {
        message.model.as_str()
    };
    let now = AgentMessage::now_ms();
    let day = now.div_euclid(DAY_MS);
    let key = ledger_key(day, &account.id, account.project_id.as_deref());
    let timeout = deps.cfg().await.session_timeout_ms;
    let written = async {
        let _guard = deps.locks.guard(&format!("account_usage:{key}")).await;
        let stored = crate::state::state_get(&deps.iii, LEDGER_SCOPE, &key, timeout).await?;
        let mut doc = serde_json::from_value::<LedgerDay>(stored).unwrap_or(LedgerDay {
            account: account.id.clone(),
            project_id: account.project_id.clone(),
            day,
            hours: BTreeMap::new(),
        });
        add_to_ledger(&mut doc, now, model, tokens);
        let value = serde_json::to_value(&doc)
            .map_err(|e| HarnessError::State(format!("account usage serialize: {e}")))?;
        crate::state::state_set(&deps.iii, LEDGER_SCOPE, &key, value, timeout).await
    }
    .await;
    if let Err(error) = written {
        tracing::warn!(session_id = %record.session_id, %error, "account usage not recorded");
    }
    if PRUNED_DAY.swap(day, Ordering::Relaxed) != day {
        let _ = load_ledger(deps, now).await;
    }
}

/// The ledger's documents, after deleting those past [`LEDGER_DAYS`].
async fn load_ledger(deps: &Deps, now: i64) -> Result<Vec<LedgerDay>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let docs: Vec<LedgerDay> = crate::state::list_values(&deps.iii, LEDGER_SCOPE, timeout).await?;
    let oldest = now.div_euclid(DAY_MS) - LEDGER_DAYS;
    let (kept, stale): (Vec<_>, Vec<_>) = docs.into_iter().partition(|doc| doc.day >= oldest);
    for doc in stale {
        if let Err(error) =
            crate::state::state_delete(&deps.iii, LEDGER_SCOPE, &doc.key(), timeout).await
        {
            tracing::warn!(%error, key = %doc.key(), "stale account usage not deleted");
        }
    }
    Ok(kept)
}

/// One account's Chat tokens since `since_ms`, by project (`None`: no
/// project) and model. An hour that overlaps the start counts whole.
fn chat_tokens(
    ledger: &[LedgerDay],
    account: &str,
    since_ms: i64,
) -> BTreeMap<Option<String>, BTreeMap<String, Tokens>> {
    let mut out: BTreeMap<Option<String>, BTreeMap<String, Tokens>> = BTreeMap::new();
    for doc in ledger.iter().filter(|doc| doc.account == account) {
        for (hour, models) in &doc.hours {
            let end = doc.day * DAY_MS + (i64::from(*hour) + 1) * HOUR_MS;
            if end <= since_ms {
                continue;
            }
            let project = out.entry(doc.project_id.clone()).or_default();
            for (model, tokens) in models {
                project.entry(model.clone()).or_default().add(*tokens);
            }
        }
    }
    out
}

// ── prices ──

/// USD per million tokens.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Price {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_write: f64,
}

impl Price {
    fn new(input: f64, output: f64, cache_read: f64) -> Self {
        Self {
            input,
            output,
            cache_read,
            // The 5-minute cache write: 1.25x input.
            cache_write: input * 1.25,
        }
    }

    fn from_catalog(pricing: &crate::types::model::Pricing) -> Option<Self> {
        let input = pricing.input?;
        Some(Self {
            input,
            output: pricing.output.unwrap_or(input * 5.0),
            cache_read: pricing.cache_read.unwrap_or(input * 0.1),
            cache_write: pricing.cache_write.unwrap_or(input * 1.25),
        })
    }

    fn cost(&self, tokens: &Tokens) -> f64 {
        (tokens.input as f64 * self.input
            + tokens.output as f64 * self.output
            + tokens.cache_read as f64 * self.cache_read
            + tokens.cache_write as f64 * self.cache_write)
            / 1_000_000.0
    }
}

/// The Anthropic API list prices (`provider-anthropic`'s `pricing_for`), for
/// when the router lists no `anthropic/<model>` (no API key configured).
fn list_price(model: &str) -> Option<Price> {
    Some(match model {
        "claude-opus-5-5" => Price::new(4.0, 20.0, 0.20),
        "claude-opus-5" | "claude-opus-4-8" | "claude-opus-4-7" | "claude-opus-4-6"
        | "claude-opus-4-5" => Price::new(5.0, 25.0, 0.50),
        "claude-opus-4-1" | "claude-opus-4" => Price::new(15.0, 75.0, 1.50),
        "claude-sonnet-5" | "claude-sonnet-5-5" => Price::new(2.0, 10.0, 0.20),
        "claude-sonnet-4-6" | "claude-sonnet-4-5" | "claude-sonnet-4" => {
            Price::new(3.0, 15.0, 0.30)
        }
        "claude-haiku-4-5" => Price::new(1.0, 5.0, 0.10),
        "claude-haiku-5-5" => Price::new(0.10, 0.50, 0.01),
        "claude-fable-5-1" | "claude-mythos-5-1" => Price::new(10.0, 50.0, 0.25),
        "claude-fable-5" | "claude-mythos-5" => Price::new(10.0, 50.0, 1.0),
        _ => return None,
    })
}

/// Every model the built-in list prices.
const LISTED_MODELS: [&str; 18] = [
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-opus-4-5",
    "claude-opus-4-1",
    "claude-opus-4",
    "claude-sonnet-5-5",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
    "claude-sonnet-4-5",
    "claude-sonnet-4",
    "claude-haiku-5-5",
    "claude-haiku-4-5",
    "claude-fable-5-1",
    "claude-fable-5",
    "claude-mythos-5",
];

/// `claude-opus-4-8` → (`opus`, 4.08).
fn family_and_version(model: &str) -> Option<(&str, f64)> {
    let rest = model.strip_prefix("claude-")?;
    let mut parts = rest.split('-');
    let family = parts.next().filter(|family| !family.is_empty())?;
    let numbers: Vec<u32> = parts.map_while(|part| part.parse().ok()).collect();
    let major = f64::from(*numbers.first()?);
    let minor = numbers.get(1).copied().map_or(0.0, f64::from);
    Some((family, major + minor / 100.0))
}

/// The price of the model's nearest relative: the same family, the closest
/// version (the newer on a tie).
fn nearest_price(model: &str, priced: &BTreeMap<String, Price>) -> Option<Price> {
    let (family, version) = family_and_version(model)?;
    priced
        .iter()
        .filter_map(|(candidate, price)| {
            let (other_family, other_version) = family_and_version(candidate)?;
            (other_family == family).then_some((other_version, *price))
        })
        .min_by(|(a, _), (b, _)| {
            let (da, db) = ((a - version).abs(), (b - version).abs());
            da.total_cmp(&db).then(b.total_cmp(a))
        })
        .map(|(_, price)| price)
}

/// The list price of each model: the router's `anthropic/<model>` entry,
/// else the built-in list, else the nearest model of its family. `None`
/// for a model with no family to go by.
async fn price_book(deps: &Deps, models: &BTreeSet<String>) -> BTreeMap<String, Option<Price>> {
    let lookups = models.iter().map(|model| async move {
        let catalog = call_optional(
            deps,
            ROUTER_MODEL_GET_ID,
            json!({ "id": model, "provider": PRICE_PROVIDER }),
            CATALOG_TIMEOUT_MS,
        )
        .await
        .ok()
        .flatten()
        .and_then(|value| value.get("model").cloned())
        .and_then(|model| serde_json::from_value::<crate::types::model::Model>(model).ok())
        .and_then(|model| model.pricing)
        .and_then(|pricing| Price::from_catalog(&pricing));
        (model.clone(), catalog)
    });
    let catalog: BTreeMap<String, Option<Price>> = futures_util::future::join_all(lookups)
        .await
        .into_iter()
        .collect();
    price_models(models, &catalog)
}

fn price_models(
    models: &BTreeSet<String>,
    catalog: &BTreeMap<String, Option<Price>>,
) -> BTreeMap<String, Option<Price>> {
    let mut priced: BTreeMap<String, Price> = LISTED_MODELS
        .iter()
        .filter_map(|model| Some((model.to_string(), list_price(model)?)))
        .collect();
    for (model, price) in catalog {
        if let Some(price) = price {
            priced.insert(model.clone(), *price);
        }
    }
    models
        .iter()
        .map(|model| {
            let price = priced
                .get(model)
                .copied()
                .or_else(|| nearest_price(model, &priced));
            (model.clone(), price)
        })
        .collect()
}

fn cost_of(models: &BTreeMap<String, Tokens>, prices: &BTreeMap<String, Option<Price>>) -> f64 {
    models
        .iter()
        .filter_map(|(model, tokens)| Some(prices.get(model).copied().flatten()?.cost(tokens)))
        .sum()
}

// ── harness::metrics::account-split ──

pub const ACCOUNT_SPLIT_ID: &str = "harness::metrics::account-split";
pub const ACCOUNT_SPLIT_DESC: &str = "Each tmux Claude Code account's weekly usage split by \
     project (chat-projects D28, D133): the Chat's tokens from the harness's ledger and the tokens \
     tmux measured outside the Chat, both priced at the Anthropic API list price, share the \
     account's weekly percentage. `[]` with no tmux worker.";

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct AccountSplitRequest {}

/// One account's week.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AccountSplit {
    /// The tmux account id.
    pub account: String,
    /// How much of the week is used, 0 to 100 (tmux's reading).
    pub week_percent: f64,
    /// When the week starts again, in unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at_ms: Option<u64>,
    /// tmux measured the usage outside the Chat. When it could not, there is
    /// no `outside` segment and the Chat's segments split the week alone.
    pub outside_measured: bool,
    /// They add up to `week_percent`; none when nothing was spent.
    pub segments: Vec<SplitSegment>,
}

/// A share of the week.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SplitSegment {
    Project {
        project_id: String,
        name: String,
        color: String,
        percent: f64,
    },
    NoProject {
        percent: f64,
    },
    Outside {
        percent: f64,
    },
}

/// One account's week: its start, reset and percentage.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Week {
    start_ms: i64,
    resets_at_ms: Option<u64>,
    percent: f64,
}

/// The week starts at `resets_at_ms − 7 days`. A window that reset after
/// tmux read it has started again: nothing used yet.
fn week_of(limit: Option<&TmuxLimit>, now: i64) -> Week {
    match limit {
        Some(limit) if limit.reset_since_read => Week {
            start_ms: limit.resets_at_ms.map_or(now - WEEK_MS, |at| at as i64),
            resets_at_ms: limit.resets_at_ms.map(|at| at + WEEK_MS as u64),
            percent: 0.0,
        },
        Some(limit) => Week {
            start_ms: limit
                .resets_at_ms
                .map_or(now - WEEK_MS, |at| at as i64 - WEEK_MS),
            resets_at_ms: limit.resets_at_ms,
            percent: limit.used_percent.clamp(0.0, 100.0),
        },
        None => Week {
            start_ms: now - WEEK_MS,
            resets_at_ms: None,
            percent: 0.0,
        },
    }
}

/// `percent = week_percent × segment cost ÷ (Chat cost + outside cost)`:
/// projects by cost (then name), then no project, then the outside. A week
/// spent only outside is all outside; nothing spent is no segment.
fn segments(
    week_percent: f64,
    projects: Vec<(Project, f64)>,
    no_project: f64,
    outside: Option<f64>,
) -> Vec<SplitSegment> {
    let total: f64 =
        projects.iter().map(|(_, cost)| cost).sum::<f64>() + no_project + outside.unwrap_or(0.0);
    if total <= 0.0 {
        return match outside {
            Some(_) if week_percent > 0.0 => vec![SplitSegment::Outside {
                percent: week_percent,
            }],
            _ => Vec::new(),
        };
    }
    let share = |cost: f64| week_percent * cost / total;
    let mut projects = projects;
    projects.sort_by(|(a, cost_a), (b, cost_b)| cost_b.total_cmp(cost_a).then(a.name.cmp(&b.name)));
    let mut out: Vec<SplitSegment> = projects
        .into_iter()
        .filter(|(_, cost)| *cost > 0.0)
        .map(|(project, cost)| SplitSegment::Project {
            project_id: project.id,
            name: project.name,
            color: project.color,
            percent: share(cost),
        })
        .collect();
    if no_project > 0.0 {
        out.push(SplitSegment::NoProject {
            percent: share(no_project),
        });
    }
    if let Some(outside) = outside.filter(|cost| *cost > 0.0) {
        out.push(SplitSegment::Outside {
            percent: share(outside),
        });
    }
    out
}

#[derive(Debug, Clone, Default, Deserialize)]
struct OutsideReading {
    #[serde(default)]
    by_model: Vec<OutsideModel>,
}

#[derive(Debug, Clone, Deserialize)]
struct OutsideModel {
    model: String,
    #[serde(default)]
    input: u64,
    #[serde(default)]
    output: u64,
    #[serde(default)]
    cache_read: u64,
    #[serde(default)]
    cache_write: u64,
}

fn outside_tokens(reading: OutsideReading) -> BTreeMap<String, Tokens> {
    let mut out: BTreeMap<String, Tokens> = BTreeMap::new();
    for row in reading.by_model {
        out.entry(anthropic_model(&row.model))
            .or_default()
            .add(Tokens {
                input: row.input,
                output: row.output,
                cache_read: row.cache_read,
                cache_write: row.cache_write,
            });
    }
    out
}

/// tmux's measure of the account's tokens outside the Chat since the week
/// started; `None` when it could not say.
async fn outside_since(
    deps: &Deps,
    account: &str,
    since_ms: i64,
) -> Option<BTreeMap<String, Tokens>> {
    let payload = json!({ "account": account, "since_ms": since_ms.max(0) });
    match call_optional(deps, OUTSIDE_ID, payload, OUTSIDE_TIMEOUT_MS).await {
        Ok(Some(value)) => match serde_json::from_value::<OutsideReading>(value) {
            Ok(reading) => Some(outside_tokens(reading)),
            Err(error) => {
                tracing::warn!(account, %error, "tmux::usage::claude_outside answered an unknown shape");
                None
            }
        },
        Ok(None) => None,
        Err(error) => {
            tracing::warn!(account, %error, "tmux::usage::claude_outside failed");
            None
        }
    }
}

/// `harness::metrics::account-split`.
pub async fn account_split(
    deps: &Deps,
    _req: AccountSplitRequest,
) -> Result<Vec<AccountSplit>, HarnessError> {
    let usage = match call_optional(deps, USAGE_READ_ID, json!({}), TMUX_TIMEOUT_MS).await {
        Ok(Some(value)) => value,
        Ok(None) => return Ok(Vec::new()),
        Err(error) => {
            return Err(HarnessError::rejected(
                "tmux_unavailable",
                "The accounts' usage could not be read from tmux. Try again in a moment.",
                json!({ "detail": error }),
            ));
        }
    };
    let claude: TmuxClaude =
        serde_json::from_value(usage.get("claude").cloned().unwrap_or_default())
            .map_err(|e| HarnessError::Dependency(format!("{USAGE_READ_ID}: {e}")))?;
    let now = AgentMessage::now_ms();
    let ledger = load_ledger(deps, now).await?;
    let path = deps.cfg().await.projects_file_path.clone();
    let projects = deps
        .projects
        .list(&path)
        .await
        .map(|document| document.projects)
        .unwrap_or_else(|error| {
            tracing::warn!(%error, "projects unreadable; the split shows no project");
            Vec::new()
        });

    let mut weeks = Vec::new();
    for account in &claude.accounts {
        let week = week_of(
            account.usage.as_ref().and_then(|usage| usage.week.as_ref()),
            now,
        );
        let chat = chat_tokens(&ledger, &account.id, week.start_ms);
        let outside = outside_since(deps, &account.id, week.start_ms).await;
        weeks.push((account.id.clone(), week, chat, outside));
    }
    let models: BTreeSet<String> = weeks
        .iter()
        .flat_map(|(_, _, chat, outside)| {
            chat.values()
                .flat_map(|models| models.keys())
                .chain(outside.iter().flat_map(|models| models.keys()))
                .cloned()
                .collect::<Vec<_>>()
        })
        .collect();
    let prices = price_book(deps, &models).await;

    Ok(weeks
        .into_iter()
        .map(|(account, week, chat, outside)| {
            let mut by_project: BTreeMap<String, (Project, f64)> = BTreeMap::new();
            let mut no_project = 0.0;
            for (project_id, models) in &chat {
                let cost = cost_of(models, &prices);
                match project_id
                    .as_deref()
                    .and_then(|id| projects.iter().find(|project| project.id == id))
                {
                    Some(project) => {
                        by_project
                            .entry(project.id.clone())
                            .or_insert_with(|| (project.clone(), 0.0))
                            .1 += cost;
                    }
                    // A project deleted since counts as no project.
                    None => no_project += cost,
                }
            }
            let outside_cost = outside.as_ref().map(|models| cost_of(models, &prices));
            AccountSplit {
                account,
                week_percent: week.percent,
                resets_at_ms: week.resets_at_ms,
                outside_measured: outside.is_some(),
                segments: segments(
                    week.percent,
                    by_project.into_values().collect(),
                    no_project,
                    outside_cost,
                ),
            }
        })
        .collect())
}

#[cfg(test)]
#[path = "accounts/tests.rs"]
mod tests;
