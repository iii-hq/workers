//! `harness::todo` — the session's structured task list (full-list replace,
//! TodoWrite semantics): the model maintains it during multi-step work so
//! the user can watch progress. Persists to the `harness_todo` state scope
//! and mirrors each update into a `custom` session entry (type "todo") the
//! console renders as a checklist (latest entry wins — `session::append`
//! is a no-op on repeated entry ids, so every update gets a fresh id).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::deps::Deps;
use crate::error::HarnessError;

pub const TODO_ID: &str = "harness::todo";
pub const TODO_DESC: &str = "Replace this session's structured todo list (full-list semantics — \
     send the COMPLETE list every time, not a delta). Use it for \
     multi-step tasks so progress is visible: at most 50 items, 500 \
     chars each, statuses pending | in_progress | completed, and keep \
     exactly ONE item in_progress while working. The list persists on \
     the session and renders as a live checklist in the console.";

/// State scope holding each session's current list.
pub const TODO_SCOPE: &str = "harness_todo";

const MAX_ITEMS: usize = 50;
const MAX_ITEM_CHARS: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TodoItem {
    /// The task, imperative and specific (≤ 500 chars). `content` is
    /// accepted as an alias — models trained on TodoWrite-shaped tools
    /// send it.
    #[serde(alias = "content")]
    pub text: String,
    pub status: TodoStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TodoRequest {
    /// The session whose list to replace. Callers inside a turn omit it —
    /// the harness injects the calling session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The complete list (replaces the previous one; empty clears it).
    /// `todos` is accepted as an alias for the same reason as `content`.
    #[serde(alias = "todos")]
    pub items: Vec<TodoItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TodoResponse {
    pub ok: bool,
    /// Items stored (after validation).
    pub count: u32,
}

pub async fn handle(deps: &Deps, req: TodoRequest) -> Result<TodoResponse, HarnessError> {
    let Some(session_id) = req.session_id.clone() else {
        return Err(HarnessError::InvalidRequest(
            "harness::todo requires a session_id (injected automatically for in-turn calls)".into(),
        ));
    };
    validate(&req.items)?;

    let cfg = deps.cfg().await;
    let value =
        json!({ "items": req.items, "updated_at": crate::types::message::AgentMessage::now_ms() });
    crate::state::put_scoped(
        &deps.iii,
        TODO_SCOPE,
        &session_id,
        &value,
        cfg.session_timeout_ms,
    )
    .await?;

    // Mirror into the transcript for console rendering (the console shows
    // the LATEST todo entry; older ones are history). Best-effort: a
    // session-manager hiccup must not fail the todo write.
    let session = deps.session().await;
    let turn_id = crate::state::get_turn(&deps.iii, &session_id, cfg.session_timeout_ms)
        .await
        .ok()
        .flatten()
        .map(|r| r.turn_id)
        .unwrap_or_else(|| "manual".to_string());
    let _ = session
        .append_custom(
            &session_id,
            "todo",
            value,
            &format!("e_todo_{}", crate::types::message::AgentMessage::now_ms()),
            Some(&json!({ "turn_id": turn_id })),
        )
        .await;

    Ok(TodoResponse {
        ok: true,
        count: req.items.len() as u32,
    })
}

fn validate(items: &[TodoItem]) -> Result<(), HarnessError> {
    if items.len() > MAX_ITEMS {
        return Err(HarnessError::InvalidRequest(format!(
            "todo list has {} items; the cap is {MAX_ITEMS} — collapse finished work",
            items.len()
        )));
    }
    for (i, item) in items.iter().enumerate() {
        if item.text.trim().is_empty() {
            return Err(HarnessError::InvalidRequest(format!(
                "todo item {i} is empty"
            )));
        }
        if item.text.chars().count() > MAX_ITEM_CHARS {
            return Err(HarnessError::InvalidRequest(format!(
                "todo item {i} exceeds {MAX_ITEM_CHARS} chars"
            )));
        }
    }
    let in_progress = items
        .iter()
        .filter(|i| i.status == TodoStatus::InProgress)
        .count();
    if in_progress > 1 {
        return Err(HarnessError::InvalidRequest(format!(
            "{in_progress} items are in_progress; keep exactly one active task"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(text: &str, status: TodoStatus) -> TodoItem {
        TodoItem {
            text: text.into(),
            status,
        }
    }

    #[test]
    fn accepts_a_reasonable_list() {
        let items = vec![
            item("read the failing test", TodoStatus::Completed),
            item("fix the bug", TodoStatus::InProgress),
            item("run the suite", TodoStatus::Pending),
        ];
        assert!(validate(&items).is_ok());
    }

    #[test]
    fn rejects_multiple_in_progress() {
        let items = vec![
            item("a", TodoStatus::InProgress),
            item("b", TodoStatus::InProgress),
        ];
        let err = validate(&items).unwrap_err().to_string();
        assert!(err.contains("exactly one"), "{err}");
    }

    #[test]
    fn rejects_oversized_and_empty_items() {
        assert!(validate(&[item("", TodoStatus::Pending)]).is_err());
        assert!(validate(&[item(&"x".repeat(501), TodoStatus::Pending)]).is_err());
        let too_many: Vec<TodoItem> = (0..51)
            .map(|i| item(&format!("t{i}"), TodoStatus::Pending))
            .collect();
        assert!(validate(&too_many).is_err());
    }

    #[test]
    fn empty_list_clears_without_error() {
        assert!(validate(&[]).is_ok());
    }

    #[test]
    fn todowrite_shaped_payload_deserializes() {
        // gpt-5.4 emits the TodoWrite field names it was trained on.
        let req: TodoRequest = serde_json::from_value(serde_json::json!({
            "todos": [
                { "content": "fix the bug", "status": "in_progress", "activeForm": "Fixing" }
            ]
        }))
        .unwrap();
        assert_eq!(req.items.len(), 1);
        assert_eq!(req.items[0].text, "fix the bug");
        assert_eq!(req.items[0].status, TodoStatus::InProgress);
    }
}
