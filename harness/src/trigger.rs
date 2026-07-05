//! The function-trigger pipeline (harness.md § Functions / §
//! `harness::function::trigger`): the fail-closed allow/deny globs first,
//! then the target invocation, then result normalisation. Discovery results
//! (`engine::functions::list` / `info`) are post-filtered through the same
//! globs so the model only discovers what it can call.
//!
//! P1 covers the glob policy + target + normalisation. The `pre_trigger` /
//! `post_trigger` hook chain and `pending` deferral layer on in later phases.

use serde_json::{json, Value};

use crate::clients::EngineClient;
use crate::policy::CompiledPolicy;
use crate::types::content::ContentBlock;

/// A normalised function result ready to become a `function_result` entry.
#[derive(Debug, Clone)]
pub struct ResultData {
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub details: Value,
}

/// The outcome of triggering one call.
pub enum TriggerResult {
    /// A settled result (success, policy denial, or target error).
    Result(ResultData),
    /// Deferred — the result arrives later via `harness::function::resolve`.
    #[allow(dead_code)]
    Pending(PendingInfo),
}

#[allow(dead_code)]
#[derive(Debug, Clone, Default)]
pub struct PendingInfo {
    pub pending_timeout_ms: Option<u64>,
    pub held_by: Option<String>,
    pub child_session_id: Option<String>,
    pub child_turn_id: Option<String>,
    /// The child's isolated worktree (spawn `isolation: "worktree"`).
    pub worktree: Option<crate::types::turn::WorktreeRef>,
}

/// Run the trigger pipeline for one call. `function_id` is the unwrapped
/// target; `arguments` the target payload.
pub async fn trigger_call(
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
) -> TriggerResult {
    // Fail-closed glob policy — structural and final.
    if !policy.allows(function_id) {
        return TriggerResult::Result(denied_result(function_id));
    }

    TriggerResult::Result(invoke_target(engine, policy, function_id, arguments).await)
}

/// Invoke the target and normalise its result — WITHOUT the policy gate (the
/// caller checks `policy.allows` first) or the hook chain. `policy` is still
/// used to post-filter runtime discovery results. Used by the loop and the
/// hook-held release path after `pre_trigger` has run.
pub async fn invoke_target(
    engine: &EngineClient,
    policy: &CompiledPolicy,
    function_id: &str,
    arguments: &Value,
) -> ResultData {
    match engine.dispatch(function_id, arguments.clone()).await {
        Ok(mut value) => {
            if function_id == "engine::functions::list" || function_id == "engine::functions::info"
            {
                post_filter_discovery(&mut value, policy);
            }
            let (content, is_error) = normalize(&value);
            ResultData {
                content,
                is_error,
                details: value,
            }
        }
        Err(e) => {
            let message = e.message.clone();
            ResultData {
                content: vec![ContentBlock::text(message.clone())],
                is_error: true,
                details: json!({ "error": { "code": e.code, "message": message } }),
            }
        }
    }
}

/// The `is_error` result for a policy denial (no allow match or a deny match).
pub fn denied_result(function_id: &str) -> ResultData {
    let msg = format!(
        "function {function_id} is not permitted by this agent's dispatch policy (no allow-glob \
         match or a deny-glob match)"
    );
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": "policy_denied", "function_id": function_id, "message": msg }),
    }
}

/// Normalise an arbitrary function return into content blocks. `details`
/// always carries the raw value; content is a string render, an explicit
/// `content` block array, or a compact JSON fallback.
fn normalize(value: &Value) -> (Vec<ContentBlock>, bool) {
    let is_error = value
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);

    if let Value::String(s) = value {
        return (vec![ContentBlock::text(s.clone())], is_error);
    }
    if let Some(blocks) = value.get("content") {
        if let Ok(parsed) = serde_json::from_value::<Vec<ContentBlock>>(blocks.clone()) {
            if !parsed.is_empty() {
                return (parsed, is_error);
            }
        }
    }
    let rendered = serde_json::to_string(value).unwrap_or_else(|_| "<unserializable>".to_string());
    (vec![ContentBlock::text(rendered)], is_error)
}

/// Drop functions the agent cannot call from a discovery result so the model
/// only sees what its policy permits.
fn post_filter_discovery(value: &mut Value, policy: &CompiledPolicy) {
    let keep = |id: &str| policy.allows(id);
    if let Some(arr) = value.get_mut("functions").and_then(Value::as_array_mut) {
        arr.retain(|f| {
            f.get("function_id")
                .or_else(|| f.get("id"))
                .or_else(|| f.get("name"))
                .and_then(Value::as_str)
                .map(keep)
                .unwrap_or(false)
        });
    } else if let Some(id) = value
        .get("function_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
    {
        // engine::functions::info: blank the descriptor when not callable.
        if !keep(id) {
            *value = Value::Null;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::turn::FunctionPolicy;

    fn pol(allow: &[&str]) -> CompiledPolicy {
        CompiledPolicy::from(Some(&FunctionPolicy {
            allow: allow.iter().map(|s| s.to_string()).collect(),
            deny: vec![],
            expose: Default::default(),
        }))
    }

    #[test]
    fn normalize_string_value() {
        let (content, is_error) = normalize(&json!("hello"));
        assert!(!is_error);
        assert_eq!(content.len(), 1);
        assert!(matches!(&content[0], ContentBlock::Text { text } if text == "hello"));
    }

    #[test]
    fn normalize_uses_content_blocks_when_present() {
        let v = json!({ "content": [{ "type": "text", "text": "ok" }], "is_error": true });
        let (content, is_error) = normalize(&v);
        assert!(is_error);
        assert_eq!(content.len(), 1);
    }

    #[test]
    fn discovery_filter_drops_uncallable_functions() {
        let policy = pol(&["shell::*"]);
        let mut v = json!({ "functions": [
            { "function_id": "shell::run" },
            { "function_id": "fs::read" }
        ]});
        post_filter_discovery(&mut v, &policy);
        let ids: Vec<&str> = v["functions"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["function_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec!["shell::run"]);
    }
}
