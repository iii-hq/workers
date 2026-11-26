//! Autocompact (chat-projects phase 3, D158-D167): a conversation compacts by
//! itself when its context passes a number of tokens set globally, per
//! project or per conversation, or at the model's own limit when that comes
//! first (D4).
//!
//! - **Settings.** The global one is the harness configuration's
//!   `autocompact {mode: "off" | "tokens", tokens?, instructions?}`
//!   (`harness::chat-settings`); a project's is its record's `autocompact`, and
//!   a conversation's is `metadata.autocompact`, both
//!   `{mode: "inherit" | "tokens" | "off", tokens?}`. A malformed project or
//!   conversation value reads as `inherit`.
//! - **Resolution** ([`resolve`]): the conversation, then its project, then
//!   the global; the first that is not `inherit` decides. A side chat reads
//!   its conversation's `metadata.autocompact`; a sub-agent has none of its
//!   own (spawns copy only `project_id`), so it follows its project and the
//!   global. Resolved at a turn's first step and kept on the record
//!   ([`TurnAutocompact`]), and resolved again when the turn ends.
//! - **Two thresholds** (D159). At the end of a turn the conversation
//!   compacts once `total + the last output ≥ effective`, where `effective`
//!   is the setting when it is below the model's `usable`
//!   ([`at_turn_end`]). Inside a turn, `context::assemble` gets a soft
//!   `compaction_budget` of `setting + max(10 %, 20k)` ([`cap_tokens`]), so a
//!   turn that grows fast still compacts before the model's limit.
//! - **Back-off** (D166), per session in `harness_autocompact/<session_id>`:
//!   after a failure, or a compaction that gained less than 10 %, the next
//!   try waits until the context grew by another 10 % of `effective`. A usage
//!   limit starts none: the next turn tries again.
//! - **The log** (`harness::autocompact::log`): the last automatic
//!   compactions, one CAS-updated ring of [`LOG_CAPACITY`] in
//!   `harness_autocompact_log`.
//! - **The models index** (`harness::autocompact::models`): the models that
//!   ran a turn on this engine, with the `usable` context-manager measured,
//!   one key per model in `harness_autocompact_models`, written with the
//!   context snapshot.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::clients::SessionClient;
use crate::config::{AutocompactConfig, AutocompactMode};
use crate::context_snapshot::ContextSnapshotV1;
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::projects::Project;
use crate::types::message::AgentMessage;
use crate::types::turn::TurnRecord;

/// The session metadata key of a conversation's own setting.
pub const SESSION_KEY: &str = "autocompact";
pub const MIN_TOKENS: u64 = 20_000;
pub const MAX_TOKENS: u64 = 2_000_000;
pub const MAX_INSTRUCTIONS_CHARS: usize = 1_000;
/// The inline cap's least headroom over the setting (D159).
const HEADROOM_FLOOR_TOKENS: u64 = 20_000;

pub const BACKOFF_SCOPE: &str = "harness_autocompact";
pub const LOG_SCOPE: &str = "harness_autocompact_log";
const LOG_KEY: &str = "ring";
pub const LOG_CAPACITY: usize = 50;
const LOG_DEFAULT_LIMIT: usize = 20;
const LOG_CAS_ATTEMPTS: usize = 8;
pub const MODELS_SCOPE: &str = "harness_autocompact_models";
const MODELS_WINDOW_MS: i64 = 7 * 24 * 3_600_000;
/// A model's index entry is rewritten at most this often when its `usable`
/// did not change (one state write per step otherwise).
const MODELS_REFRESH_MS: i64 = 10 * 60_000;

pub const MODELS_ID: &str = "harness::autocompact::models";
pub const MODELS_DESC: &str = "Internal control-plane: the models that ran a turn on this engine in the \
     last 7 days, for the autocompact settings (chat-projects phase 3): \
     [{ provider, model, name, context_window, usable, last_used_ms }]. `usable` is the input budget \
     context-manager measured for the model; names and windows come from the router catalog.";
pub const LOG_ID: &str = "harness::autocompact::log";
pub const LOG_DESC: &str = "Internal control-plane: the engine's last automatic compactions, newest first \
     (chat-projects phase 3): { limit?: 20 } → [{ session_id, title, at_ms, trigger, threshold_tokens, \
     context_before, context_after, rule }].";

// ── settings and validation ──

/// One level of the setting chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    Inherit,
    Off,
    Tokens(u64),
}

