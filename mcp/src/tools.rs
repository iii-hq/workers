//! `tools/list` and `tools/call` MCP methods.
//!
//! Direct port of `src/mcp/tools.ts`.

use std::sync::Arc;

use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Map, Value};

use crate::config::McpConfig;

/// Built-in iii engine function id used to enumerate registered functions.
pub const ENGINE_LIST_FUNCTIONS: &str = "engine::functions::list";

/// Maps every `::` to `__` for MCP tool names (multi-segment function ids).
pub fn encode_tool_name(function_id: &str) -> String {
    function_id.replace("::", "__")
}

/// Reverses [`encode_tool_name`]. Function ids that contain literal `__`
/// would round-trip incorrectly; iii ids do not.
pub fn decode_tool_name(tool_name: &str) -> String {
    tool_name.replace("__", "::")
}

pub fn is_hidden_function_id(cfg: &McpConfig, function_id: &str) -> bool {
    cfg.hidden_prefixes
        .iter()
        .any(|p| function_id.starts_with(p))
}

#[derive(Debug)]
pub struct McpInvalidParamsError(pub String);

impl std::fmt::Display for McpInvalidParamsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for McpInvalidParamsError {}

/// Decide whether a function should appear in `tools/list`. Mirrors the TS
/// `shouldAdvertise` predicate.
fn should_advertise(cfg: &McpConfig, fn_obj: &Map<String, Value>) -> bool {
    let function_id = match fn_obj.get("function_id").and_then(Value::as_str) {
        Some(id) => id,
        None => return false,
    };
    if is_hidden_function_id(cfg, function_id) {
        return false;
    }
    if !cfg.require_expose {
        return true;
    }
    let Some(meta) = fn_obj.get("metadata").and_then(Value::as_object) else {
        return false;
    };
    let Some(mcp_meta) = meta.get("mcp").and_then(Value::as_object) else {
        return false;
    };
    matches!(mcp_meta.get("expose"), Some(Value::Bool(true)))
}

