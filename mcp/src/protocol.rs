//! Pure helpers shared by the MCP dispatcher. No IO, no async — just
//! wire-format types, name mapping, and the hard-floor matcher.
//!
//! Kept side-effect-free so unit tests in `cargo test --lib` cover every
//! branch without needing an iii engine.

use iii_sdk::FunctionInfo;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// MCP spec revision this worker advertises in `initialize`. Must be a
/// real date-stamped version from <https://spec.modelcontextprotocol.io>;
/// MCP Inspector rejects future / unrecognised versions outright.
pub const MCP_PROTOCOL_VERSION: &str = "2025-06-18";

// JSON-RPC 2.0 error codes used across the dispatcher.
pub const PARSE_ERROR: i32 = -32700;
pub const INVALID_REQUEST: i32 = -32600;
pub const METHOD_NOT_FOUND: i32 = -32601;
pub const INVALID_PARAMS: i32 = -32602;
pub const INTERNAL_ERROR: i32 = -32603;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcResponse {
    pub jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<JsonRpcError>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonRpcError {
    pub code: i32,
    pub message: String,
}

impl JsonRpcResponse {
    pub fn success(id: Option<Value>, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    pub fn error(id: Option<Value>, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

/// Function-id prefixes that are NEVER advertised in `tools/list` and
/// rejected at `tools/call`. Mirrors the hard floor enforced by `skills`
/// (see `skills/src/functions/skills.rs::ALWAYS_HIDDEN_PREFIXES`).
/// Duplicated here so this crate doesn't depend on the skills crate;
/// keep the two lists in sync when adding an infra namespace.
pub const ALWAYS_HIDDEN_PREFIXES: &[&str] = &[
    "engine::",
    "state::",
    "stream::",
    "iii.",
    "iii::",
    "mcp::",
    "a2a::",
    "skills::",
    "prompts::",
];

/// True if `function_id` starts with any of the always-hidden prefixes
/// or any operator-supplied `extra` prefix from `config.yaml`.
pub fn is_hidden(function_id: &str, extra: &[String]) -> bool {
    ALWAYS_HIDDEN_PREFIXES
        .iter()
        .any(|p| function_id.starts_with(p))
        || extra.iter().any(|p| function_id.starts_with(p))
}

/// True if a function declares `metadata.mcp.expose == true`. Workers
/// like agentmemory tag every typed agent-callable handler this way and
/// intentionally omit the flag from HTTP wrappers, sub-skill handlers,
/// and prompt handlers. Honored by `tools/list` only when
/// `McpConfig.require_expose` is set; off by default for backwards
/// compatibility with workers that haven't adopted the flag yet.
pub fn is_mcp_exposed(f: &FunctionInfo) -> bool {
    f.metadata
        .as_ref()
        .and_then(|m| m.get("mcp.expose"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// MCP requires tool names to match `^[a-zA-Z0-9_-]+$`. iii function ids
/// use `::` as the namespace delimiter, so we map `::` → `__` on the way
/// out and `__` → `::` on the way back in.
pub fn function_id_to_tool_name(function_id: &str) -> String {
    function_id.replace("::", "__")
}

pub fn tool_name_to_function_id(tool_name: &str) -> String {
    tool_name.replace("__", "::")
}

/// Build the MCP tool object for a single iii function. The `metadata`
/// field on `FunctionInfo` is currently unused here — the v0.1 surface
/// doesn't expose `tool annotations` from MCP 2025-06-18; if you add
/// them, pull `metadata.mcp.{title,read_only_hint,...}` out and emit a
/// camelCase `annotations` field alongside `description`.
pub fn function_to_tool(f: &FunctionInfo) -> Value {
    let input = f
        .request_format
        .clone()
        .unwrap_or_else(|| json!({ "type": "object", "properties": {} }));
    let mut tool = json!({
        "name": function_id_to_tool_name(&f.function_id),
        "inputSchema": sanitize_schema(input),
    });
    if let Some(desc) = &f.description {
        tool["description"] = Value::String(desc.clone());
    }
    if let Some(out) = &f.response_format {
        tool["outputSchema"] = sanitize_schema(out.clone());
    }
    tool
}

/// Normalize a JSON Schema for MCP wire compatibility.
///
/// Cursor's MCP client (and other zod-based clients) reject two shapes
/// that schemars happily emits for `serde_json::Value` fields and raw
/// `Value` inputs/outputs:
///
/// - A literal boolean property value (`{ "properties": { "x": true } }`).
///   Draft-07 considers `true` to mean "any value", but strict validators
///   refuse to accept a boolean where they expect an object schema. We
///   replace `true`/`false` with `{}`, which is the equivalent
///   "any value" shape that every validator accepts.
/// - A top-level schema with no `type` field (e.g. the `AnyValue`
///   placeholder schemars emits for `serde_json::Value`). We inject
///   `"type": "object"` so the wire envelope is well-formed; struct-derived
///   schemas already have this and are left untouched.
///
/// Only the top schema and its direct `properties.*` are walked; deeper
/// traversal is unnecessary because schemars only emits literal `true`
/// at the property-value position.
pub fn sanitize_schema(mut schema: Value) -> Value {
    if let Value::Object(map) = &mut schema {
        if !map.contains_key("type") {
            map.insert("type".into(), Value::String("object".into()));
        }
        if let Some(Value::Object(props)) = map.get_mut("properties") {
            for (_, prop) in props.iter_mut() {
                if matches!(prop, Value::Bool(_)) {
                    *prop = json!({});
                }
            }
        }
    }
    schema
}

/// Wrap an arbitrary JSON value as an MCP tool result. Strings come
/// back as `text/plain`; everything else is pretty-printed JSON.
pub fn tool_text(value: &Value) -> Value {
    let text = match value {
        Value::String(s) => s.clone(),
        other => serde_json::to_string_pretty(other).unwrap_or_else(|_| other.to_string()),
    };
    json!({
        "content": [{ "type": "text", "text": text }],
        "isError": false
    })
}

/// Tool result with `isError: true`. Clients distinguish from a
/// JSON-RPC error: a tool error means "the call reached the tool, the
/// tool said no", while a JSON-RPC error means "the call never reached
/// the tool".
pub fn tool_error(message: &str) -> Value {
    json!({
        "content": [{ "type": "text", "text": message }],
        "isError": true
    })
}

/// Body of `initialize`'s `result`. v0.1 advertises only the surfaces
/// this worker actually implements — no `subscribe`, no `listChanged`,
/// no `logging`, no `completions`. Adding any of those means wiring
/// them in `handler.rs` first.
pub fn initialize_result() -> Value {
    json!({
        "protocolVersion": MCP_PROTOCOL_VERSION,
        "capabilities": {
            "tools": {},
            "resources": {},
            "prompts": {}
        },
        "serverInfo": {
            "name": "mcp",
            "version": env!("CARGO_PKG_VERSION")
        },
        "instructions": "iii engine MCP bridge. Tools are iii functions; resources and prompts come from the skills worker. Install `iii worker add skills` to populate the resources/prompts surface."
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_name_round_trip() {
        assert_eq!(
            function_id_to_tool_name("brain::summarize"),
            "brain__summarize"
        );
        assert_eq!(
            tool_name_to_function_id("brain__summarize"),
            "brain::summarize"
        );
        let id = "my-worker::deep::nested";
        assert_eq!(tool_name_to_function_id(&function_id_to_tool_name(id)), id);
    }

    #[test]
    fn hidden_matcher_blocks_every_always_hidden_prefix() {
        let extra: Vec<String> = vec![];
        assert!(is_hidden("engine::list", &extra));
        assert!(is_hidden("state::get", &extra));
        assert!(is_hidden("stream::publish", &extra));
        assert!(is_hidden("iii.on_foo", &extra));
        assert!(is_hidden("iii::internal", &extra));
        assert!(is_hidden("mcp::handler", &extra));
        assert!(is_hidden("a2a::send", &extra));
        assert!(is_hidden("skills::register", &extra));
        assert!(is_hidden("prompts::register", &extra));
    }

    #[test]
    fn hidden_matcher_allows_user_namespaces() {
        let extra: Vec<String> = vec![];
        assert!(!is_hidden("brain::summarize", &extra));
        assert!(!is_hidden("mem::observe", &extra));
        assert!(!is_hidden("my-worker::echo", &extra));
    }

    #[test]
    fn hidden_matcher_honours_extra_prefixes() {
        let extra = vec!["secret::".to_string(), "internal::".to_string()];
        assert!(is_hidden("secret::api-key", &extra));
        assert!(is_hidden("internal::ping", &extra));
        assert!(!is_hidden("public::call", &extra));
    }

    fn fn_info(id: &str, metadata: Option<Value>) -> FunctionInfo {
        FunctionInfo {
            function_id: id.into(),
            description: None,
            request_format: None,
            response_format: None,
            metadata,
        }
    }

    #[test]
    fn is_mcp_exposed_true_when_metadata_flag_set() {
        let f = fn_info("mem::observe", Some(json!({ "mcp.expose": true })));
        assert!(is_mcp_exposed(&f));
    }

    #[test]
    fn is_mcp_exposed_false_when_flag_missing() {
        let f = fn_info("api::post::agentmemory/observe", Some(json!({ "a2a.expose": true })));
        assert!(!is_mcp_exposed(&f));
    }

    #[test]
    fn is_mcp_exposed_false_when_metadata_absent() {
        let f = fn_info("fts::index::create", None);
        assert!(!is_mcp_exposed(&f));
    }

    #[test]
    fn is_mcp_exposed_false_when_flag_is_false() {
        let f = fn_info("mem::observe", Some(json!({ "mcp.expose": false })));
        assert!(!is_mcp_exposed(&f));
    }

    #[test]
    fn sanitize_replaces_boolean_property_values_with_object() {
        let raw = json!({
            "type": "object",
            "properties": { "data": true, "name": { "type": "string" }, "flag": false }
        });
        let s = sanitize_schema(raw);
        assert_eq!(s["properties"]["data"], json!({}));
        assert_eq!(s["properties"]["flag"], json!({}));
        assert_eq!(s["properties"]["name"]["type"], "string");
    }

    #[test]
    fn sanitize_injects_top_level_type_when_missing() {
        let raw = json!({ "title": "AnyValue" });
        let s = sanitize_schema(raw);
        assert_eq!(s["type"], "object");
        assert_eq!(s["title"], "AnyValue");
    }

    #[test]
    fn sanitize_preserves_well_formed_schemas() {
        let raw = json!({
            "type": "object",
            "properties": { "x": { "type": "integer" } },
            "required": ["x"]
        });
        let s = sanitize_schema(raw.clone());
        assert_eq!(s, raw);
    }

    #[test]
    fn sanitize_passes_non_object_schema_through() {
        let raw = json!(true);
        let s = sanitize_schema(raw.clone());
        assert_eq!(s, raw);
    }

    #[test]
    fn function_to_tool_sanitizes_boolean_property_in_response() {
        let f = FunctionInfo {
            function_id: "mem::graph::query".into(),
            description: None,
            request_format: Some(json!({ "title": "AnyValue" })),
            response_format: Some(json!({
                "properties": { "edges": true, "nodes": true, "available": { "type": "boolean" } }
            })),
            metadata: None,
        };
        let tool = function_to_tool(&f);
        assert_eq!(tool["inputSchema"]["type"], "object");
        assert_eq!(tool["outputSchema"]["type"], "object");
        assert_eq!(tool["outputSchema"]["properties"]["edges"], json!({}));
        assert_eq!(tool["outputSchema"]["properties"]["nodes"], json!({}));
        assert_eq!(
            tool["outputSchema"]["properties"]["available"]["type"],
            "boolean"
        );
    }

    #[test]
    fn function_to_tool_uses_request_format_or_default_object() {
        let f = FunctionInfo {
            function_id: "demo::echo".into(),
            description: Some("echo".into()),
            request_format: Some(
                json!({ "type": "object", "properties": { "x": { "type": "string" } }, "required": ["x"] }),
            ),
            response_format: Some(
                json!({ "type": "object", "properties": { "y": { "type": "string" } } }),
            ),
            metadata: None,
        };
        let tool = function_to_tool(&f);
        assert_eq!(tool["name"], "demo__echo");
        assert_eq!(tool["description"], "echo");
        assert_eq!(tool["inputSchema"]["required"][0], "x");
        assert_eq!(tool["outputSchema"]["properties"]["y"]["type"], "string");
    }

    #[test]
    fn function_to_tool_handles_missing_optional_fields() {
        let f = FunctionInfo {
            function_id: "demo::ping".into(),
            description: None,
            request_format: None,
            response_format: None,
            metadata: None,
        };
        let tool = function_to_tool(&f);
        assert_eq!(tool["name"], "demo__ping");
        assert!(tool.get("description").is_none());
        assert!(tool.get("outputSchema").is_none());
        assert_eq!(tool["inputSchema"]["type"], "object");
    }

    #[test]
    fn tool_text_string_pass_through() {
        let v = json!("hello");
        let out = tool_text(&v);
        assert_eq!(out["content"][0]["type"], "text");
        assert_eq!(out["content"][0]["text"], "hello");
        assert_eq!(out["isError"], false);
    }

    #[test]
    fn tool_text_json_is_pretty_printed() {
        let v = json!({ "x": 1 });
        let out = tool_text(&v);
        let text = out["content"][0]["text"].as_str().unwrap();
        assert!(text.contains('\n'), "expected pretty-printed: {text}");
        assert!(text.contains("\"x\""));
    }

    #[test]
    fn tool_error_marks_iserror() {
        let out = tool_error("nope");
        assert_eq!(out["content"][0]["text"], "nope");
        assert_eq!(out["isError"], true);
    }

    #[test]
    fn jsonrpc_success_omits_error() {
        let r = JsonRpcResponse::success(Some(json!(1)), json!({}));
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"result\""));
        assert!(!s.contains("\"error\""));
    }

    #[test]
    fn jsonrpc_error_omits_result() {
        let r = JsonRpcResponse::error(Some(json!(1)), METHOD_NOT_FOUND, "Unknown method");
        let s = serde_json::to_string(&r).unwrap();
        assert!(s.contains("\"error\""));
        assert!(!s.contains("\"result\""));
        assert!(s.contains("-32601"));
    }

    #[test]
    fn initialize_result_has_required_fields() {
        let v = initialize_result();
        assert_eq!(v["protocolVersion"], MCP_PROTOCOL_VERSION);
        assert!(v["capabilities"]["tools"].is_object());
        assert!(v["capabilities"]["resources"].is_object());
        assert!(v["capabilities"]["prompts"].is_object());
        assert_eq!(v["serverInfo"]["name"], "mcp");
        assert_eq!(v["serverInfo"]["version"], env!("CARGO_PKG_VERSION"));
    }
}
