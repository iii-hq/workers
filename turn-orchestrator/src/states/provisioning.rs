//! `provisioning` state handler. First state of every new chat. Builds the
//! system prompt by fetching each URI in `cfg.system_default_skills` and
//! handing the per-URI results to `system_prompt::build`. Soft-fail per URI.

use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;

use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::agent_call;
use crate::config::TurnOrchestratorConfig;
use crate::persistence;
use crate::state::{TurnState, TurnStateRecord};
use crate::system_prompt::{self, DefaultSkillBody};

/// Per-URI timeout for the chat-init fetch. Each call is independent —
/// failure of one URI never blocks the others.
const FETCH_TIMEOUT_MS: u64 = 10_000;

pub async fn handle(
    iii: &III,
    cfg: &Arc<TurnOrchestratorConfig>,
    record: &mut TurnStateRecord,
) -> anyhow::Result<()> {
    let request = persistence::load_run_request(iii, &record.session_id).await;

    let tools = json!([agent_call::agent_call_tool()]);
    persistence::save_function_schemas(iii, &record.session_id, tools.clone()).await;

    let override_prompt = request
        .get("system_prompt")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty());
    let cwd = request.get("cwd").and_then(Value::as_str);
    let cwd_path = cwd.map(Path::new);

    let fetched = fetch_default_skills(iii, &cfg.system_default_skills).await;
    let bodies = build_default_skill_bodies(&cfg.system_default_skills, &fetched);

    let prompt = system_prompt::build(&bodies, cwd_path, override_prompt);

    let mut updated = request.clone();
    if let Some(obj) = updated.as_object_mut() {
        obj.insert("system_prompt".into(), json!(prompt));
    }
    persistence::save_run_request(iii, &record.session_id, updated).await;

    record.transition_to(TurnState::AwaitingAssistant);
    Ok(())
}

/// Fetch each URI independently and return a map of `uri → body` for URIs
/// that fetched successfully. Missing entries become `body: None` in the
/// assembled prompt; failures are logged.
///
/// `directory::skills::fetch-skill` accepts a batched `{ uris: [...] }` form,
/// but it returns the bodies as one concatenated string with no per-URI
/// attribution, which would lose the per-URI failure granularity we rely on
/// for the recovery-stub mechanism. With `system_default_skills` typically
/// one or two entries, N sequential calls is acceptable; chat-init is not
/// a hot path.
async fn fetch_default_skills(iii: &III, uris: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::with_capacity(uris.len());
    for uri in uris {
        match fetch_uri(iii, uri).await {
            Some(body) => {
                out.insert(uri.clone(), body);
            }
            None => {
                tracing::warn!(
                    %uri,
                    "default skill fetch failed at chat-init; agent will see a recovery stub"
                );
            }
        }
    }
    out
}

/// Fetch a single `iii://` URI via `directory::skills::fetch-skill`.
/// Tolerates either a raw string response or `{ body: "..." }` envelope.
async fn fetch_uri(iii: &III, uri: &str) -> Option<String> {
    let resp = match iii
        .trigger(TriggerRequest {
            function_id: "directory::skills::fetch-skill".into(),
            payload: json!({ "uri": uri }),
            action: None,
            timeout_ms: Some(FETCH_TIMEOUT_MS),
        })
        .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(%uri, %err, "directory::skills::fetch-skill trigger failed");
            return None;
        }
    };
    response_to_string(&resp)
}

/// Zip configured URIs with the fetched body map, preserving config order.
/// URIs not in the map become `DefaultSkillBody { body: None }`.
fn build_default_skill_bodies(
    uris: &[String],
    fetched: &HashMap<String, String>,
) -> Vec<DefaultSkillBody> {
    uris.iter()
        .map(|uri| DefaultSkillBody {
            uri: uri.clone(),
            body: fetched.get(uri).cloned(),
        })
        .collect()
}

fn response_to_string(resp: &Value) -> Option<String> {
    if let Some(s) = resp.as_str() {
        return Some(s.to_string());
    }
    resp.get("body").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_default_skill_bodies_preserves_order_and_misses() {
        let uris = vec!["iii://iii".to_string(), "iii://shell".to_string()];
        let mut fetched: HashMap<String, String> = HashMap::new();
        fetched.insert("iii://iii".to_string(), "ALPHA".to_string());

        let out = build_default_skill_bodies(&uris, &fetched);
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].uri, "iii://iii");
        assert_eq!(out[0].body.as_deref(), Some("ALPHA"));
        assert_eq!(out[1].uri, "iii://shell");
        assert!(out[1].body.is_none());
    }

    #[test]
    fn build_default_skill_bodies_with_empty_uris_returns_empty() {
        let out = build_default_skill_bodies(&[], &HashMap::new());
        assert!(out.is_empty());
    }

    #[test]
    fn response_to_string_handles_string_and_envelope() {
        assert_eq!(
            response_to_string(&json!("hello")).as_deref(),
            Some("hello")
        );
        assert_eq!(
            response_to_string(&json!({"body": "world"})).as_deref(),
            Some("world")
        );
        assert!(response_to_string(&json!({"unrelated": 1})).is_none());
    }

    /// Smoke test: compose the two assembly halves (zip-with-fetched +
    /// system_prompt::build) so a regression in either half shows up here
    /// even if its dedicated unit test was deleted.
    #[test]
    fn assembled_prompt_contains_preamble_header_and_body() {
        let uris = vec!["iii://iii".to_string()];
        let mut fetched: HashMap<String, String> = HashMap::new();
        fetched.insert("iii://iii".to_string(), "THE BODY".to_string());

        let bodies = build_default_skill_bodies(&uris, &fetched);
        let prompt = crate::system_prompt::build(&bodies, None, None);

        assert!(prompt.contains("You are an iii agent worker."));
        assert!(prompt.contains("# iii://iii"));
        assert!(prompt.contains("THE BODY"));
    }

    /// Smoke test: a fully failed chat-init still produces a coherent
    /// prompt — the preamble survives and every URI gets a stub.
    #[test]
    fn assembled_prompt_with_all_fetches_failed_keeps_preamble_and_stubs() {
        let uris = vec!["iii://iii".to_string(), "iii://shell".to_string()];
        let fetched: HashMap<String, String> = HashMap::new();

        let bodies = build_default_skill_bodies(&uris, &fetched);
        let prompt = crate::system_prompt::build(&bodies, None, None);

        assert!(prompt.contains("You are an iii agent worker."));
        assert!(prompt.contains("# iii://iii"));
        assert!(prompt.contains("# iii://shell"));
        assert!(prompt.contains("skill body unavailable at chat start"));
    }
}