/// A project's or a conversation's value, leniently: absent or malformed
/// (an unknown mode, `tokens` missing or out of range) is `inherit`.
pub fn level_of(value: Option<&Value>) -> Level {
    let Some(value) = value else {
        return Level::Inherit;
    };
    match value.get("mode").and_then(Value::as_str) {
        Some("off") => Level::Off,
        Some("tokens") => value
            .get("tokens")
            .and_then(Value::as_u64)
            .filter(|tokens| (MIN_TOKENS..=MAX_TOKENS).contains(tokens))
            .map_or(Level::Inherit, Level::Tokens),
        _ => Level::Inherit,
    }
}

/// The global level. A `tokens` mode with no valid `tokens` (a configuration
/// written by hand) is off, the default.
pub fn global_level(config: &AutocompactConfig) -> Level {
    match (config.mode, config.tokens) {
        (AutocompactMode::Tokens, Some(tokens)) if (MIN_TOKENS..=MAX_TOKENS).contains(&tokens) => {
            Level::Tokens(tokens)
        }
        (AutocompactMode::Tokens, tokens) => {
            tracing::warn!(
                ?tokens,
                "autocompact is on without a valid `tokens` (20,000-2,000,000); treating it as off"
            );
            Level::Off
        }
        (AutocompactMode::Off, _) => Level::Off,
    }
}

/// The global `instructions`, when set and not blank.
pub fn global_instructions(config: &AutocompactConfig) -> Option<String> {
    config
        .instructions
        .as_deref()
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_string)
}

const BAD_SETTING_OVERRIDE: &str =
    "Choose the global setting, a size in tokens, or the model's limit only.";
const BAD_SETTING_GLOBAL: &str = "Choose a size in tokens, or off.";
const BAD_TOKENS: &str = "Between 20k and 2M tokens";
const BAD_INSTRUCTIONS: &str = "Up to 1,000 characters";

fn bad_setting(message: &str, value: &Value) -> HarnessError {
    HarnessError::rejected(
        "bad_setting",
        message,
        json!({ "field": "autocompact", "autocompact": value }),
    )
}

fn bad_tokens(value: &Value) -> HarnessError {
    HarnessError::rejected(
        "bad_tokens",
        BAD_TOKENS,
        json!({ "field": "autocompact.tokens", "tokens": value,
            "min": MIN_TOKENS, "max": MAX_TOKENS }),
    )
}

fn bad_instructions(chars: usize) -> HarnessError {
    HarnessError::rejected(
        "bad_instructions",
        BAD_INSTRUCTIONS,
        json!({ "field": "autocompact.instructions", "chars": chars,
            "max": MAX_INSTRUCTIONS_CHARS }),
    )
}

/// `tokens` checked: present, an integer, in range.
fn checked_tokens(value: &Value, setting: &Value, message: &str) -> Result<u64, HarnessError> {
    match value.get("tokens") {
        None | Some(Value::Null) => Err(bad_setting(message, setting)),
        Some(tokens) => tokens
            .as_u64()
            .filter(|tokens| (MIN_TOKENS..=MAX_TOKENS).contains(tokens))
            .ok_or_else(|| bad_tokens(tokens)),
    }
}

/// A project's (or a conversation's) setting, as `harness::projects::create`
/// and `update` store it: `{mode: "inherit"}`, `{mode: "off"}` or
/// `{mode: "tokens", tokens}`. Refused with `bad_setting` (a mode other than
/// these, or `tokens` without a number) or `bad_tokens` (outside
/// 20,000-2,000,000).
pub fn validate_override(value: &Value) -> Result<Value, HarnessError> {
    match value.get("mode").and_then(Value::as_str) {
        Some("inherit") => Ok(json!({ "mode": "inherit" })),
        Some("off") => Ok(json!({ "mode": "off" })),
        Some("tokens") => {
            let tokens = checked_tokens(value, value, BAD_SETTING_OVERRIDE)?;
            Ok(json!({ "mode": "tokens", "tokens": tokens }))
        }
        _ => Err(bad_setting(BAD_SETTING_OVERRIDE, value)),
    }
}

