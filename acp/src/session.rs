use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const STATE_TIMEOUT_MS: u64 = 5_000;
const HISTORY_UPDATE_ATTEMPTS: usize = 16;
const CURSOR_ITEM_REPLAY_WINDOW: usize = 4_096;

pub fn scope() -> String {
    "acp-v0.3".to_string()
}

// Persisted keys are NOT scoped by conn_id. session_id is a globally
// unique uuid (sess_<32hex>) and must survive subprocess restarts so a
// reconnecting editor can resume an old thread via session/load. conn_id
// stays in-memory only as transient ownership metadata for routing
// agent::events to the right subprocess.
pub fn session_key(session_id: &str) -> String {
    format!("sessions:{}", session_id)
}

pub fn session_index_key() -> &'static str {
    "sessions:_index"
}

pub fn session_history_key(session_id: &str) -> String {
    format!("sessions:{}:history", session_id)
}

// Streaming wire = the iii ecosystem's `agent::events` stream. No
// per-connection topic exists. Brains (turn-orchestrator and any
// drop-in replacement) emit AgentEvent frames into that stream with
// group_id = session_id; iii-acp subscribes once and routes by group.
pub const AGENT_EVENTS_STREAM: &str = "agent::events";

pub fn cancel_topic(conn_id: &str, session_id: &str) -> String {
    format!("acp:{}:session:{}:cancel", conn_id, session_id)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub conn_id: String,
    pub cwd: String,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    pub created_at_ms: i64,
    pub last_activity_ms: i64,
    // Optional ACP mode set via session/set_mode. None until first set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    // Per-session config options set via session/set_config_option. Keys are
    // configId strings, values are arbitrary JSON.
    #[serde(default)]
    pub config_options: serde_json::Map<String, Value>,
}

