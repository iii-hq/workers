//! `provisioning` state handler.

use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};
use crate::system_prompt;
use crate::tools_catalog;

pub async fn handle(iii: &III, record: &mut TurnStateRecord) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;

    let tools = json!([tools_catalog::agent_call_tool()]);
    persistence::save_tool_schemas(iii, &record.session_id, tools.clone()).await;

    let override_prompt = request
        .get("system_prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let cwd = request.get("cwd").and_then(Value::as_str);
    let skills_index = fetch_skills_index(iii).await;
    let prompt =
        system_prompt::build(skills_index.as_deref(), cwd, override_prompt);
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

/// Best-effort fetch of the `iii://skills` index. Returns `None` on any
/// failure — provisioning must not block on a skills worker that isn't
/// up yet. The fallback section in `system_prompt::build` covers this.
async fn fetch_skills_index(iii: &III) -> Option<String> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "skill::fetch".into(),
            payload: json!({ "uri": "iii://skills" }),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await
        .ok()?;
    if let Some(s) = resp.as_str() {
        return Some(s.to_string());
    }
    resp.get("body").and_then(Value::as_str).map(str::to_string)
}
