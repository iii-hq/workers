//! The function-trigger pipeline (harness.md § Functions / §
//! `harness::function::trigger`): the fail-closed allow/deny globs first,
//! then the target invocation, then result normalisation. Discovery results
//! (`engine::functions::list` / `info`) are post-filtered through the same
//! globs so the model only discovers what it can call.
//!
//! P1 covers the glob policy + target + normalisation. The `pre_trigger` /
//! `post_trigger` hook chain and `pending` deferral layer on in later phases.

use iii_sdk::IIIClient;
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

/// Route worker-to-worker calls through this worker's namespace while keeping
/// engine builtins in `default`.
pub(crate) fn route_namespace(iii: &IIIClient, function_id: &str) -> Option<String> {
    (!is_engine_builtin(function_id))
        .then(|| iii.namespace())
        .flatten()
}

fn is_engine_builtin(function_id: &str) -> bool {
    matches!(
        function_id.split("::").next(),
        Some(
            "state"
                | "stream"
                | "queue"
                | "pubsub"
                | "configuration"
                | "cron"
                | "http"
                | "engine"
                | "sandbox"
                | "log"
                | "secret"
                | "kv"
                | "iii"
        )
    )
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
    /// Hook holds only: the arguments as mutated by the chain up to the hold,
    /// checkpointed so a release executes the mutated call (issue #506).
    pub held_arguments: Option<Value>,
    pub child_session_id: Option<String>,
    pub child_turn_id: Option<String>,
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
            if function_id == "engine::functions::list" {
                post_filter_discovery(&mut value, policy);
            } else if function_id == "engine::functions::info" {
                post_filter_info(&mut value, policy);
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

/// Post-filter an `engine::functions::info` result — the flat single-id
/// detail (blanked to `null` when not callable, as ever) or the engine's
/// `function_ids` batch envelope. Batch entries the agent cannot dispatch are
/// masked to the same `{ function_id, error: "not available" }` stub the
/// engine uses for unknown ids — denied stays indistinguishable from
/// nonexistent, and (unlike the list filter) entries are never dropped, so
/// the model can pair every requested id with an outcome.
fn post_filter_info(value: &mut Value, policy: &CompiledPolicy) {
    let keep = |id: &str| policy.allows(id);
    if let Some(items) = value.get_mut("functions").and_then(Value::as_array_mut) {
        for item in items.iter_mut() {
            let id = item
                .get("function_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            if !keep(&id) {
                *item = json!({ "function_id": id, "error": "not available" });
            } else {
                overlay_control_contract(item, &id);
            }
        }
    } else if let Some(id) = value
        .get("function_id")
        .or_else(|| value.get("id"))
        .and_then(Value::as_str)
    {
        let id = id.to_string();
        if !keep(&id) {
            *value = Value::Null;
        } else {
            overlay_control_contract(value, &id);
        }
    }
}

/// `engine::register_trigger` / `engine::unregister_trigger` are intercepted
/// in-turn, so the engine's own registration (raw: `function_id` required, no
/// `once`/`lifecycle`/`conditions`) is NOT the contract an agent calls.
/// Discovery must describe the intercept, or an agent that reads
/// `functions::info` "learns" its tool schema is wrong.
fn overlay_control_contract(item: &mut Value, id: &str) {
    let Some((description, schema)) = crate::functions::subscribe::control_contract(id) else {
        return;
    };
    if let Some(map) = item.as_object_mut() {
        map.insert("description".into(), Value::String(description.into()));
        if map.contains_key("request_schema") {
            map.insert("request_schema".into(), schema);
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

/// The `is_error` result for an `agent_trigger` call with no resolvable
/// target — arguments were empty, null, or unparseable (local models emit
/// malformed JSON args). Dispatching the wrapper name to the engine would
/// only return a cryptic `function_not_found: agent_trigger`.
/// Char-safe ≤200-char preview of raw arguments for error messages —
/// model-emitted JSON serializes with literal UTF-8, and a byte-indexed
/// `String::truncate` panics mid-char on CJK/emoji payloads.
fn arguments_preview(arguments: &Value) -> String {
    let s = arguments.to_string();
    match s.char_indices().nth(200) {
        Some((i, _)) => s[..i].to_string(),
        None => s,
    }
}

pub fn wrapper_without_target_result(arguments: &Value) -> ResultData {
    let got = arguments_preview(arguments);
    let msg = format!(
        "agent_trigger was called without a usable target (arguments were {got}); expected \
         {{\"function\": \"<function id>\", \"payload\": {{...}}}}. Re-issue the call with the \
         target function id."
    );
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": "agent_trigger_no_target", "message": msg }),
    }
}

/// Provider-degraded arguments (a stream that died or hit max_tokens
/// mid-args, salvaged to a `"_partial": true` prefix or a raw `{"_raw": …}`
/// evidence object) must never execute: the salvage preserves evidence for
/// the transcript, not intent. Teachable local failure, mirroring
/// [`wrapper_without_target_result`].
pub fn truncated_arguments_result(function_id: &str, arguments: &Value) -> ResultData {
    let got = arguments_preview(arguments);
    let msg = format!(
        "the arguments for {function_id} arrived truncated (the model stream ended \
         mid-arguments; received {got}). The call was NOT executed — re-issue it with \
         complete arguments."
    );
    ResultData {
        content: vec![ContentBlock::text(msg.clone())],
        is_error: true,
        details: json!({ "error": "arguments_truncated", "message": msg }),
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

/// Drop functions the agent cannot call from an `engine::functions::list`
/// result so the model only sees what its policy permits (a list is a
/// filtered view — dropping is correct there; info results go through
/// [`post_filter_info`] instead, which masks rather than drops).
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
        for item in arr.iter_mut() {
            let id = item
                .get("function_id")
                .or_else(|| item.get("id"))
                .or_else(|| item.get("name"))
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            overlay_control_contract(item, &id);
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
    fn namespace_routing_keeps_builtins_in_default() {
        for function_id in [
            "state::get",
            "engine::functions::list",
            "queue::push",
            "configuration::get",
        ] {
            assert!(is_engine_builtin(function_id), "{function_id}");
        }
        for function_id in ["router::chat", "session::get", "approval::evaluate"] {
            assert!(!is_engine_builtin(function_id), "{function_id}");
        }
    }

    #[test]
    fn discovery_reports_the_intercepted_registration_contract() {
        // The raw engine row for register_trigger would claim `function_id`
        // is required and know nothing of once/lifecycle/conditions — the
        // overlay swaps in the intercept's contract.
        let policy = pol(&["*"]);
        let mut info = serde_json::json!({
            "function_id": "engine::register_trigger",
            "description": "Register a trigger that fires `function_id` directly",
            "request_schema": { "required": ["function_id", "trigger_type"] }
        });
        post_filter_info(&mut info, &policy);
        assert!(info["description"]
            .as_str()
            .unwrap()
            .contains("omit function_id"));
        let schema = serde_json::to_string(&info["request_schema"]).unwrap();
        assert!(
            schema.contains("lifecycle"),
            "intercept schema expected: {schema}"
        );

        let mut list = serde_json::json!({ "functions": [
            { "function_id": "engine::register_trigger", "description": "raw" },
            { "function_id": "state::get", "description": "Get a value from state" }
        ]});
        post_filter_discovery(&mut list, &policy);
        assert!(list["functions"][0]["description"]
            .as_str()
            .unwrap()
            .contains("omit function_id"));
        assert_eq!(
            list["functions"][1]["description"],
            "Get a value from state"
        );
    }

    #[test]
    fn arguments_preview_is_char_safe_on_multibyte_payloads() {
        // Byte 200 lands mid-emoji: the old byte-indexed String::truncate
        // panicked here on model-emitted CJK/emoji args.
        let args = json!({ "text": "🎉".repeat(100) });
        let preview = arguments_preview(&args);
        assert!(preview.chars().count() <= 200);
        assert!(args.to_string().starts_with(&preview));
        // Both teachable results render without panicking.
        assert!(wrapper_without_target_result(&args).is_error);
        assert!(truncated_arguments_result("state::set", &args).is_error);
        // Short args pass through whole.
        assert_eq!(arguments_preview(&json!({"a": 1})), r#"{"a":1}"#);
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

    #[test]
    fn info_filter_masks_denied_batch_entries_without_dropping() {
        let policy = pol(&["shell::*"]);
        let mut v = json!({ "functions": [
            { "function_id": "shell::run", "request_schema": { "type": "object" } },
            { "function_id": "fs::read", "request_schema": { "type": "object" } },
            { "function_id": "nope::missing", "error": "not_found" }
        ]});
        post_filter_info(&mut v, &policy);
        let items = v["functions"].as_array().unwrap();
        assert_eq!(items.len(), 3, "batch entries are masked, never dropped");
        assert!(items[0].get("request_schema").is_some());
        assert_eq!(
            items[1],
            json!({ "function_id": "fs::read", "error": "not available" })
        );
        // A not_found marker for an id outside the policy is masked uniformly.
        assert_eq!(
            items[2],
            json!({ "function_id": "nope::missing", "error": "not available" })
        );
    }

    #[test]
    fn info_filter_blanks_denied_single_detail() {
        let policy = pol(&["shell::*"]);
        let mut allowed = json!({ "function_id": "shell::run", "request_schema": {} });
        post_filter_info(&mut allowed, &policy);
        assert!(allowed.is_object());

        let mut denied = json!({ "function_id": "fs::read", "request_schema": {} });
        post_filter_info(&mut denied, &policy);
        assert!(denied.is_null());
    }
}