/// Convert iii's `request_format` into the MCP `inputSchema` shape: drop
/// the `name`/`description` envelope keys, force `type: "object"`. Identical
/// to the TS `formatToInputSchema`.
fn format_to_input_schema(fmt: Option<&Value>) -> Value {
    let Some(obj) = fmt.and_then(Value::as_object) else {
        return json!({"type": "object", "properties": {}});
    };
    let mut out = Map::new();
    out.insert("type".to_string(), Value::String("object".to_string()));
    for (k, v) in obj {
        if k == "name" || k == "description" {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    Value::Object(out)
}

/// Some engines wrap the list under a `{ functions: [...] }` envelope; some
/// return the array directly. Match the TS `unwrapFunctionList` logic.
fn unwrap_function_list(raw: &Value) -> Vec<Map<String, Value>> {
    if let Some(arr) = raw.as_array() {
        return arr.iter().filter_map(|v| v.as_object().cloned()).collect();
    }
    if let Some(obj) = raw.as_object() {
        if let Some(arr) = obj.get("functions").and_then(Value::as_array) {
            return arr.iter().filter_map(|v| v.as_object().cloned()).collect();
        }
    }
    Vec::new()
}

pub async fn mcp_tools_list(iii: &Arc<III>, cfg: &Arc<McpConfig>) -> Result<Value, IIIError> {
    let raw = iii
        .trigger(TriggerRequest {
            function_id: ENGINE_LIST_FUNCTIONS.to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    let functions = unwrap_function_list(&raw);
    let tools: Vec<Value> = functions
        .into_iter()
        .filter(|f| should_advertise(cfg, f))
        .map(|f| {
            let function_id = f
                .get("function_id")
                .and_then(Value::as_str)
                .unwrap_or_default();
            let description = f
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or_default();
            json!({
                "name": encode_tool_name(function_id),
                "description": description,
                "inputSchema": format_to_input_schema(f.get("request_format")),
            })
        })
        .collect();
    Ok(json!({ "tools": tools }))
}

fn stringify_result(v: &Value) -> String {
    if let Some(s) = v.as_str() {
        return s.to_string();
    }
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

pub async fn mcp_tools_call(
    iii: &Arc<III>,
    cfg: &Arc<McpConfig>,
    params: &Value,
) -> Result<Value, McpToolsCallError> {
    let name = params
        .get("name")
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            McpToolsCallError::InvalidParams(McpInvalidParamsError(
                "tools/call requires a non-empty string name".into(),
            ))
        })?;
    let function_id = decode_tool_name(&name);
    if is_hidden_function_id(cfg, &function_id) {
        return Err(McpToolsCallError::InvalidParams(McpInvalidParamsError(
            "tool name resolves to a hidden iii function id".into(),
        )));
    }
    let arguments = match params.get("arguments") {
        Some(v) if v.is_object() => v.clone(),
        _ => json!({}),
    };
    match iii
        .trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload: arguments,
            action: None,
            timeout_ms: None,
        })
        .await
    {
        Ok(result) => {
            let mut out = Map::new();
            out.insert(
                "content".to_string(),
                json!([{ "type": "text", "text": stringify_result(&result) }]),
            );
            if let Some(obj) = result.as_object() {
                out.insert("structuredContent".to_string(), Value::Object(obj.clone()));
            }
            Ok(Value::Object(out))
        }
        Err(e) => {
            let msg = e.to_string();
            Ok(json!({
                "content": [{"type": "text", "text": msg}],
                "isError": true,
            }))
        }
    }
}

#[derive(Debug)]
pub enum McpToolsCallError {
    InvalidParams(McpInvalidParamsError),
}

impl std::fmt::Display for McpToolsCallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            McpToolsCallError::InvalidParams(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for McpToolsCallError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> Arc<McpConfig> {
        Arc::new(McpConfig::default())
    }

    #[test]
    fn encode_tool_name_replaces_double_colons() {
        assert_eq!(encode_tool_name("worker::echo"), "worker__echo");
        assert_eq!(
            encode_tool_name("a::b::c::d"),
            "a__b__c__d",
            "every :: pair must be replaced"
        );
    }

    #[test]
    fn decode_is_inverse_of_encode_for_iii_ids() {
        let id = "demo::nested::call";
        assert_eq!(decode_tool_name(&encode_tool_name(id)), id);
    }

    #[test]
    fn hidden_prefix_filter_uses_config() {
        let c = cfg();
        assert!(is_hidden_function_id(&c, "engine::list"));
        assert!(is_hidden_function_id(&c, "skills::register"));
        assert!(is_hidden_function_id(&c, "mcp::handler"));
        assert!(!is_hidden_function_id(&c, "resend::contacts::add"));
    }

    #[test]
    fn mcp_handler_id_is_auto_hidden() {
        let c = cfg();
        assert!(
            is_hidden_function_id(&c, "mcp::handler"),
            "the bridge's own function id must be excluded"
        );
    }

    #[test]
    fn format_to_input_schema_drops_envelope_keys() {
        let fmt = json!({
            "name": "echo-input",
            "description": "hidden",
            "properties": {"name": {"type": "string"}},
            "required": ["name"]
        });
        let schema = format_to_input_schema(Some(&fmt));
        assert_eq!(schema["type"], "object");
        assert!(schema.get("name").is_none());
        assert!(schema.get("description").is_none());
        assert_eq!(schema["required"], json!(["name"]));
        assert_eq!(schema["properties"]["name"]["type"], "string");
    }

    #[test]
    fn format_to_input_schema_handles_missing() {
        let schema = format_to_input_schema(None);
        assert_eq!(schema["type"], "object");
        assert_eq!(schema["properties"], json!({}));
    }

    #[test]
    fn unwrap_function_list_handles_both_shapes() {
        let raw = json!([{"function_id": "a"}, {"function_id": "b"}]);
        assert_eq!(unwrap_function_list(&raw).len(), 2);
        let raw = json!({"functions": [{"function_id": "a"}]});
        assert_eq!(unwrap_function_list(&raw).len(), 1);
        assert!(unwrap_function_list(&json!(null)).is_empty());
        assert!(unwrap_function_list(&json!({"unrelated": 1})).is_empty());
    }

    #[test]
    fn should_advertise_respects_require_expose() {
        let mut c = (*cfg()).clone();
        c.require_expose = true;
        let fn_visible = json!({
            "function_id": "demo::echo",
            "metadata": {"mcp": {"expose": true}}
        });
        let fn_hidden = json!({"function_id": "demo::echo"});
        assert!(should_advertise(&c, fn_visible.as_object().unwrap()));
        assert!(!should_advertise(&c, fn_hidden.as_object().unwrap()));
    }

    #[test]
    fn should_advertise_filters_hidden_prefixes_even_without_require_expose() {
        let c = cfg();
        let f = json!({"function_id": "engine::functions::list"});
        assert!(!should_advertise(&c, f.as_object().unwrap()));
    }

    #[test]
    fn stringify_passthrough_for_strings() {
        assert_eq!(stringify_result(&json!("hello")), "hello");
    }

    #[test]
    fn stringify_pretty_prints_objects() {
        let s = stringify_result(&json!({"a": 1}));
        assert!(s.contains("\"a\": 1"));
    }
}