/// The global setting after a `chat-settings::set` patch was merged into it:
/// `mode` `off` or `tokens` (`tokens` required with it), `tokens` in range
/// whenever it is kept, `instructions` up to 1,000 characters.
pub fn validate_global(value: &Value) -> Result<AutocompactConfig, HarnessError> {
    let Some(object) = value.as_object() else {
        return Err(bad_setting(BAD_SETTING_GLOBAL, value));
    };
    if let Some(unknown) = object
        .keys()
        .find(|key| !matches!(key.as_str(), "mode" | "tokens" | "instructions"))
    {
        return Err(HarnessError::rejected(
            "bad_setting",
            format!("Unknown auto-compact setting `{unknown}`."),
            json!({ "field": format!("autocompact.{unknown}") }),
        ));
    }
    let mode = match object.get("mode").and_then(Value::as_str) {
        None | Some("off") => AutocompactMode::Off,
        Some("tokens") => AutocompactMode::Tokens,
        Some(_) => return Err(bad_setting(BAD_SETTING_GLOBAL, value)),
    };
    let tokens = match object.get("tokens") {
        None | Some(Value::Null) if mode == AutocompactMode::Tokens => {
            return Err(bad_setting(BAD_SETTING_GLOBAL, value));
        }
        None | Some(Value::Null) => None,
        Some(_) => Some(checked_tokens(value, value, BAD_SETTING_GLOBAL)?),
    };
    let instructions = match object.get("instructions") {
        None | Some(Value::Null) => None,
        Some(Value::String(text)) => {
            let chars = text.chars().count();
            if chars > MAX_INSTRUCTIONS_CHARS {
                return Err(bad_instructions(chars));
            }
            Some(text.trim().to_string()).filter(|text| !text.is_empty())
        }
        Some(_) => {
            return Err(HarnessError::rejected(
                "bad_instructions",
                "Write what the summary keeps as text.",
                json!({ "field": "autocompact.instructions" }),
            ));
        }
    };
    Ok(AutocompactConfig {
        mode,
        tokens,
        instructions,
    })
}

// ── resolution ──

/// Which level's setting applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleScope {
    Global,
    Project,
    /// The conversation's own (a side chat's: its conversation's).
    Session,
}

/// The rule that decided a conversation's setting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Rule {
    pub scope: RuleScope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_id: Option<String>,
    /// The project's name when the rule was read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_name: Option<String>,
}

impl Rule {
    pub fn global() -> Self {
        Self {
            scope: RuleScope::Global,
            project_id: None,
            project_name: None,
        }
    }

    pub fn session() -> Self {
        Self {
            scope: RuleScope::Session,
            project_id: None,
            project_name: None,
        }
    }

    /// The rule as the clients name it: "global rule", "acme-web rule",
    /// "this conversation's rule".
    pub fn name(&self) -> String {
        match self.scope {
            RuleScope::Global => "global rule".into(),
            RuleScope::Session => "this conversation's rule".into(),
            RuleScope::Project => format!(
                "{} rule",
                self.project_name
                    .as_deref()
                    .or(self.project_id.as_deref())
                    .unwrap_or("project")
            ),
        }
    }
}

/// What the chain resolved to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolution {
    /// The resolved `tokens`; `None` for off (also the global default).
    pub setting_tokens: Option<u64>,
    /// The level that decided, off included.
    pub rule: Rule,
}

fn project_id_of(metadata: &Map<String, Value>) -> Option<&str> {
    metadata
        .get(crate::projects::METADATA_KEY)
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty() && *id != crate::projects::NONE)
}

/// The chain over one conversation's metadata (pure). `projects` is read
/// only for a conversation that inherits and names a project; a project that
/// is gone is skipped.
pub fn resolve_with(
    conversation: &Map<String, Value>,
    projects: &[Project],
    global: &AutocompactConfig,
) -> Resolution {
    let decided = |level: Level, rule: Rule| match level {
        Level::Inherit => None,
        Level::Off => Some(Resolution {
            setting_tokens: None,
            rule,
        }),
        Level::Tokens(tokens) => Some(Resolution {
            setting_tokens: Some(tokens),
            rule,
        }),
    };
    if let Some(resolution) = decided(level_of(conversation.get(SESSION_KEY)), Rule::session()) {
        return resolution;
    }
    if let Some(project) =
        project_id_of(conversation).and_then(|id| projects.iter().find(|project| project.id == id))
        && let Some(resolution) = decided(
            level_of(Some(&project.autocompact)),
            Rule {
                scope: RuleScope::Project,
                project_id: Some(project.id.clone()),
                project_name: Some(project.name.clone()),
            },
        )
    {
        return resolution;
    }
    decided(global_level(global), Rule::global()).unwrap_or(Resolution {
        setting_tokens: None,
        rule: Rule::global(),
    })
}

