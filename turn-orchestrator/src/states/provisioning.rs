//! `provisioning` state handler.

use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};

const SHELL_PREFIX: &str = "shell::";

pub fn requires_sandbox(tools: &Value) -> bool {
    tools
        .as_array()
        .map(|arr| {
            arr.iter().any(|t| {
                t.get("name")
                    .and_then(Value::as_str)
                    .map_or(false, |n| n.starts_with(SHELL_PREFIX))
            })
        })
        .unwrap_or(false)
}

fn sandbox_list_contains_id(resp: &Value, sandbox_id: &str) -> bool {
    let items = resp
        .as_array()
        .or_else(|| resp.get("items").and_then(Value::as_array))
        .or_else(|| resp.get("sandboxes").and_then(Value::as_array));

    items.is_some_and(|items| {
        items.iter().any(|item| {
            if item.get("stopped").and_then(Value::as_bool) == Some(true) {
                return false;
            }

            item.get("sandbox_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                == Some(sandbox_id)
        })
    })
}

async fn sandbox_alive(iii: &III, sandbox_id: &str) -> bool {
    let Ok(resp) = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
    else {
        return false;
    };

    sandbox_list_contains_id(&resp, sandbox_id)
}

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;
    let tools = request.get("tools").cloned().unwrap_or_else(|| json!([]));

    persistence::save_tool_schemas(iii, &record.session_id, tools.clone()).await;

    let sandbox_id = if requires_sandbox(&tools) {
        provision_sandbox(iii, &record.session_id, &request).await?
    } else {
        None
    };
    persistence::save_sandbox_id(iii, &record.session_id, sandbox_id.as_deref()).await;

    record.transition_to(TurnState::AwaitingAssistant);
    Ok(())
}

async fn provision_sandbox(
    iii: &III,
    session_id: &str,
    request: &Value,
) -> anyhow::Result<Option<String>> {
    if let Some(existing) = persistence::load_sandbox_id(iii, session_id).await {
        if sandbox_alive(iii, &existing).await {
            return Ok(Some(existing));
        }
        tracing::warn!(
            sandbox_id = %existing,
            session_id = %session_id,
            "stored sandbox id was not listed during resume; recreating"
        );
    }

    let image = request
        .get("image")
        .and_then(Value::as_str)
        .unwrap_or("python");
    let payload = json!({
        "image": image,
        "name": format!("session-{session_id}"),
        "idle_timeout_secs": request
            .get("idle_timeout_secs")
            .and_then(Value::as_u64)
            .unwrap_or(300),
    });
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::create".into(),
            payload,
            action: None,
            timeout_ms: Some(300_000),
        })
        .await
        .map_err(|e| anyhow::anyhow!("sandbox::create failed: {e}"))?;
    let sandbox_id = resp
        .get("sandbox_id")
        .and_then(Value::as_str)
        .map(str::to_string);
    Ok(sandbox_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requires_sandbox_when_any_shell_tool_present() {
        let tools = json!([{ "name": "read" }, { "name": "shell::bash::exec" }]);
        assert!(requires_sandbox(&tools));
    }

    #[test]
    fn does_not_require_sandbox_for_inline_tools() {
        let tools = json!([{ "name": "read" }, { "name": "write" }]);
        assert!(!requires_sandbox(&tools));
    }

    #[test]
    fn empty_tools_skips_sandbox() {
        assert!(!requires_sandbox(&json!([])));
        assert!(!requires_sandbox(&json!(null)));
    }

    #[test]
    fn sandbox_list_contains_id_accepts_bare_array() {
        let resp = json!([{ "sandbox_id": "s1" }, { "id": "s2" }]);
        assert!(sandbox_list_contains_id(&resp, "s1"));
        assert!(sandbox_list_contains_id(&resp, "s2"));
    }

    #[test]
    fn sandbox_list_contains_id_accepts_items_envelope() {
        let resp = json!({ "items": [{ "sandbox_id": "s1" }] });
        assert!(sandbox_list_contains_id(&resp, "s1"));
    }

    #[test]
    fn sandbox_list_contains_id_accepts_sandboxes_envelope() {
        let resp = json!({ "sandboxes": [{ "sandbox_id": "s1" }] });
        assert!(sandbox_list_contains_id(&resp, "s1"));
    }

    #[test]
    fn sandbox_list_contains_id_rejects_stopped_sandbox() {
        let resp = json!({ "sandboxes": [{ "sandbox_id": "s1", "stopped": true }] });
        assert!(!sandbox_list_contains_id(&resp, "s1"));
    }

    #[test]
    fn sandbox_list_contains_id_rejects_missing_id() {
        let resp = json!([{ "sandbox_id": "s1" }]);
        assert!(!sandbox_list_contains_id(&resp, "s3"));
    }
}
