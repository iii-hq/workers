//! `agent::call` — single LLM-facing dispatcher.
//!
//! The model emits exactly one tool name, `agent_call`, with `{function,
//! payload}` arguments. This module owns the LLM-facing tool descriptor
//! (`agent_call_tool`), the shared dispatch helper (`dispatch`) that
//! `states::functions::handle_execute` calls into, and the iii function
//! registration (`register`) that surfaces the dispatcher on the bus.

use std::sync::Arc;

use harness_types::{ContentBlock, FunctionResult, TextContent};
use iii_sdk::{IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

/// LLM-facing tool name (regex-safe, no `::`).
pub const TOOL_NAME: &str = "agent_call";

/// iii function id under which the dispatcher is registered on the bus.
pub const FUNCTION_ID: &str = "agent::call";

/// Default per-call dispatch timeout. None lets iii-sdk pick its default;
/// override per call site when a tighter ceiling is wanted.
pub(crate) const DEFAULT_DISPATCH_TIMEOUT_MS: Option<u64> = None;

/// The single tool schema sent to the provider.
///
/// Snapshot-tested below — any change here is a wire-format change for every
/// active session and must be intentional.
pub fn agent_call_tool() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Call any iii function on the bus. The argument `function` is the function id (use `::` separators, e.g. `shell::fs::ls`). The argument `payload` is the function-specific JSON arguments. Skills loaded into your context tell you which functions exist and what arguments they take. The result is whatever that function returns.",
        "parameters": {
            "type": "object",
            "properties": {
                "function": {
                    "type": "string",
                    "description": "iii function id to dispatch, e.g. 'shell::fs::ls'."
                },
                "payload": {
                    "type": "object",
                    "description": "Arguments forwarded to the function. Shape depends on the target function."
                }
            },
            "required": ["function"]
        },
        "label": "agent_call",
        "execution_mode": "parallel",
        "prepare_arguments_supported": false,
    })
}

/// Build a `FunctionResult` carrying a structured error envelope. The agent
/// loop must continue regardless of failure class — never throw from the
/// dispatcher.
fn error_result(envelope: Value) -> FunctionResult {
    FunctionResult {
        content: vec![ContentBlock::Text(TextContent {
            text: envelope.to_string(),
        })],
        details: envelope,
        terminate: false,
    }
}

/// Validate the `function` field. Returns `Err(FunctionResult)` when the field
/// is missing, empty, or not a string. The caller short-circuits without
/// touching `iii.trigger`.
fn validate_function_field(function: &Value) -> Result<String, FunctionResult> {
    match function.as_str() {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(error_result(json!({
            "error": "missing_function",
            "message": "agent_call requires a non-empty `function` string field"
        }))),
    }
}

/// True only for the engine's canonical "no such function" remote error.
/// Matches on `IIIError::Remote` with the exact `function_not_found` code
/// the engine emits in `iii.rs:1701`. Substring matching on `Display` was
/// previously used; that misclassified inner errors whose message text
/// happened to contain the magic substring.
#[must_use]
fn is_function_not_found(err: &IIIError) -> bool {
    matches!(err, IIIError::Remote { code, .. } if code == "function_not_found")
}

/// True only for the SDK's structured `Timeout` variant (see
/// `iii.rs:1155`). Substring matching on `Display` was previously used and
/// misclassified inner errors mentioning "timeout" / "timed out".
#[must_use]
pub(crate) fn is_timeout(err: &IIIError) -> bool {
    matches!(err, IIIError::Timeout)
}