/// The metadata whose setting a turn of `record`'s session follows: its
/// own, or a side chat's conversation's (its own copy goes stale). An
/// unreadable one reads as none (every level inherits).
async fn conversation_metadata(session: &SessionClient, record: &TurnRecord) -> Map<String, Value> {
    let read = |session_id: String| async move {
        session
            .metadata_of(&session_id)
            .await
            .map(Option::unwrap_or_default)
            .unwrap_or_else(|error| {
                tracing::warn!(session_id, %error, "autocompact: session metadata unreadable; it inherits");
                Map::new()
            })
    };
    let own = read(record.session_id.clone()).await;
    if record.parent.is_some() {
        return own;
    }
    match crate::side_chat::side_chat_parent(&own) {
        Some(parent) => {
            let mut parent_metadata = read(parent.to_string()).await;
            // A conversation that went away leaves the side chat its own copy.
            if parent_metadata.is_empty() {
                parent_metadata = own;
            }
            parent_metadata
        }
        None => own,
    }
}

/// Resolve the setting a turn of `record` follows now.
pub async fn resolve(deps: &Deps, session: &SessionClient, record: &TurnRecord) -> Resolution {
    let metadata = conversation_metadata(session, record).await;
    let cfg = deps.cfg().await;
    let projects = if level_of(metadata.get(SESSION_KEY)) == Level::Inherit
        && project_id_of(&metadata).is_some()
    {
        match deps.projects.list(&cfg.projects_file_path).await {
            Ok(document) => document.projects,
            Err(error) => {
                tracing::warn!(session_id = %record.session_id, %error, "autocompact: the projects could not be read; the project's setting is skipped");
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };
    resolve_with(&metadata, &projects, &cfg.autocompact)
}

/// What a turn keeps of its resolution: the setting, the rule, and the
/// session's back-off while it still holds (judged on the stored snapshot;
/// the turn's own snapshots judge it at each later step).
pub async fn for_turn(
    deps: &Deps,
    session: &SessionClient,
    record: &TurnRecord,
) -> TurnAutocompact {
    let resolution = resolve(deps, session, record).await;
    let Some(setting) = resolution.setting_tokens else {
        return TurnAutocompact::new(resolution, None);
    };
    let mut backoff = read_backoff(deps, &record.session_id).await;
    if let Some(held) = &backoff {
        let timeout = deps.cfg().await.session_timeout_ms;
        let last = match &record.context_snapshot {
            Some(snapshot) => Some(snapshot.clone()),
            None => crate::context_snapshot::get(&deps.iii, &record.session_id, timeout)
                .await
                .ok()
                .flatten(),
        };
        if let Some(last) = last {
            let effective = effective_tokens(Some(setting), last.usable).unwrap_or(setting);
            if held.allows(measure(&last), effective) {
                backoff = None;
            }
        }
    }
    TurnAutocompact::new(resolution, backoff)
}

// ── the numbers ──

/// The inline budget `context::assemble` is sent for a setting (D159):
/// `setting + max(10 % of setting, 20k)`.
pub fn cap_tokens(setting_tokens: u64) -> u64 {
    setting_tokens.saturating_add((setting_tokens / 10).max(HEADROOM_FLOOR_TOKENS))
}

/// The threshold that applies: the setting when it is below the model's
/// `usable`, else `None` (the model's own limit, as before autocompact).
pub fn effective_tokens(setting_tokens: Option<u64>, usable: u64) -> Option<u64> {
    setting_tokens.filter(|setting| *setting < usable)
}

/// What the next request carries (D159): the billed input of the last step
/// (`input + cache_read + cache_write`) plus that step's output. The
/// snapshot's `total` is that billed input only once `exactify` applied it,
/// which needs the parts counted by the router first: on a conversation's
/// first turn, and the first after a restart, it is still
/// context-manager's estimate, which runs about 20 tokens a message high.
/// So the step's own usage decides when it has one; the snapshot's `total`
/// is the fallback for a step the provider reported no input for.
pub fn measure(snapshot: &ContextSnapshotV1) -> u64 {
    let usage = snapshot.usage.as_ref();
    let billed = usage
        .map(|usage| {
            usage
                .input
                .unwrap_or(0)
                .saturating_add(usage.cache_read.unwrap_or(0))
                .saturating_add(usage.cache_write.unwrap_or(0))
        })
        .filter(|billed| *billed > 0)
        .unwrap_or(snapshot.total);
    billed.saturating_add(usage.and_then(|usage| usage.output).unwrap_or(0))
}

/// A compaction gained at least 10 % of the context.
pub fn gained_enough(before: u64, after: u64) -> bool {
    after.saturating_mul(10) <= before.saturating_mul(9)
}

/// `160000` → `160k`, `1200000` → `1.2M`: the token sizes the notices name.
pub fn short_tokens(tokens: u64) -> String {
    fn trimmed(value: f64) -> String {
        let text = format!("{value:.1}");
        text.strip_suffix(".0").unwrap_or(&text).to_string()
    }
    if tokens < 1_000 {
        tokens.to_string()
    } else if tokens < 1_000_000 {
        let k = tokens as f64 / 1_000.0;
        if k < 10.0 {
            format!("{}k", trimmed(k))
        } else {
            format!("{}k", k.round() as u64)
        }
    } else {
        format!("{}M", trimmed(tokens as f64 / 1_000_000.0))
    }
}

// ── the turn's state ──

/// What a turn's record keeps of its resolution (resolved at its first
/// step).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TurnAutocompact {
    pub setting_tokens: Option<u64>,
    pub rule: Rule,
    /// An inline attempt failed, was busy, or gained less than 10 %: the
    /// rest of the turn assembles without a `compaction_budget` (D160).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub paused: bool,
    /// The session's back-off when the turn started.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff: Option<Backoff>,
}

impl TurnAutocompact {
    pub fn new(resolution: Resolution, backoff: Option<Backoff>) -> Self {
        Self {
            setting_tokens: resolution.setting_tokens,
            rule: resolution.rule,
            paused: false,
            backoff,
        }
    }

    /// The `compaction_budget` this step sends, given the last snapshot the
    /// turn knows: none while paused, or while the back-off holds (a size it
    /// cannot judge holds too).
    pub fn budget(&self, last: Option<&ContextSnapshotV1>) -> Option<u64> {
        let setting = self.setting_tokens?;
        if self.paused {
            return None;
        }
        if let Some(backoff) = &self.backoff {
            let last = last?;
            let effective = effective_tokens(Some(setting), last.usable).unwrap_or(setting);
            if !backoff.allows(measure(last), effective) {
                return None;
            }
        }
        Some(cap_tokens(setting))
    }

    /// The snapshot's `autocompact` for a step that assembled against
    /// `usable`: the same numbers the clients compute between turns from
    /// the setting chain and the last `usable` (D164).
    pub fn snapshot(&self, usable: u64) -> SnapshotAutocompactV1 {
        SnapshotAutocompactV1 {
            setting_tokens: self.setting_tokens,
            effective_tokens: effective_tokens(self.setting_tokens, usable),
            cap_tokens: self.setting_tokens.map(cap_tokens),
            rule: Some(self.rule.clone()),
        }
    }
}

/// `ContextSnapshotV1.autocompact`: the threshold the meters mark.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SnapshotAutocompactV1 {
    /// The resolved setting; `null` for off.
    pub setting_tokens: Option<u64>,
    /// Where the conversation compacts at the end of a turn: the setting when
    /// it is below `usable`; `null` when the model's own limit applies.
    pub effective_tokens: Option<u64>,
    /// The budget inside a turn, `setting + max(10 %, 20k)`; `null` without
    /// a setting. context-manager compacts inline at `min(usable, cap)`.
    pub cap_tokens: Option<u64>,
    /// The level that decided; `null` when none was resolved.
    pub rule: Option<Rule>,
}

