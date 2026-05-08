//! `provisioning` state handler.

use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::agent_call;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};
use crate::system_prompt;

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;

    let tools = json!([agent_call::agent_call_tool()]);
    persistence::save_function_schemas(iii, &record.session_id, tools.clone()).await;

    let override_prompt = request
        .get("system_prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let cwd = request.get("cwd").and_then(Value::as_str);
    let skills_index = fetch_skills_bootstrap(iii).await;
    let prompt = system_prompt::build(skills_index.as_deref(), cwd, override_prompt);
    let mut updated = request.clone();
    if let Some(obj) = updated.as_object_mut() {
        obj.insert("system_prompt".into(), json!(prompt));
    }
    persistence::save_run_request(iii, &record.session_id, updated).await;

    // Sandbox provisioning is the LLM's responsibility, learned from a
    // skill (registered separately via the skills worker). The dispatcher
    // does not auto-provision; sessions that need a sandbox follow a
    // skill recipe to call sandbox::list / sandbox::create themselves.

    record.transition_to(TurnState::AwaitingAssistant);
    Ok(())
}

/// Best-effort bootstrap of the skills surface for the system prompt.
///
/// Concatenates:
///   1. the auto-rendered `iii://skills` index (links to every registered skill), and
///   2. the bodies of every **root-depth** registered skill (no `/` in the id),
///      batched in a single `skill::fetch` call.
///
/// The agent therefore boots with both the table of contents AND the
/// router-style bodies the agent normally reaches for first — eliminating
/// the round-trip to `skill::fetch iii://skills` on the first turn.
///
/// Any sub-step that fails returns `None` for that piece; the caller
/// degrades gracefully (the fallback section in `system_prompt::build`
/// covers a fully missing skills surface).
async fn fetch_skills_bootstrap(iii: &III) -> Option<String> {
    let index = fetch_uri(iii, "iii://skills").await;
    let root_uris = list_root_skill_uris(iii).await;
    let bodies = if root_uris.is_empty() {
        None
    } else {
        fetch_uris_batched(iii, &root_uris).await
    };

    match (index, bodies) {
        (Some(idx), Some(bod)) => Some(format!("{idx}\n\n---\n\n# Root skill bodies\n\n{bod}")),
        (Some(idx), None) => Some(idx),
        (None, Some(bod)) => Some(bod),
        (None, None) => None,
    }
}

/// Fetch a single `iii://` URI via `skill::fetch`. Tolerates either a raw
/// string response or `{ body: "..." }` envelope.
async fn fetch_uri(iii: &III, uri: &str) -> Option<String> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "skill::fetch".into(),
            payload: json!({ "uri": uri }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok()?;
    response_to_string(&resp)
}

/// Batch-fetch many URIs in one round trip. `skill::fetch` joins them with
/// `\n\n---\n\n` already, so the return is a single concatenated body.
async fn fetch_uris_batched(iii: &III, uris: &[String]) -> Option<String> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "skill::fetch".into(),
            payload: json!({ "uris": uris }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .ok()?;
    response_to_string(&resp)
}

/// List every registered skill id and keep only **root-depth** ones (no
/// `/` in the id). Empty list on any failure.
async fn list_root_skill_uris(iii: &III) -> Vec<String> {
    let Ok(resp) = iii
        .trigger(TriggerRequest {
            function_id: "skills::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
    else {
        return Vec::new();
    };
    let Some(arr) = resp.get("skills").and_then(Value::as_array) else {
        return Vec::new();
    };
    arr.iter()
        .filter_map(|entry| entry.get("id").and_then(Value::as_str))
        .filter(|id| is_root_skill_id(id))
        .map(|id| format!("iii://{id}"))
        .collect()
}

/// `iii` and `harness` are root; `resend/email`, `shell/bash` are not.
fn is_root_skill_id(id: &str) -> bool {
    !id.is_empty() && !id.contains('/')
}

fn response_to_string(resp: &Value) -> Option<String> {
    if let Some(s) = resp.as_str() {
        return Some(s.to_string());
    }
    resp.get("body").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::is_root_skill_id;

    #[test]
    fn root_ids_have_no_slash() {
        assert!(is_root_skill_id("iii"));
        assert!(is_root_skill_id("harness"));
        assert!(is_root_skill_id("shell-bash"));
    }

    #[test]
    fn nested_ids_are_not_root() {
        assert!(!is_root_skill_id("resend/email"));
        assert!(!is_root_skill_id("shell/bash/exec"));
    }

    #[test]
    fn empty_id_is_not_root() {
        assert!(!is_root_skill_id(""));
    }
}