/// If the inner function returned a `FunctionResult`-shaped value, deserialize
/// it. Otherwise wrap the value as the tool's `details` so function-level
/// envelopes (`{ok: false, error}`) pass through verbatim per the spec.
///
/// For a JSON `String` value (any function returning raw text), use the
/// inner string content as `text`. `serde_json::Value::to_string()` emits
/// the JSON-encoded form — surrounding quotes and `\n` literals — which
/// the harness web's `<pre>` then renders verbatim and looks like "raw
/// JSON in chat" (turn-orchestrator/agent_call.rs regression).
pub(crate) fn decode_or_passthrough(value: Value) -> FunctionResult {
    if let Ok(tr) = serde_json::from_value::<FunctionResult>(value.clone()) {
        return tr;
    }
    let text = match &value {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    FunctionResult {
        content: vec![ContentBlock::Text(TextContent { text })],
        details: value,
        terminate: false,
    }
}

/// Shared dispatch helper. Both `agent::call`'s registered iii handler and
/// `states::functions::handle_execute` call this so policy → trigger → error
/// mapping has one source of truth.
///
/// Tier 2: no schema lookup, no payload validation, no sandbox automation.
/// Skills (served by the iii-directory worker) teach the LLM iii
/// contracts — registry introspection, sandbox lifecycle, etc. The
/// dispatcher only does what skills can't: validate the `function` field,
/// dispatch via `iii.trigger`, and map errors back to envelopes the model
/// can read.
///
/// `_session_id` kept in the signature for caller symmetry; not consumed.
pub async fn dispatch(
    iii: &III,
    _session_id: &str,
    function: &Value,
    payload: Value,
) -> FunctionResult {
    let function_id = match validate_function_field(function) {
        Ok(id) => id,
        Err(result) => return result,
    };

    let response = iii
        .trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload,
            action: None,
            timeout_ms: DEFAULT_DISPATCH_TIMEOUT_MS,
        })
        .await;

    match response {
        Ok(value) => decode_or_passthrough(value),
        Err(ref e) if is_function_not_found(e) => error_result(json!({
            "error": "function_not_found",
            "function": function_id,
            "hint": "load the relevant skill via directory::skills::get, or check the function id"
        })),
        Err(ref e) if is_timeout(e) => error_result(json!({
            "error": "timeout",
            "function": function_id,
            "message": e.to_string()
        })),
        Err(e) => error_result(json!({
            "error": "trigger_failed",
            "function": function_id,
            "message": e.to_string()
        })),
    }
}