// ── back-off ──

/// `harness_autocompact/<session_id>` (D166).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Backoff {
    /// The context size of the attempt.
    pub at_tokens: u64,
    /// `failed`, `small_gain` or `empty`.
    pub reason: String,
    pub at_ms: i64,
}

impl Backoff {
    /// The next attempt waits for another 10 % of `effective`.
    pub fn allows(&self, measure: u64, effective: u64) -> bool {
        measure >= self.at_tokens.saturating_add(effective / 10)
    }
}

pub async fn read_backoff(deps: &Deps, session_id: &str) -> Option<Backoff> {
    let timeout = deps.cfg().await.session_timeout_ms;
    match crate::state::state_get(&deps.iii, BACKOFF_SCOPE, session_id, timeout).await {
        Ok(Value::Null) => None,
        Ok(value) => serde_json::from_value(value)
            .inspect_err(|error| tracing::warn!(session_id, %error, "autocompact back-off unreadable; ignored"))
            .ok(),
        Err(error) => {
            tracing::warn!(session_id, %error, "autocompact back-off unreadable; ignored");
            None
        }
    }
}

pub async fn write_backoff(deps: &Deps, session_id: &str, backoff: &Backoff) {
    let timeout = deps.cfg().await.session_timeout_ms;
    let value = serde_json::to_value(backoff).unwrap_or(Value::Null);
    if let Err(error) =
        crate::state::state_set(&deps.iii, BACKOFF_SCOPE, session_id, value, timeout).await
    {
        tracing::warn!(session_id, %error, "autocompact back-off not stored");
    }
}