pub async fn state_get(iii: &IIIClient, scope: &str, key: &str) -> Result<Option<Value>, Error> {
    let result = iii
        .trigger(TriggerRequest {
            function_id: "state::get".to_string(),
            payload: json!({ "scope": scope, "key": key }),
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await;
    match result {
        Ok(val) => Ok(unwrap_value(val)),
        Err(e) => {
            let msg = e.to_string().to_lowercase();
            if msg.contains("not found") || msg.contains("no such") {
                Ok(None)
            } else {
                Err(e)
            }
        }
    }
}

pub async fn state_set(iii: &IIIClient, scope: &str, key: &str, value: Value) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::set".to_string(),
        payload: json!({ "scope": scope, "key": key, "value": value }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn state_compare_and_set(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    expected: Option<&Value>,
    value: Value,
) -> Result<bool, Error> {
    Ok(
        state_compare_and_set_with_current(iii, scope, key, expected, value)
            .await?
            .swapped,
    )
}

struct StateCompareAndSetResult {
    swapped: bool,
    current: Option<Value>,
}

async fn state_compare_and_set_with_current(
    iii: &IIIClient,
    scope: &str,
    key: &str,
    expected: Option<&Value>,
    value: Value,
) -> Result<StateCompareAndSetResult, Error> {
    let mut payload = json!({ "scope": scope, "key": key, "value": value });
    if let Some(expected) = expected {
        payload["expected"] = expected.clone();
    }
    let response = iii
        .trigger(TriggerRequest {
            function_id: "state::compare-and-set".to_string(),
            payload,
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await?;
    let result = unwrap_value(response).unwrap_or(Value::Null);
    Ok(StateCompareAndSetResult {
        swapped: result
            .get("swapped")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        current: result
            .get("current")
            .cloned()
            .filter(|value| !value.is_null()),
    })
}

pub async fn state_delete(iii: &IIIClient, scope: &str, key: &str) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "state::delete".to_string(),
        payload: json!({ "scope": scope, "key": key }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

pub async fn durable_publish(iii: &IIIClient, topic: &str, data: Value) -> Result<(), Error> {
    iii.trigger(TriggerRequest {
        function_id: "iii::durable::publish".to_string(),
        payload: json!({ "topic": topic, "data": data }),
        action: None,
        timeout_ms: Some(STATE_TIMEOUT_MS),
    })
    .await?;
    Ok(())
}

fn unwrap_value(v: Value) -> Option<Value> {
    if v.is_null() {
        return None;
    }
    if let Some(obj) = v.as_object() {
        if let Some(inner) = obj.get("value") {
            if inner.is_null() {
                return None;
            }
            return Some(inner.clone());
        }
        if obj.is_empty() {
            return None;
        }
    }
    Some(v)
}

pub async fn append_history(iii: &IIIClient, session_id: &str, entry: Value) -> Result<(), Error> {
    append_history_once(iii, session_id, None, None, vec![entry])
        .await
        .map(|_| ())
}

pub async fn append_history_once(
    iii: &IIIClient,
    session_id: &str,
    owner_conn_id: Option<&str>,
    cursor_item_id: Option<&str>,
    entries: Vec<Value>,
) -> Result<bool, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to append safely",
        |history| {
            if apply_history_update(history, owner_conn_id, cursor_item_id, entries.clone()) {
                HistoryMutation::Commit(true)
            } else {
                HistoryMutation::Return(false)
            }
        },
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryOwnerResult {
    AlreadyOwned,
    Transferred { previous_owner: Option<String> },
    ActivePrompt(ActivePromptClaim),
    Closed,
}

pub async fn set_history_owner(
    iii: &IIIClient,
    session_id: &str,
    owner_conn_id: &str,
) -> Result<HistoryOwnerResult, Error> {
    update_history(
        iii,
        session_id,
        "history owner changed too frequently to update safely",
        |history| {
            let result = apply_owner_transfer(history, owner_conn_id);
            if matches!(result, HistoryOwnerResult::Transferred { .. }) {
                HistoryMutation::Commit(result)
            } else {
                HistoryMutation::Return(result)
            }
        },
    )
    .await
}

pub async fn restore_history_owner(
    iii: &IIIClient,
    session_id: &str,
    current_owner_conn_id: &str,
    previous_owner_conn_id: Option<&str>,
) -> Result<bool, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to restore ownership safely",
        |history| {
            if history.closed
                || history.active_prompt.is_some()
                || history.owner_conn_id.as_deref() != Some(current_owner_conn_id)
            {
                return HistoryMutation::Return(false);
            }
            history.owner_conn_id = previous_owner_conn_id.map(str::to_string);
            HistoryMutation::Commit(true)
        },
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptClaimResult {
    Claimed,
    AlreadyActive,
    NotOwner,
    Closed,
}

pub async fn claim_prompt(
    iii: &IIIClient,
    session_id: &str,
    owner_conn_id: &str,
    claim_id: &str,
    started_at_ms: i64,
    dispatch: PromptDispatchIdentity,
    user_entry: Value,
) -> Result<PromptClaimResult, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to claim a prompt safely",
        |history| {
            let result = apply_prompt_claim(
                history,
                owner_conn_id,
                claim_id,
                started_at_ms,
                dispatch.clone(),
                user_entry.clone(),
            );
            if result == PromptClaimResult::Claimed {
                HistoryMutation::Commit(result)
            } else {
                HistoryMutation::Return(result)
            }
        },
    )
    .await
}

pub async fn release_prompt_claim(
    iii: &IIIClient,
    session_id: &str,
    owner_conn_id: &str,
    claim_id: &str,
) -> Result<bool, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to release a prompt safely",
        |history| {
            if apply_prompt_release(history, owner_conn_id, claim_id) {
                HistoryMutation::Commit(true)
            } else {
                HistoryMutation::Return(false)
            }
        },
    )
    .await
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptRecoveryResult {
    Claimed(ActivePromptClaim),
    Changed,
    Closed,
}

pub async fn begin_prompt_recovery(
    iii: &IIIClient,
    session_id: &str,
    expected_claim: &ActivePromptClaim,
    recovery_owner_conn_id: &str,
    recovery_claim_id: &str,
    recovery_started_at_ms: i64,
) -> Result<PromptRecoveryResult, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to begin prompt recovery safely",
        |history| {
            let result = apply_prompt_recovery(
                history,
                expected_claim,
                recovery_owner_conn_id,
                recovery_claim_id,
                recovery_started_at_ms,
            );
            if matches!(result, PromptRecoveryResult::Claimed(_)) {
                HistoryMutation::Commit(result)
            } else {
                HistoryMutation::Return(result)
            }
        },
    )
    .await
}

pub async fn read_active_prompt_claim(
    iii: &IIIClient,
    session_id: &str,
) -> Result<Option<ActivePromptClaim>, Error> {
    let scope = scope();
    let key = session_history_key(session_id);
    Ok(decode_history(state_get(iii, &scope, &key).await?.as_ref())?.active_prompt)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptRecoveryFinishResult {
    AlreadyOwned,
    Transferred { previous_owner: Option<String> },
    Changed,
    Closed,
}

pub async fn finish_prompt_recovery(
    iii: &IIIClient,
    session_id: &str,
    recovery_claim: &ActivePromptClaim,
    new_owner_conn_id: &str,
) -> Result<PromptRecoveryFinishResult, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to finish prompt recovery safely",
        |history| {
            let result = apply_prompt_recovery_finish(history, recovery_claim, new_owner_conn_id);
            if matches!(
                result,
                PromptRecoveryFinishResult::Changed | PromptRecoveryFinishResult::Closed
            ) {
                HistoryMutation::Return(result)
            } else {
                HistoryMutation::Commit(result)
            }
        },
    )
    .await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseHistoryResult {
    Closed,
    AlreadyClosed,
    ActivePrompt,
    NotOwner,
}

pub async fn close_history_owned_by(
    iii: &IIIClient,
    session_id: &str,
    owner_conn_id: &str,
) -> Result<CloseHistoryResult, Error> {
    update_history(
        iii,
        session_id,
        "history changed too frequently to close safely",
        |history| {
            let result = apply_history_close(history, owner_conn_id);
            if result == CloseHistoryResult::Closed {
                HistoryMutation::Commit(result)
            } else {
                HistoryMutation::Return(result)
            }
        },
    )
    .await
}

pub async fn read_history(iii: &IIIClient, session_id: &str) -> Result<Vec<Value>, Error> {
    let scope = scope();
    let key = session_history_key(session_id);
    Ok(decode_history(state_get(iii, &scope, &key).await?.as_ref())?.entries)
}

pub async fn history_owned_by(
    iii: &IIIClient,
    session_id: &str,
    owner_conn_id: &str,
) -> Result<bool, Error> {
    let scope = scope();
    let key = session_history_key(session_id);
    let history = decode_history(state_get(iii, &scope, &key).await?.as_ref())?;
    Ok(history_owner_matches(&history, owner_conn_id))
}

fn history_owner_matches(history: &HistoryState, owner_conn_id: &str) -> bool {
    !history.closed && history.owner_conn_id.as_deref() == Some(owner_conn_id)
}

#[derive(Default, Serialize, Deserialize)]
struct HistoryState {
    entries: Vec<Value>,
    #[serde(default)]
    cursor_item_ids: Vec<String>,
    #[serde(default)]
    owner_conn_id: Option<String>,
    #[serde(default)]
    active_prompt: Option<ActivePromptClaim>,
    #[serde(default)]
    closed: bool,
    #[serde(default)]
    closed_by_conn_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActivePromptClaim {
    pub claim_id: String,
    pub owner_conn_id: String,
    pub started_at_ms: i64,
    #[serde(default)]
    pub dispatch: Option<PromptDispatchIdentity>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptDispatchIdentity {
    pub namespace: Option<String>,
    pub brain_function_id: Option<String>,
    pub stop_function_id: Option<String>,
}

fn decode_history(value: Option<&Value>) -> Result<HistoryState, Error> {
    match value {
        None => Ok(HistoryState::default()),
        Some(Value::Array(entries)) => Ok(HistoryState {
            entries: entries.clone(),
            cursor_item_ids: Vec::new(),
            owner_conn_id: None,
            active_prompt: None,
            closed: false,
            closed_by_conn_id: None,
        }),
        Some(value) => serde_json::from_value(value.clone())
            .map_err(|error| Error::Handler(format!("history decode failed: {error}"))),
    }
}

enum HistoryMutation<T> {
    Return(T),
    Commit(T),
}

async fn update_history<T, F>(
    iii: &IIIClient,
    session_id: &str,
    exhaustion_error: &str,
    mut transition: F,
) -> Result<T, Error>
where
    F: FnMut(&mut HistoryState) -> HistoryMutation<T>,
{
    let scope = scope();
    let key = session_history_key(session_id);
    let mut current = state_get(iii, &scope, &key).await?;
    for _ in 0..HISTORY_UPDATE_ATTEMPTS {
        let mut history = decode_history(current.as_ref())?;
        let result = match transition(&mut history) {
            HistoryMutation::Return(result) => return Ok(result),
            HistoryMutation::Commit(result) => result,
        };
        let next = serde_json::to_value(history)
            .map_err(|error| Error::Handler(format!("history encode failed: {error}")))?;
        let cas =
            state_compare_and_set_with_current(iii, &scope, &key, current.as_ref(), next).await?;
        if cas.swapped {
            return Ok(result);
        }
        current = cas.current;
    }
    Err(Error::Handler(exhaustion_error.to_string()))
}

fn apply_owner_transfer(history: &mut HistoryState, owner_conn_id: &str) -> HistoryOwnerResult {
    if history.closed {
        return HistoryOwnerResult::Closed;
    }
    if history.active_prompt.is_some() {
        return HistoryOwnerResult::ActivePrompt(
            history
                .active_prompt
                .clone()
                .expect("active prompt was checked"),
        );
    }
    if history.owner_conn_id.as_deref() == Some(owner_conn_id) {
        return HistoryOwnerResult::AlreadyOwned;
    }
    let previous_owner = history.owner_conn_id.replace(owner_conn_id.to_string());
    HistoryOwnerResult::Transferred { previous_owner }
}

fn apply_prompt_claim(
    history: &mut HistoryState,
    owner_conn_id: &str,
    claim_id: &str,
    started_at_ms: i64,
    dispatch: PromptDispatchIdentity,
    user_entry: Value,
) -> PromptClaimResult {
    if history.closed {
        return PromptClaimResult::Closed;
    }
    if history.owner_conn_id.as_deref() != Some(owner_conn_id) {
        return PromptClaimResult::NotOwner;
    }
    if history.active_prompt.is_some() {
        return PromptClaimResult::AlreadyActive;
    }
    history.active_prompt = Some(ActivePromptClaim {
        claim_id: claim_id.to_string(),
        owner_conn_id: owner_conn_id.to_string(),
        started_at_ms,
        dispatch: Some(dispatch),
    });
    history.entries.push(user_entry);
    PromptClaimResult::Claimed
}

fn apply_prompt_release(history: &mut HistoryState, owner_conn_id: &str, claim_id: &str) -> bool {
    let matches = history
        .active_prompt
        .as_ref()
        .is_some_and(|claim| claim.owner_conn_id == owner_conn_id && claim.claim_id == claim_id);
    if matches {
        history.active_prompt = None;
    }
    matches
}

fn apply_prompt_recovery(
    history: &mut HistoryState,
    expected_claim: &ActivePromptClaim,
    recovery_owner_conn_id: &str,
    recovery_claim_id: &str,
    recovery_started_at_ms: i64,
) -> PromptRecoveryResult {
    if history.closed {
        return PromptRecoveryResult::Closed;
    }
    if history.active_prompt.as_ref() != Some(expected_claim) {
        return PromptRecoveryResult::Changed;
    }
    let recovery_claim = ActivePromptClaim {
        claim_id: recovery_claim_id.to_string(),
        owner_conn_id: recovery_owner_conn_id.to_string(),
        started_at_ms: recovery_started_at_ms,
        dispatch: expected_claim.dispatch.clone(),
    };
    history.active_prompt = Some(recovery_claim.clone());
    PromptRecoveryResult::Claimed(recovery_claim)
}

fn apply_prompt_recovery_finish(
    history: &mut HistoryState,
    recovery_claim: &ActivePromptClaim,
    new_owner_conn_id: &str,
) -> PromptRecoveryFinishResult {
    if history.closed {
        return PromptRecoveryFinishResult::Closed;
    }
    if history.active_prompt.as_ref() != Some(recovery_claim) {
        return PromptRecoveryFinishResult::Changed;
    }
    history.active_prompt = None;
    if history.owner_conn_id.as_deref() == Some(new_owner_conn_id) {
        PromptRecoveryFinishResult::AlreadyOwned
    } else {
        let previous_owner = history.owner_conn_id.replace(new_owner_conn_id.to_string());
        PromptRecoveryFinishResult::Transferred { previous_owner }
    }
}

fn apply_history_close(history: &mut HistoryState, owner_conn_id: &str) -> CloseHistoryResult {
    if history.closed {
        return CloseHistoryResult::AlreadyClosed;
    }
    if history.owner_conn_id.as_deref() != Some(owner_conn_id) {
        return CloseHistoryResult::NotOwner;
    }
    if history.active_prompt.is_some() {
        return CloseHistoryResult::ActivePrompt;
    }
    history.entries.clear();
    history.cursor_item_ids.clear();
    history.owner_conn_id = None;
    history.closed = true;
    history.closed_by_conn_id = Some(owner_conn_id.to_string());
    CloseHistoryResult::Closed
}

fn apply_history_update(
    history: &mut HistoryState,
    owner_conn_id: Option<&str>,
    cursor_item_id: Option<&str>,
    entries: Vec<Value>,
) -> bool {
    if history.closed {
        return false;
    }
    if let Some(owner_conn_id) = owner_conn_id {
        if history.owner_conn_id.as_deref() != Some(owner_conn_id) {
            return false;
        }
    }
    if let Some(item_id) = cursor_item_id {
        if history.cursor_item_ids.iter().any(|seen| seen == item_id) {
            return false;
        }
        if history.cursor_item_ids.len() >= CURSOR_ITEM_REPLAY_WINDOW {
            let remove = history.cursor_item_ids.len() + 1 - CURSOR_ITEM_REPLAY_WINDOW;
            history.cursor_item_ids.drain(..remove);
        }
        history.cursor_item_ids.push(item_id.to_string());
    }
    history.entries.extend(entries);
    true
}

pub async fn append_session_to_index(iii: &IIIClient, session_id: &str) -> Result<(), Error> {
    // Read-modify-write under in-process index mutex (caller-owned).
    let scope = scope();
    let key = session_index_key();
    let mut idx = state_get(iii, &scope, key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let new_entry = Value::String(session_id.to_string());
    if !idx.contains(&new_entry) {
        idx.push(new_entry);
        state_set(iii, &scope, key, Value::Array(idx)).await?;
    }
    Ok(())
}

pub async fn remove_session_from_index(iii: &IIIClient, session_id: &str) -> Result<(), Error> {
    // state::update has no array-element-by-value remove op, so this stays
    // a read-modify-write. Race window: a concurrent append from
    // session/new for a different id can be lost. Acceptable in practice
    // because session/close is single-user / single-action and the
    // sweeper-style use case isn't part of v0.
    let scope = scope();
    let key = session_index_key();
    let idx = state_get(iii, &scope, key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    let entry = Value::String(session_id.to_string());
    let next: Vec<Value> = idx.into_iter().filter(|v| v != &entry).collect();
    state_set(iii, &scope, key, Value::Array(next)).await
}

pub async fn read_session_index(iii: &IIIClient) -> Result<Vec<String>, Error> {
    let scope = scope();
    let key = session_index_key();
    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::new();
    for v in state_get(iii, &scope, key)
        .await?
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
    {
        if let Some(s) = v.as_str() {
            // Dedupe on read — append_session_to_index uses an atomic
            // append, so the index can carry duplicates if the same id
            // ever lands twice.
            if seen.insert(s.to_string()) {
                out.push(s.to_string());
            }
        }
    }
    Ok(out)
}

pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_session_id_only() {
        assert_eq!(scope(), "acp-v0.3");
        assert_eq!(session_key("s1"), "sessions:s1");
        assert_eq!(session_index_key(), "sessions:_index");
        assert_eq!(session_history_key("s1"), "sessions:s1:history");
    }

    #[test]
    fn topics_namespace_globally() {
        assert_eq!(AGENT_EVENTS_STREAM, "agent::events");
        assert_eq!(cancel_topic("c1", "s1"), "acp:c1:session:s1:cancel");
    }

    #[test]
    fn unwrap_value_handles_envelope_and_bare() {
        assert_eq!(unwrap_value(json!(null)), None);
        assert_eq!(unwrap_value(json!({"value": null})), None);
        assert_eq!(unwrap_value(json!({"value": 42})), Some(json!(42)));
        assert_eq!(unwrap_value(json!({"a": 1})), Some(json!({"a": 1})));
        assert_eq!(unwrap_value(json!([1, 2, 3])), Some(json!([1, 2, 3])));
    }

    #[test]
    fn history_migrates_legacy_arrays_and_claims_cursor_items_once() {
        let legacy = json!([{ "sessionUpdate": "user_message_chunk" }]);
        let mut history = decode_history(Some(&legacy)).unwrap();

        assert!(apply_history_update(
            &mut history,
            None,
            Some("cursor-item"),
            vec![json!({ "sessionUpdate": "agent_message_chunk" })],
        ));
        assert!(!apply_history_update(
            &mut history,
            None,
            Some("cursor-item"),
            vec![json!({ "sessionUpdate": "agent_message_chunk" })],
        ));
        assert_eq!(history.entries.len(), 2);
        assert_eq!(history.cursor_item_ids, vec!["cursor-item"]);
    }

    #[test]
    fn cursor_item_dedup_keeps_only_the_recent_replay_window() {
        let mut history = HistoryState::default();
        for index in 0..=CURSOR_ITEM_REPLAY_WINDOW {
            assert!(apply_history_update(
                &mut history,
                None,
                Some(&format!("cursor-{index}")),
                Vec::new(),
            ));
        }
        assert_eq!(history.cursor_item_ids.len(), CURSOR_ITEM_REPLAY_WINDOW);
        assert!(
            !history
                .cursor_item_ids
                .iter()
                .any(|item| item == "cursor-0")
        );
        assert!(!apply_history_update(
            &mut history,
            None,
            Some(&format!("cursor-{}", CURSOR_ITEM_REPLAY_WINDOW)),
            Vec::new(),
        ));
        assert!(apply_history_update(
            &mut history,
            None,
            Some("cursor-0"),
            Vec::new(),
        ));
        let encoded = serde_json::to_value(&history).unwrap();
        let restored = decode_history(Some(&encoded)).unwrap();
        assert_eq!(restored.cursor_item_ids, history.cursor_item_ids);
    }

    #[test]
    fn history_owner_transfer_routes_new_items_only_to_the_new_connection() {
        let mut history = HistoryState {
            owner_conn_id: Some("old".to_string()),
            ..HistoryState::default()
        };

        history.owner_conn_id = Some("new".to_string());

        assert!(!apply_history_update(
            &mut history,
            Some("old"),
            Some("cursor-after-transfer"),
            vec![json!({ "sessionUpdate": "agent_message_chunk" })],
        ));
        assert!(apply_history_update(
            &mut history,
            Some("new"),
            Some("cursor-after-transfer"),
            vec![json!({ "sessionUpdate": "agent_message_chunk" })],
        ));
        assert_eq!(history.entries.len(), 1);
    }

    #[test]
    fn history_owner_comparison_rejects_the_previous_connection() {
        let history = HistoryState {
            owner_conn_id: Some("new".to_string()),
            ..HistoryState::default()
        };

        assert!(!history_owner_matches(&history, "old"));
        assert!(history_owner_matches(&history, "new"));
    }

    #[test]
    fn closed_history_rejects_ownership_and_new_entries() {
        let mut history = HistoryState {
            owner_conn_id: Some("owner".to_string()),
            closed: true,
            ..HistoryState::default()
        };

        assert!(!history_owner_matches(&history, "owner"));
        assert!(!apply_history_update(
            &mut history,
            Some("owner"),
            None,
            vec![json!({ "sessionUpdate": "agent_message_chunk" })],
        ));
        assert!(history.entries.is_empty());
    }

    #[test]
    fn active_prompt_blocks_takeover_and_close_until_exact_release() {
        let mut history = HistoryState {
            owner_conn_id: Some("owner".to_string()),
            ..HistoryState::default()
        };
        assert_eq!(
            apply_prompt_claim(
                &mut history,
                "owner",
                "claim-one",
                42,
                PromptDispatchIdentity {
                    namespace: Some("test".to_string()),
                    brain_function_id: Some("brain::run".to_string()),
                    stop_function_id: Some("brain::stop".to_string()),
                },
                json!({ "sessionUpdate": "user_message_chunk" }),
            ),
            PromptClaimResult::Claimed
        );
        assert_eq!(
            apply_owner_transfer(&mut history, "owner"),
            HistoryOwnerResult::ActivePrompt(ActivePromptClaim {
                claim_id: "claim-one".to_string(),
                owner_conn_id: "owner".to_string(),
                started_at_ms: 42,
                dispatch: Some(PromptDispatchIdentity {
                    namespace: Some("test".to_string()),
                    brain_function_id: Some("brain::run".to_string()),
                    stop_function_id: Some("brain::stop".to_string()),
                }),
            })
        );
        assert_eq!(
            apply_owner_transfer(&mut history, "other"),
            HistoryOwnerResult::ActivePrompt(ActivePromptClaim {
                claim_id: "claim-one".to_string(),
                owner_conn_id: "owner".to_string(),
                started_at_ms: 42,
                dispatch: Some(PromptDispatchIdentity {
                    namespace: Some("test".to_string()),
                    brain_function_id: Some("brain::run".to_string()),
                    stop_function_id: Some("brain::stop".to_string()),
                }),
            })
        );
        assert_eq!(
            apply_history_close(&mut history, "owner"),
            CloseHistoryResult::ActivePrompt
        );
        assert!(!apply_prompt_release(&mut history, "owner", "wrong"));
        assert!(!apply_prompt_release(&mut history, "other", "claim-one"));
        assert!(apply_prompt_release(&mut history, "owner", "claim-one"));
        assert_eq!(
            apply_owner_transfer(&mut history, "other"),
            HistoryOwnerResult::Transferred {
                previous_owner: Some("owner".to_string())
            }
        );
    }

    #[test]
    fn recovery_claim_pins_the_observed_prompt_before_stopping_it() {
        let dispatch = PromptDispatchIdentity {
            namespace: Some("test".to_string()),
            brain_function_id: Some("brain::run".to_string()),
            stop_function_id: Some("brain::stop".to_string()),
        };
        let original_claim = ActivePromptClaim {
            claim_id: "prompt-a".to_string(),
            owner_conn_id: "process-a".to_string(),
            started_at_ms: 1,
            dispatch: Some(dispatch.clone()),
        };
        let mut history = HistoryState {
            owner_conn_id: Some("process-a".to_string()),
            active_prompt: Some(original_claim.clone()),
            ..HistoryState::default()
        };

        let recovery_claim = match apply_prompt_recovery(
            &mut history,
            &original_claim,
            "process-b",
            "recovery-b",
            2,
        ) {
            PromptRecoveryResult::Claimed(claim) => claim,
            result => panic!("unexpected recovery result: {result:?}"),
        };

        assert!(!apply_prompt_release(&mut history, "process-a", "prompt-a"));
        assert_eq!(
            apply_prompt_claim(
                &mut history,
                "process-a",
                "prompt-b",
                3,
                dispatch,
                json!({ "sessionUpdate": "user_message_chunk" }),
            ),
            PromptClaimResult::AlreadyActive
        );
        assert_eq!(
            apply_prompt_recovery(&mut history, &original_claim, "process-c", "recovery-c", 3,),
            PromptRecoveryResult::Changed
        );
        assert_eq!(
            apply_prompt_recovery_finish(&mut history, &recovery_claim, "process-b"),
            PromptRecoveryFinishResult::Transferred {
                previous_owner: Some("process-a".to_string())
            }
        );
        assert_eq!(
            apply_prompt_claim(
                &mut history,
                "process-a",
                "prompt-b",
                3,
                PromptDispatchIdentity {
                    namespace: Some("test".to_string()),
                    brain_function_id: Some("brain::run".to_string()),
                    stop_function_id: Some("brain::stop".to_string()),
                },
                json!({ "sessionUpdate": "user_message_chunk" }),
            ),
            PromptClaimResult::NotOwner
        );
    }

    #[test]
    fn close_tombstone_allows_reconnect_cleanup_but_not_a_new_close() {
        let mut history = HistoryState {
            owner_conn_id: Some("owner".to_string()),
            ..HistoryState::default()
        };
        assert_eq!(
            apply_history_close(&mut history, "owner"),
            CloseHistoryResult::Closed
        );
        assert_eq!(
            apply_history_close(&mut history, "owner"),
            CloseHistoryResult::AlreadyClosed
        );
        assert_eq!(
            apply_history_close(&mut history, "other"),
            CloseHistoryResult::AlreadyClosed
        );
    }
}
