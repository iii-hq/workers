//! `agent::call` — single LLM-facing dispatcher.
//!
//! The model emits exactly one tool name, `agent_call`, with `{function,
//! payload}` arguments. This module owns the LLM-facing tool descriptor
//! (`agent_call_tool`), the shared dispatch helper (`dispatch`) that
//! `states::tools::handle_execute` calls into, and the iii function
//! registration (`register`) that surfaces the dispatcher on the bus.

use std::sync::Arc;

use harness_types::{ContentBlock, TextContent, ToolResult};
use iii_sdk::{RegisterFunctionMessage, TriggerRequest, Value, III};
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
        "description": "Call any iii function on the bus. The argument `function` is the function id (use `::` separators, e.g. `shell::filesystem::ls`). The argument `payload` is the function-specific JSON arguments. Skills loaded into your context tell you which functions exist and what arguments they take. The result is whatever that function returns.",
        "parameters": {
            "type": "object",
            "properties": {
                "function": {
                    "type": "string",
                    "description": "iii function id to dispatch, e.g. 'shell::filesystem::ls'."
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

/// Build a `ToolResult` carrying a structured error envelope. The agent
/// loop must continue regardless of failure class — never throw from the
/// dispatcher.
fn error_result(envelope: Value) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: envelope.to_string(),
        })],
        details: envelope,
        terminate: false,
    }
}

/// Validate the `function` field. Returns `Err(ToolResult)` when the field
/// is missing, empty, or not a string. The caller short-circuits without
/// touching `iii.trigger`.
fn validate_function_field(function: &Value) -> Result<String, ToolResult> {
    match function.as_str() {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(error_result(json!({
            "error": "missing_function",
            "message": "agent_call requires a non-empty `function` string field"
        }))),
    }
}

#[must_use]
fn is_function_not_found<E: std::fmt::Display>(err: &E) -> bool {
    let msg = err.to_string();
    msg.contains("function_not_found") || msg.contains("Function not found")
}

#[must_use]
pub(crate) fn is_timeout<E: std::fmt::Display>(err: &E) -> bool {
    let s = err.to_string().to_ascii_lowercase();
    s.contains("timeout") || s.contains("timed out")
}

/// If the inner function returned a `ToolResult`-shaped value, deserialize
/// it. Otherwise wrap the value as the tool's `details` so function-level
/// envelopes (`{ok: false, error}`) pass through verbatim per the spec.
fn decode_or_passthrough(value: Value) -> ToolResult {
    if let Ok(tr) = serde_json::from_value::<ToolResult>(value.clone()) {
        return tr;
    }
    ToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: value.to_string(),
        })],
        details: value,
        terminate: false,
    }
}

/// Shared dispatch helper. Both `agent::call`'s registered iii handler and
/// `states::tools::handle_execute` call this so policy → trigger → error
/// mapping has one source of truth.
///
/// Tier 2: no schema lookup, no payload validation, no sandbox automation.
/// Skills (registered separately via the skills worker) teach the LLM iii
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
) -> ToolResult {
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
            "hint": "load the relevant skill via skill::fetch, or check the function id"
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
            "LLM-facing dispatcher: dispatches an iii function and returns a ToolResult."
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
                let inner_payload = payload
                    .get("payload")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
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
        let result = validate_function_field(&json!("shell::filesystem::ls")).unwrap();
        assert_eq!(result, "shell::filesystem::ls");
    }

    #[test]
    fn decode_or_passthrough_preserves_function_error_envelopes() {
        let inner = json!({ "ok": false, "error": "thing went wrong", "code": 42 });
        let tr = decode_or_passthrough(inner.clone());
        assert_eq!(tr.details, inner);
        assert!(!tr.terminate);
    }

    #[test]
    fn is_timeout_recognizes_engine_timeout_strings() {
        struct E(&'static str);
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        assert!(is_timeout(&E("trigger timed out after 240s")));
        assert!(is_timeout(&E("Timeout waiting for reply")));
        assert!(!is_timeout(&E("function_not_found")));
    }

    #[test]
    fn function_not_found_detector_matches_engine_error_codes() {
        struct E(&'static str);
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        assert!(is_function_not_found(&E(
            "remote error (function_not_found): Function sandbox::create not found"
        )));
        assert!(is_function_not_found(&E("Function not found")));
        assert!(!is_function_not_found(&E("timeout waiting for reply")));
        assert!(!is_function_not_found(&E("invocation_failed: bad payload")));
    }
}