pub async fn clear_backoff(deps: &Deps, session_id: &str) {
    let timeout = deps.cfg().await.session_timeout_ms;
    if let Err(error) =
        crate::state::state_delete(&deps.iii, BACKOFF_SCOPE, session_id, timeout).await
    {
        tracing::warn!(session_id, %error, "autocompact back-off not cleared");
    }
}

/// The back-off an automatic attempt leaves: `Some` to store, `None` to
/// clear, and nothing to do for a busy or stopped one.
pub fn backoff_after(
    status: crate::compaction::Status,
    usage_limit: bool,
    before: u64,
    after: Option<u64>,
    now_ms: i64,
) -> Option<Option<Backoff>> {
    use crate::compaction::Status;
    let backoff = |reason: &str| {
        Some(Some(Backoff {
            at_tokens: before,
            reason: reason.into(),
            at_ms: now_ms,
        }))
    };
    match status {
        Status::Ok => match after {
            Some(after) if !gained_enough(before, after) => backoff("small_gain"),
            _ => Some(None),
        },
        // A usage limit: the next turn tries again (D162).
        Status::Failed if usage_limit => None,
        Status::Failed => backoff("failed"),
        Status::Empty => backoff("empty"),
        Status::Busy | Status::Cancelled => None,
    }
}

// ── the end of a turn (D158) ──

/// Compact the conversation at the end of its turn when it passed its
/// threshold. Runs at the top of `finalize_completed`, the record still
/// `Running` (sends during it queue) and marked `compacting`; leaves the
/// refreshed snapshot on the record for `on-turn-completed`. A Stop abandons
/// the summary; the turn still completes. Never fails the turn.
pub async fn at_turn_end(deps: &Deps, session: &SessionClient, record: &mut TurnRecord) {
    // Sub-agents: their parent waits on this turn; the inline cap bounds them.
    if record.parent.is_some() || record.depth > 0 {
        return;
    }
    let Some(snapshot) = record.context_snapshot.clone() else {
        return;
    };
    // Resolved again: a change made during the turn applies at its end.
    let resolution = resolve(deps, session, record).await;
    let Some(effective) = effective_tokens(resolution.setting_tokens, snapshot.usable) else {
        return;
    };
    let before = measure(&snapshot);
    if before < effective || deps.compactions.is_running(&record.session_id) {
        return;
    }
    if let Some(backoff) = read_backoff(deps, &record.session_id).await
        && !backoff.allows(before, effective)
    {
        tracing::info!(session_id = %record.session_id, before, effective, at_tokens = backoff.at_tokens, "autocompact held by its back-off");
        return;
    }
    let cfg = deps.cfg().await;
    let compaction_id = crate::ids::new_compaction_id();
    record.compacting = Some(compaction_id.clone());
    record.updated_at = AgentMessage::now_ms();
    if let Err(error) = crate::state::put_turn(&deps.iii, record, cfg.session_timeout_ms).await {
        tracing::warn!(session_id = %record.session_id, %error, "autocompact skipped: the turn could not be marked compacting");
        record.compacting = None;
        return;
    }
    crate::session_status::write_phase(deps, record, crate::compaction::STATUS_REASON).await;
    let request = crate::compaction::Request {
        session_id: record.session_id.clone(),
        model: record.options.model.clone(),
        provider: record.options.provider.clone(),
        instructions: global_instructions(&cfg.autocompact),
        entry_id: None,
        // The turn owns the status line.
        mark_working: false,
        trigger: crate::compaction::Trigger::Auto,
        threshold_tokens: Some(effective),
        rule: Some(resolution.rule.clone()),
        provider_options: record
            .options
            .provider_options
            .as_ref()
            .and_then(|options| serde_json::to_value(options).ok()),
        account: record.account.as_ref().map(|account| account.id.clone()),
        context_before: Some(before),
        stop_turn: Some(record.turn_id.clone()),
    };
    let outcome = Box::pin(crate::compaction::run(deps, request, compaction_id)).await;
    record.compacting = None;
    if outcome.status == crate::compaction::Status::Ok {
        match crate::context_snapshot::get(&deps.iii, &record.session_id, cfg.session_timeout_ms)
            .await
        {
            Ok(stored) => {
                record.context_snapshot =
                    crate::context_snapshot::newest(record.context_snapshot.take(), stored);
            }
            Err(error) => {
                tracing::warn!(session_id = %record.session_id, %error, "the compacted snapshot could not be read back for on-turn-completed");
            }
        }
    }
    let usage_limit = outcome
        .failure
        .as_ref()
        .is_some_and(crate::clients::context::CompactFailure::is_usage_limit);
    match backoff_after(
        outcome.status,
        usage_limit,
        before,
        outcome.context_after,
        AgentMessage::now_ms(),
    ) {
        Some(Some(backoff)) => write_backoff(deps, &record.session_id, &backoff).await,
        Some(None) => clear_backoff(deps, &record.session_id).await,
        None => {}
    }
    if outcome.status == crate::compaction::Status::Ok {
        log_compaction(deps, session, &outcome).await;
    }
}

