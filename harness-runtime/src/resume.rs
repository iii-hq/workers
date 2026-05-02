use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use harness_types::AgentMessage;
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;
use sha2::{Digest, Sha256};

const STATE_SCOPE: &str = "agent";
const POLL_INTERVAL_MS: u64 = 100;
const RESUME_TIMEOUT_MS: u64 = 600_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    NoSuchSession {
        session_id: String,
    },
    StaleIndex {
        cwd: String,
        session_id: String,
    },
    NoInProgressForCwd {
        cwd: String,
        terminal_session_id: Option<String>,
    },
    StateDecodeFailed(String),
    BusError(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionInfo {
    pub session_id: String,
    pub state: String,
    pub cwd: Option<String>,
    pub started_at_ms: i64,
    pub last_state_change_ms: i64,
    pub last_assistant_snippet: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
struct TurnStateSnapshot {
    session_id: String,
    state: String,
    #[serde(default)]
    started_at_ms: i64,
    #[serde(default, alias = "last_state_change_ms")]
    updated_at_ms: i64,
}

fn decode_state_value(value: Value) -> Result<TurnStateSnapshot, ResumeError> {
    let raw = value
        .get("value")
        .filter(|_| value.get("session_id").is_none())
        .cloned()
        .unwrap_or(value);
    serde_json::from_value(raw).map_err(|e| ResumeError::StateDecodeFailed(e.to_string()))
}

fn decode_state_list(value: Value) -> Vec<(String, Value)> {
    match value {
        Value::Array(items) => items
            .into_iter()
            .filter_map(decode_state_list_item)
            .collect(),
        Value::Object(mut map) => {
            if let Some(items) = map
                .remove("items")
                .and_then(|items| items.as_array().cloned())
            {
                return items
                    .into_iter()
                    .filter_map(decode_state_list_item)
                    .collect();
            }
            let object_value = Value::Object(map.clone());
            if let Some(key) = synthetic_turn_state_key(&object_value) {
                return vec![(key, object_value)];
            }
            map.into_iter().collect()
        }
        _ => Vec::new(),
    }
}

fn decode_state_list_item(item: Value) -> Option<(String, Value)> {
    match item {
        Value::Object(mut map) => {
            if let Some(key) = map
                .remove("key")
                .and_then(|key| key.as_str().map(str::to_string))
            {
                let value = map.remove("value").unwrap_or(Value::Null);
                return Some((key, value));
            }
            let value = Value::Object(map);
            synthetic_turn_state_key(&value).map(|key| (key, value))
        }
        Value::Array(mut pair) if pair.len() == 2 => {
            let value = pair.pop().unwrap_or(Value::Null);
            let key = pair.pop()?.as_str()?.to_string();
            Some((key, value))
        }
        _ => None,
    }
}

fn synthetic_turn_state_key(value: &Value) -> Option<String> {
    let session_id = value.get("session_id")?.as_str()?;
    value.get("state")?.as_str()?;
    Some(format!("session/{session_id}/turn_state"))
}

async fn state_get(iii: &III, key: &str) -> Result<Option<Value>, ResumeError> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "state::get".into(),
            payload: json!({ "scope": STATE_SCOPE, "key": key }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| ResumeError::BusError(e.to_string()))?;
    let value = value
        .get("value")
        .filter(|_| value.get("key").is_some() || value.as_object().is_some_and(|o| o.len() == 1))
        .cloned()
        .unwrap_or(value);
    Ok((!value.is_null()).then_some(value))
}

async fn state_list(iii: &III, prefix: &str) -> Result<Vec<(String, Value)>, ResumeError> {
    let value = iii
        .trigger(TriggerRequest {
            function_id: "state::list".into(),
            payload: json!({ "scope": STATE_SCOPE, "prefix": prefix }),
            action: None,
            timeout_ms: None,
        })
        .await
        .map_err(|e| ResumeError::BusError(e.to_string()))?;
    Ok(decode_state_list(value))
}

async fn load_messages(iii: &III, session_id: &str) -> Result<Vec<AgentMessage>, ResumeError> {
    let Some(value) = state_get(iii, &format!("session/{session_id}/messages")).await? else {
        return Ok(Vec::new());
    };
    serde_json::from_value(value).map_err(|e| ResumeError::StateDecodeFailed(e.to_string()))
}

pub async fn resume_session(iii: &III, session_id: &str) -> Result<Vec<AgentMessage>, ResumeError> {
    let key = format!("session/{session_id}/turn_state");
    let Some(value) = state_get(iii, &key).await? else {
        return Err(ResumeError::NoSuchSession {
            session_id: session_id.to_string(),
        });
    };
    let snapshot = decode_state_value(value)?;
    if snapshot.state == "stopped" {
        return load_messages(iii, session_id).await;
    }

    publish_step(iii, session_id).await?;
    wait_until_terminal(iii, session_id).await?;
    load_messages(iii, session_id).await
}

async fn publish_step(iii: &III, session_id: &str) -> Result<(), ResumeError> {
    iii.trigger(TriggerRequest {
        function_id: "publish".into(),
        payload: json!({
            "topic": "turn::step_requested",
            "data": { "session_id": session_id },
        }),
        action: None,
        timeout_ms: None,
    })
    .await
    .map(|_| ())
    .map_err(|e| ResumeError::BusError(e.to_string()))
}

async fn wait_until_terminal(iii: &III, session_id: &str) -> Result<(), ResumeError> {
    let deadline = Instant::now() + Duration::from_millis(RESUME_TIMEOUT_MS);
    loop {
        let key = format!("session/{session_id}/turn_state");
        let Some(value) = state_get(iii, &key).await? else {
            return Err(ResumeError::NoSuchSession {
                session_id: session_id.to_string(),
            });
        };
        if decode_state_value(value)?.state == "stopped" {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(ResumeError::BusError(format!(
                "resume timed out after {RESUME_TIMEOUT_MS} ms"
            )));
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

pub async fn continue_session(iii: &III, cwd: &Path) -> Result<Vec<AgentMessage>, ResumeError> {
    let session_id = resolve_continue_session_id(iii, cwd).await?;
    resume_session(iii, &session_id).await
}

pub async fn resolve_continue_session_id(iii: &III, cwd: &Path) -> Result<String, ResumeError> {
    let cwd = canonicalize_for_index(cwd);
    let cwd_display = cwd.display().to_string();
    let key = cwd_index_key(&cwd);
    let Some(value) = state_get(iii, &key).await? else {
        return Err(ResumeError::NoInProgressForCwd {
            cwd: cwd_display,
            terminal_session_id: None,
        });
    };
    let session_id = value
        .as_str()
        .ok_or_else(|| ResumeError::StateDecodeFailed(format!("{key} was not a string")))?
        .to_string();

    let state_key = format!("session/{session_id}/turn_state");
    let Some(state) = state_get(iii, &state_key).await? else {
        return Err(ResumeError::StaleIndex {
            cwd: cwd_display,
            session_id,
        });
    };
    if decode_state_value(state)?.state == "stopped" {
        return Err(ResumeError::NoInProgressForCwd {
            cwd: cwd_display,
            terminal_session_id: Some(session_id),
        });
    }
    Ok(session_id)
}

pub async fn list_sessions(iii: &III) -> Result<Vec<SessionInfo>, ResumeError> {
    let entries = state_list(iii, "session/").await?;
    let mut sessions = Vec::new();
    for (key, value) in entries {
        if !key.ends_with("/turn_state") {
            continue;
        }
        let snap = decode_state_value(value)?;
        let cwd = state_get(iii, &session_cwd_key(&snap.session_id))
            .await?
            .and_then(|v| v.as_str().map(str::to_string));
        let messages = load_messages(iii, &snap.session_id)
            .await
            .unwrap_or_default();
        sessions.push(SessionInfo {
            session_id: snap.session_id,
            state: snap.state,
            cwd,
            started_at_ms: snap.started_at_ms,
            last_state_change_ms: snap.updated_at_ms,
            last_assistant_snippet: snippet_from_messages(&messages, 80),
        });
    }
    sessions.sort_by(|a, b| b.last_state_change_ms.cmp(&a.last_state_change_ms));
    Ok(sessions)
}

pub fn cwd_hash(cwd: &Path) -> String {
    let canonical = canonicalize_for_index(cwd);
    let mut hasher = Sha256::new();
    update_hash_with_path(&mut hasher, &canonical);
    hex::encode(hasher.finalize())
}

#[cfg(unix)]
fn update_hash_with_path(hasher: &mut Sha256, path: &Path) {
    use std::os::unix::ffi::OsStrExt;

    hasher.update(path.as_os_str().as_bytes());
}

#[cfg(not(unix))]
fn update_hash_with_path(hasher: &mut Sha256, path: &Path) {
    hasher.update(path.to_string_lossy().as_bytes());
}

pub fn canonicalize_for_index(cwd: &Path) -> PathBuf {
    cwd.canonicalize().unwrap_or_else(|_| normalize_lossy(cwd))
}

pub fn cwd_index_key(cwd: &Path) -> String {
    format!("harness/cwd/{}/last_session_id", cwd_hash(cwd))
}

pub fn session_cwd_key(session_id: &str) -> String {
    format!("session/{session_id}/cwd")
}

fn normalize_lossy(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                if matches!(
                    out.components().next_back(),
                    Some(std::path::Component::Normal(_))
                ) {
                    out.pop();
                } else {
                    out.push(component.as_os_str());
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[allow(dead_code)]
pub(crate) fn snippet_from_messages(messages: &[AgentMessage], limit: usize) -> Option<String> {
    let text = messages.iter().rev().find_map(|m| match m {
        AgentMessage::Assistant(a) => a.content.iter().find_map(|c| match c {
            harness_types::ContentBlock::Text(t) if !t.text.trim().is_empty() => {
                Some(t.text.trim().to_string())
            }
            _ => None,
        }),
        _ => None,
    })?;

    if text.chars().count() <= limit {
        Some(text)
    } else {
        let mut s: String = text.chars().take(limit).collect();
        s.push('…');
        Some(s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use harness_types::{AgentMessage, AssistantMessage, ContentBlock, StopReason, TextContent};
    use serde_json::json;

    fn assistant_text(text: &str) -> AgentMessage {
        AgentMessage::Assistant(AssistantMessage {
            content: vec![ContentBlock::Text(TextContent { text: text.into() })],
            stop_reason: StopReason::End,
            error_message: None,
            error_kind: None,
            usage: None,
            model: "m".into(),
            provider: "p".into(),
            timestamp: 0,
        })
    }

    #[test]
    fn decode_state_value_reads_turn_state_snapshot() {
        let got = decode_state_value(json!({
            "session_id": "cli-1",
            "state": "stopped",
            "started_at_ms": 10,
            "updated_at_ms": 20
        }))
        .unwrap();

        assert_eq!(got.session_id, "cli-1");
        assert_eq!(got.state, "stopped");
        assert_eq!(got.started_at_ms, 10);
        assert_eq!(got.updated_at_ms, 20);
    }

    #[test]
    fn decode_state_value_accepts_last_state_change_alias() {
        let got = decode_state_value(json!({
            "session_id": "cli-1",
            "state": "stopped",
            "started_at_ms": 10,
            "last_state_change_ms": 30
        }))
        .unwrap();

        assert_eq!(got.updated_at_ms, 30);
    }

    #[test]
    fn decode_state_list_accepts_items_envelope() {
        let got = decode_state_list(json!({
            "items": [{ "key": "session/a/turn_state", "value": { "session_id": "a" } }]
        }));

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "session/a/turn_state");
        assert_eq!(got[0].1, json!({ "session_id": "a" }));
    }

    #[test]
    fn decode_state_list_derives_key_for_bare_turn_state_values() {
        let got = decode_state_list(json!([{
            "session_id": "a",
            "state": "stopped",
            "started_at_ms": 1,
            "updated_at_ms": 2
        }]));

        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "session/a/turn_state");
        assert!(got[0].0.ends_with("/turn_state"));
        assert_eq!(got[0].1["session_id"], "a");
        assert_eq!(got[0].1["state"], "stopped");
    }

    #[test]
    fn cwd_hash_is_sha256_hex_and_stable() {
        let h1 = cwd_hash(Path::new("/tmp/harness"));
        let h2 = cwd_hash(Path::new("/tmp/harness"));
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 64);
        assert!(h1.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn cwd_hash_distinguishes_paths() {
        assert_ne!(cwd_hash(Path::new("/tmp/a")), cwd_hash(Path::new("/tmp/b")));
    }

    #[cfg(unix)]
    #[test]
    fn cwd_hash_distinguishes_non_utf8_paths() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        let a = PathBuf::from(OsString::from_vec(b"/tmp/harness-\xff".to_vec()));
        let b = PathBuf::from(OsString::from_vec(b"/tmp/harness-\xfe".to_vec()));

        assert_eq!(a.to_string_lossy(), b.to_string_lossy());
        assert_ne!(cwd_hash(&a), cwd_hash(&b));
    }

    #[test]
    fn canonicalize_for_index_preserves_leading_parent_for_missing_relative_path() {
        let path = Path::new("../project-that-should-not-exist-for-resume-test");
        let normalized = canonicalize_for_index(path);
        assert_eq!(normalized, PathBuf::from(path));
    }

    #[test]
    fn cwd_index_key_uses_harness_namespace() {
        let key = cwd_index_key(Path::new("/tmp/harness"));
        assert!(key.starts_with("harness/cwd/"));
        assert!(key.ends_with("/last_session_id"));
    }

    #[test]
    fn session_cwd_key_uses_session_namespace() {
        assert_eq!(session_cwd_key("cli-1"), "session/cli-1/cwd");
    }

    #[test]
    fn snippet_uses_last_assistant_text_and_truncates() {
        let messages = vec![assistant_text(&"x".repeat(120))];
        let snippet = snippet_from_messages(&messages, 80).unwrap();
        assert!(snippet.chars().count() <= 81);
        assert!(snippet.ends_with('…'));
    }

    #[test]
    fn snippet_truncates_by_character_count_for_multibyte_text() {
        let messages = vec![assistant_text("åßçdé")];
        let snippet = snippet_from_messages(&messages, 4).unwrap();
        assert_eq!(snippet, "åßçd…");
    }
}
