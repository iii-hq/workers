//! JSON-RPC 2.0 helpers for the MCP Streamable HTTP bridge.
//!
//! Direct port of `src/mcp/jsonrpc.ts`.

use serde::Serialize;
use serde_json::Value;

pub const JSON_RPC_VERSION: &str = "2.0";

/// Parse error -- Invalid JSON. Defined for spec completeness; the engine
/// rejects malformed JSON before it reaches the bridge.
#[allow(dead_code)]
pub const PARSE_ERROR: i64 = -32_700;
/// Invalid Request -- not a valid JSON-RPC object (includes batch).
pub const INVALID_REQUEST: i64 = -32_600;
pub const METHOD_NOT_FOUND: i64 = -32_601;
pub const INVALID_PARAMS: i64 = -32_602;
pub const INTERNAL_ERROR: i64 = -32_603;

/// JSON-RPC id (string or number). Response id may also be `null` for system
/// errors; we represent the response shape with [`JsonRpcResponseId`].
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcId {
    String(String),
    Number(i64),
}

impl JsonRpcId {
    fn from_value(v: &Value) -> Option<Self> {
        match v {
            Value::String(s) => Some(JsonRpcId::String(s.clone())),
            Value::Number(n) => n.as_i64().map(JsonRpcId::Number),
            _ => None,
        }
    }

    fn to_value(&self) -> Value {
        match self {
            JsonRpcId::String(s) => Value::String(s.clone()),
            JsonRpcId::Number(n) => Value::Number((*n).into()),
        }
    }
}

/// Response id -- either a real id or `null` for system errors (e.g. invalid
/// batch).
#[derive(Debug, Clone, PartialEq)]
pub enum JsonRpcResponseId {
    Id(JsonRpcId),
    Null,
}

impl JsonRpcResponseId {
    fn to_value(&self) -> Value {
        match self {
            JsonRpcResponseId::Id(id) => id.to_value(),
            JsonRpcResponseId::Null => Value::Null,
        }
    }
}

/// Parsed JSON-RPC request.
#[derive(Debug, Clone)]
pub struct JsonRpcRequest {
    pub method: String,
    pub params: Value,
    pub id: Option<JsonRpcId>,
}

#[derive(Debug, Clone)]
pub enum JsonRpcResponse {
    Success {
        id: JsonRpcResponseId,
        result: Value,
    },
    Failure {
        id: JsonRpcResponseId,
        code: i64,
        message: String,
        data: Option<Value>,
    },
}

#[derive(Serialize)]
struct WireSuccess<'a> {
    jsonrpc: &'a str,
    id: Value,
    result: &'a Value,
}

#[derive(Serialize)]
struct WireError<'a> {
    code: i64,
    message: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<&'a Value>,
}

#[derive(Serialize)]
struct WireFailure<'a> {
    jsonrpc: &'a str,
    id: Value,
    error: WireError<'a>,
}

impl serde::Serialize for JsonRpcResponse {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            JsonRpcResponse::Success { id, result } => WireSuccess {
                jsonrpc: JSON_RPC_VERSION,
                id: id.to_value(),
                result,
            }
            .serialize(serializer),
            JsonRpcResponse::Failure {
                id,
                code,
                message,
                data,
            } => WireFailure {
                jsonrpc: JSON_RPC_VERSION,
                id: id.to_value(),
                error: WireError {
                    code: *code,
                    message,
                    data: data.as_ref(),
                },
            }
            .serialize(serializer),
        }
    }
}

pub fn json_rpc_success(id: JsonRpcResponseId, result: Value) -> JsonRpcResponse {
    JsonRpcResponse::Success { id, result }
}

pub fn json_rpc_error(
    id: JsonRpcResponseId,
    code: i64,
    message: impl Into<String>,
) -> JsonRpcResponse {
    JsonRpcResponse::Failure {
        id,
        code,
        message: message.into(),
        data: None,
    }
}

#[allow(dead_code)]
pub fn json_rpc_error_with_data(
    id: JsonRpcResponseId,
    code: i64,
    message: impl Into<String>,
    data: Value,
) -> JsonRpcResponse {
    JsonRpcResponse::Failure {
        id,
        code,
        message: message.into(),
        data: Some(data),
    }
}

/// Returns true if `value` looks like a JSON-RPC 2.0 request: object with
/// `jsonrpc == "2.0"` and `method: string`. Mirrors the TS predicate exactly.
pub fn is_json_rpc_request(value: &Value) -> bool {
    let Some(obj) = value.as_object() else {
        return false;
    };
    let jsonrpc_ok = obj.get("jsonrpc").and_then(Value::as_str) == Some("2.0");
    let method_ok = obj.get("method").and_then(Value::as_str).is_some();
    jsonrpc_ok && method_ok
}