// ── the log ──

/// One automatic compaction, as `harness::autocompact::log` lists it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct LogEntry {
    pub session_id: String,
    /// The conversation's title when it compacted.
    pub title: String,
    pub at_ms: i64,
    /// `auto` or `inline`.
    pub trigger: crate::compaction::Trigger,
    pub threshold_tokens: Option<u64>,
    pub context_before: Option<u64>,
    pub context_after: Option<u64>,
    pub rule: Option<Rule>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct LogRing {
    /// Oldest first, at most [`LOG_CAPACITY`].
    #[serde(default)]
    entries: Vec<LogEntry>,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct LogRequest {
    /// How many, newest first (default 20, at most 50).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Rewrite the ring with `change` under compare-and-set (several harness
/// instances write it). An unreadable ring starts over.
async fn update_ring(deps: &Deps, change: impl Fn(&mut LogRing)) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    for _ in 0..LOG_CAS_ATTEMPTS {
        let current = crate::state::state_get(&deps.iii, LOG_SCOPE, LOG_KEY, timeout).await?;
        let mut ring: LogRing = match &current {
            Value::Null => LogRing::default(),
            value => serde_json::from_value(value.clone()).unwrap_or_else(|error| {
                tracing::warn!(%error, "the autocompact log was unreadable; it starts over");
                LogRing::default()
            }),
        };
        change(&mut ring);
        let overflow = ring.entries.len().saturating_sub(LOG_CAPACITY);
        ring.entries.drain(..overflow);
        let value = serde_json::to_value(&ring)
            .map_err(|error| HarnessError::State(format!("autocompact log: {error}")))?;
        let expected = (!current.is_null()).then_some(current);
        if crate::state::cas_value(&deps.iii, LOG_SCOPE, LOG_KEY, expected, value, timeout)
            .await?
            .is_none()
        {
            return Ok(());
        }
    }
    Err(HarnessError::State(
        "the autocompact log kept changing under this write".into(),
    ))
}

/// Add an automatic compaction (auto or inline) to the log, with the
/// conversation's title now; the others (`/compact`, the model's limit) are
/// not logged. Best effort.
pub async fn log_compaction(
    deps: &Deps,
    session: &SessionClient,
    outcome: &crate::compaction::Outcome,
) {
    use crate::compaction::Trigger;
    if !matches!(outcome.trigger, Trigger::Auto | Trigger::Inline) {
        return;
    }
    let title = match session.summary(&outcome.session_id).await {
        Ok(Some(summary)) => summary.title,
        _ => String::new(),
    };
    let entry = LogEntry {
        session_id: outcome.session_id.clone(),
        title,
        at_ms: AgentMessage::now_ms(),
        trigger: outcome.trigger,
        threshold_tokens: outcome.threshold_tokens,
        context_before: outcome.context_before,
        context_after: outcome.context_after,
        rule: outcome.rule.clone(),
    };
    if let Err(error) = update_ring(deps, |ring| ring.entries.push(entry.clone())).await {
        tracing::warn!(session_id = %outcome.session_id, %error, "autocompact log not updated");
    }
}

pub async fn log(deps: &Deps, request: LogRequest) -> Result<Vec<LogEntry>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let limit = request.limit.unwrap_or(LOG_DEFAULT_LIMIT).min(LOG_CAPACITY);
    let ring: LogRing =
        match crate::state::state_get(&deps.iii, LOG_SCOPE, LOG_KEY, timeout).await? {
            Value::Null => LogRing::default(),
            value => serde_json::from_value(value).unwrap_or_default(),
        };
    Ok(ring.entries.into_iter().rev().take(limit).collect())
}