/// Register `agent::call` as a regular iii function so it appears in
/// `engine::functions::list` and can be invoked through `bridge::trigger`
/// for testing. The browser does not call this directly — `bridge::trigger`
/// remains the browser path. The LLM reaches it via the `agent_call` tool.
pub fn register(iii: &Arc<III>) {
    let iii_clone = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string()).with_description(
            "LLM-facing dispatcher: dispatches an iii function and returns a FunctionResult."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_clone.clone();
            async move {
                let session_id = payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let function = payload.get("function").cloned().unwrap_or(Value::Null);
                let inner_payload = payload.get("payload").cloned().unwrap_or_else(|| json!({}));
                let result = dispatch(&iii, &session_id, &function, inner_payload).await;
                serde_json::to_value(&result).map_err(|e| iii_sdk::IIIError::Handler(e.to_string()))
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_call_tool_schema_is_stable() {
        let tool = agent_call_tool();
        assert_eq!(tool["name"], "agent_call");
        assert_eq!(tool["parameters"]["type"], "object");
        assert_eq!(tool["parameters"]["required"], json!(["function"]));
        assert_eq!(
            tool["parameters"]["properties"]["function"]["type"],
            "string"
        );
        assert_eq!(
            tool["parameters"]["properties"]["payload"]["type"],
            "object"
        );
        let desc = tool["description"].as_str().unwrap();
        assert!(desc.contains("::"), "description should mention `::`");
        assert!(
            desc.contains("function id"),
            "description should explain what `function` is"
        );
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_function_returns_missing_function_error() {
        let result = validate_function_field(&Value::Null).unwrap_err();
        let env = &result.details;
        assert_eq!(env["error"], "missing_function");
    }

    #[test]
    fn empty_function_returns_missing_function_error() {
        let result = validate_function_field(&json!("")).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn non_string_function_returns_missing_function_error() {
        let result = validate_function_field(&json!(42)).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn valid_function_returns_owned_string() {
        let result = validate_function_field(&json!("shell::fs::ls")).unwrap();
        assert_eq!(result, "shell::fs::ls");
    }

    #[test]
    fn decode_or_passthrough_preserves_function_error_envelopes() {
        let inner = json!({ "ok": false, "error": "thing went wrong", "code": 42 });
        let tr = decode_or_passthrough(inner.clone());
        assert_eq!(tr.details, inner);
        assert!(!tr.terminate);
    }

    /// Regression: any function returning a JSON String (e.g. a skill body
    /// from a custom worker) must produce a content block whose `text`
    /// is the inner string content (real newlines), not the JSON-encoded
    /// form (literal `\n` + surrounding quotes). The latter renders as
    /// "raw JSON in chat" because the harness web wraps `text` in a
    /// `<pre>` verbatim.
    #[test]
    fn decode_or_passthrough_unwraps_string_value_into_text() {
        let body = "# shell/fs_read\n\nObserve-only filesystem ops.";
        let tr = decode_or_passthrough(Value::String(body.to_string()));
        let harness_types::ContentBlock::Text(text_block) = &tr.content[0] else {
            panic!("expected a Text content block, got {:?}", tr.content[0]);
        };
        assert_eq!(
            text_block.text, body,
            "string value must be unwrapped, not JSON-stringified"
        );
        assert!(
            !text_block.text.starts_with('"'),
            "text must not be wrapped in JSON quotes"
        );
        assert!(
            !text_block.text.contains("\\n"),
            "text must contain real newlines, not literal \\n"
        );
    }

    fn remote_err(code: &str, message: &str) -> IIIError {
        IIIError::Remote {
            code: code.to_string(),
            message: message.to_string(),
            stacktrace: None,
        }
    }

    #[test]
    fn is_timeout_recognizes_structured_timeout_variant() {
        assert!(is_timeout(&IIIError::Timeout));
        assert!(!is_timeout(&remote_err("function_not_found", "")));
        assert!(!is_timeout(&IIIError::Runtime("timed out".into())));
    }

    #[test]
    fn function_not_found_detector_matches_engine_error_code() {
        assert!(is_function_not_found(&remote_err(
            "function_not_found",
            "Function sandbox::create not found"
        )));
        assert!(!is_function_not_found(&IIIError::Timeout));
        assert!(!is_function_not_found(&remote_err(
            "invocation_failed",
            "bad payload"
        )));
    }

    // ── Adversarial unit tests added per plan
    // /Users/ytallolayon/.claude/plans/let-s-implement-more-tests-refactored-flask.md

    #[test]
    fn validate_function_field_rejects_object_value() {
        let result = validate_function_field(&json!({"nested": "x"})).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn validate_function_field_rejects_array_value() {
        let result = validate_function_field(&json!([1, 2, 3])).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn validate_function_field_rejects_float_number_value() {
        // The existing `non_string_function_returns_missing_function_error`
        // covers i64. This adds f64 coverage; both must take the
        // not-a-string branch.
        let result = validate_function_field(&json!(42.5)).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn decode_or_passthrough_handles_array_value() {
        let inner = json!([1, 2, 3]);
        let tr = decode_or_passthrough(inner.clone());
        assert_eq!(tr.details, inner);
        assert!(!tr.terminate);
        // The Text fallback stringifies the JSON.
        match tr.content.first() {
            Some(harness_types::ContentBlock::Text(t)) => {
                assert_eq!(t.text, "[1,2,3]");
            }
            other => panic!("expected Text content block, got {other:?}"),
        }
    }

    #[test]
    fn decode_or_passthrough_handles_primitive_value() {
        let inner = json!("just a string");
        let tr = decode_or_passthrough(inner.clone());
        assert_eq!(tr.details, inner);
        match tr.content.first() {
            Some(harness_types::ContentBlock::Text(t)) => {
                // JSON String values are unwrapped to their inner content
                // (see decode_or_passthrough_unwraps_string_value_into_text).
                assert_eq!(t.text, "just a string");
            }
            other => panic!("expected Text content block, got {other:?}"),
        }
    }

    #[test]
    fn decode_or_passthrough_handles_partial_tool_result() {
        // `terminate` has #[serde(default)]; partial input still
        // deserializes as a FunctionResult. If a future change drops the
        // default, this test fails and forces an explicit decision.
        let inner = json!({"content": [], "details": {"k": "v"}});
        let tr = decode_or_passthrough(inner.clone());
        assert!(tr.content.is_empty());
        assert_eq!(tr.details, json!({"k": "v"}));
        assert!(!tr.terminate);
    }

    #[test]
    fn tool_name_and_function_id_constants_are_stable() {
        assert_eq!(TOOL_NAME, "agent_call");
        assert_eq!(FUNCTION_ID, "agent::call");
    }

    /// Inner-function errors whose payload mentions "function_not_found"
    /// must not be reclassified as the dispatcher's function_not_found
    /// envelope. Only the engine's canonical Remote{code} signals it.
    #[test]
    fn is_function_not_found_ignores_substring_in_other_variants() {
        assert!(!is_function_not_found(&IIIError::Handler(
            "tool wrote log line: 'function_not_found in user data'".into()
        )));
        assert!(!is_function_not_found(&IIIError::Runtime(
            "function_not_found in user data".into()
        )));
        assert!(!is_function_not_found(&remote_err(
            "invocation_failed",
            "function_not_found mentioned in inner message"
        )));
    }

    /// Inner-function errors whose payload mentions "timeout" / "timed out"
    /// must not be reclassified as a bus timeout. Only IIIError::Timeout
    /// signals the SDK timeout path.
    #[test]
    fn is_timeout_ignores_substring_in_other_variants() {
        assert!(!is_timeout(&IIIError::Handler(
            "user input: scheduled timeout in 30 days from now".into()
        )));
        assert!(!is_timeout(&IIIError::Runtime("timed out parsing".into())));
        assert!(!is_timeout(&remote_err(
            "invocation_failed",
            "timed out reading file"
        )));
    }
}