/// Parse a value into [`JsonRpcRequest`]. Caller must have already passed
/// [`is_json_rpc_request`] -- this never fails the shape check; it only
/// extracts fields. Missing `params` becomes `null`.
pub fn parse_json_rpc_request(value: &Value) -> Option<JsonRpcRequest> {
    let obj = value.as_object()?;
    let method = obj.get("method")?.as_str()?.to_string();
    let params = obj.get("params").cloned().unwrap_or(Value::Null);
    let id = obj.get("id").and_then(JsonRpcId::from_value);
    Some(JsonRpcRequest { method, params, id })
}

/// Pull a response id out of an arbitrary body so we can echo it back in error
/// responses for malformed-but-id-bearing requests. Mirrors
/// `responseIdFromBody` in the TS handler.
pub fn response_id_from_body(value: &Value) -> JsonRpcResponseId {
    if let Some(obj) = value.as_object() {
        if let Some(id) = obj.get("id").and_then(JsonRpcId::from_value) {
            return JsonRpcResponseId::Id(id);
        }
    }
    JsonRpcResponseId::Null
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_json_rpc_request_accepts_valid_shape() {
        let v = json!({"jsonrpc": "2.0", "method": "tools/list", "id": 1});
        assert!(is_json_rpc_request(&v));
    }

    #[test]
    fn is_json_rpc_request_rejects_arrays_and_nulls() {
        assert!(!is_json_rpc_request(&json!([])));
        assert!(!is_json_rpc_request(&Value::Null));
        assert!(!is_json_rpc_request(&json!({"method": "ping"})));
        assert!(!is_json_rpc_request(
            &json!({"jsonrpc": "1.0", "method": "x"})
        ));
        assert!(!is_json_rpc_request(&json!({"jsonrpc": "2.0"})));
    }

    #[test]
    fn parses_params_and_id() {
        let v = json!({"jsonrpc": "2.0", "method": "ping", "id": "abc", "params": {"k": 1}});
        let r = parse_json_rpc_request(&v).unwrap();
        assert_eq!(r.method, "ping");
        assert_eq!(r.id, Some(JsonRpcId::String("abc".into())));
        assert_eq!(r.params["k"], 1);
    }

    #[test]
    fn missing_id_means_notification() {
        let v = json!({"jsonrpc": "2.0", "method": "notifications/initialized"});
        let r = parse_json_rpc_request(&v).unwrap();
        assert!(r.id.is_none());
    }

    #[test]
    fn success_serialises_to_jsonrpc_2() {
        let resp = json_rpc_success(
            JsonRpcResponseId::Id(JsonRpcId::Number(7)),
            json!({"ok": true}),
        );
        let s = serde_json::to_string(&resp).unwrap();
        let parsed: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 7);
        assert_eq!(parsed["result"]["ok"], true);
        assert!(parsed.get("error").is_none());
    }

    #[test]
    fn error_serialises_with_code_and_message() {
        let resp = json_rpc_error(
            JsonRpcResponseId::Null,
            METHOD_NOT_FOUND,
            "Method not found",
        );
        let s = serde_json::to_string(&resp).unwrap();
        let parsed: Value = serde_json::from_str(&s).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert!(parsed["id"].is_null());
        assert_eq!(parsed["error"]["code"], -32_601);
        assert_eq!(parsed["error"]["message"], "Method not found");
        assert!(parsed.get("result").is_none());
    }

    #[test]
    fn response_id_from_body_handles_strings_and_numbers() {
        let v = json!({"id": "abc"});
        assert_eq!(
            response_id_from_body(&v),
            JsonRpcResponseId::Id(JsonRpcId::String("abc".into()))
        );
        let v = json!({"id": 42});
        assert_eq!(
            response_id_from_body(&v),
            JsonRpcResponseId::Id(JsonRpcId::Number(42))
        );
        assert_eq!(response_id_from_body(&json!([])), JsonRpcResponseId::Null);
        assert_eq!(response_id_from_body(&json!({})), JsonRpcResponseId::Null);
    }

    #[test]
    fn error_codes_match_jsonrpc_spec() {
        assert_eq!(PARSE_ERROR, -32_700);
        assert_eq!(INVALID_REQUEST, -32_600);
        assert_eq!(METHOD_NOT_FOUND, -32_601);
        assert_eq!(INVALID_PARAMS, -32_602);
        assert_eq!(INTERNAL_ERROR, -32_603);
    }
}