/// A deleted session leaves no back-off and no log entries.
pub async fn forget_session(deps: &Deps, session_id: &str) -> Result<(), HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    crate::state::state_delete(&deps.iii, BACKOFF_SCOPE, session_id, timeout).await?;
    let listed = crate::state::state_get(&deps.iii, LOG_SCOPE, LOG_KEY, timeout).await?;
    let mentioned = serde_json::from_value::<LogRing>(listed).is_ok_and(|ring| {
        ring.entries
            .iter()
            .any(|entry| entry.session_id == session_id)
    });
    if mentioned {
        update_ring(deps, |ring| {
            ring.entries.retain(|entry| entry.session_id != session_id)
        })
        .await?;
    }
    Ok(())
}

// ── the models index ──

/// One model in `harness_autocompact_models`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ModelUse {
    provider: String,
    model: String,
    usable: u64,
    last_used_ms: i64,
}

/// One model `harness::autocompact::models` lists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct AutocompactModel {
    pub provider: String,
    pub model: String,
    /// The catalog's display name, else the model id.
    pub name: String,
    /// The catalog's window; `null` when the router does not list it.
    pub context_window: Option<u64>,
    /// The input budget context-manager measured on its last turn: where it
    /// compacts at its own limit.
    pub usable: u64,
    pub last_used_ms: i64,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ModelsRequest {}

/// What this process last wrote for each model: `(usable, at_ms)`.
fn noted() -> &'static Mutex<HashMap<String, (u64, i64)>> {
    static NOTED: OnceLock<Mutex<HashMap<String, (u64, i64)>>> = OnceLock::new();
    NOTED.get_or_init(Mutex::default)
}

/// Record that `model` ran a turn with this `usable`. Best effort; skipped
/// while this process wrote the same value recently.
pub async fn note_model(deps: &Deps, provider: Option<&str>, model: &str, usable: u64) {
    if model.is_empty() || usable == 0 {
        return;
    }
    // A model named without its provider: the id's prefix, else the
    // router's default route (empty).
    let provider = provider
        .filter(|provider| !provider.is_empty())
        .or_else(|| model.split_once('/').map(|(provider, _)| provider))
        .unwrap_or_default();
    let key = crate::ids::model_key(provider, model);
    let now = AgentMessage::now_ms();
    {
        let mut noted = noted().lock().unwrap_or_else(|p| p.into_inner());
        if noted
            .get(&key)
            .is_some_and(|(seen, at)| *seen == usable && now - at < MODELS_REFRESH_MS)
        {
            return;
        }
        noted.insert(key.clone(), (usable, now));
    }
    let entry = ModelUse {
        provider: provider.to_string(),
        model: model.to_string(),
        usable,
        last_used_ms: now,
    };
    let timeout = deps.cfg().await.session_timeout_ms;
    let value = serde_json::to_value(&entry).unwrap_or(Value::Null);
    if let Err(error) = crate::state::state_set(&deps.iii, MODELS_SCOPE, &key, value, timeout).await
    {
        noted()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&key);
        tracing::warn!(provider, model, %error, "autocompact models index not updated");
    }
}

pub async fn models(
    deps: &Deps,
    _request: ModelsRequest,
) -> Result<Vec<AutocompactModel>, HarnessError> {
    let timeout = deps.cfg().await.session_timeout_ms;
    let rows: Vec<Value> = crate::state::list_values(&deps.iii, MODELS_SCOPE, timeout).await?;
    let since = AgentMessage::now_ms() - MODELS_WINDOW_MS;
    let mut used: Vec<ModelUse> = rows
        .into_iter()
        .filter_map(|row| serde_json::from_value::<ModelUse>(row).ok())
        .filter(|row| row.last_used_ms >= since)
        .collect();
    used.sort_by(|a, b| {
        b.last_used_ms
            .cmp(&a.last_used_ms)
            .then_with(|| a.model.cmp(&b.model))
    });
    let router = deps.router().await;
    let lookups = used.iter().map(|row| {
        let router = router.clone();
        async move {
            let provider = (!row.provider.is_empty()).then_some(row.provider.as_str());
            router.models_get(provider, &row.model).await
        }
    });
    let catalog = futures_util::future::join_all(lookups).await;
    Ok(used
        .into_iter()
        .zip(catalog)
        .map(|(row, model)| AutocompactModel {
            name: model
                .as_ref()
                .and_then(|model| model.display_name.clone())
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| row.model.clone()),
            context_window: model.as_ref().map(|model| model.context_window),
            provider: row.provider,
            model: row.model,
            usable: row.usable,
            last_used_ms: row.last_used_ms,
        })
        .collect())
}

#[cfg(test)]
mod tests;
