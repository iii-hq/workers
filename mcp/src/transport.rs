//! MCP Streamable HTTP transport: one SSE event whose data line is the
//! JSON-RPC reply.
//!
//! Direct port of `src/mcp/transport.ts`.

use crate::jsonrpc::JsonRpcResponse;

pub fn encode_sse_jsonrpc(rpc: &JsonRpcResponse) -> String {
    format!(
        "event: message\ndata: {}\n\n",
        serde_json::to_string(rpc).expect("JsonRpcResponse must serialize")
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jsonrpc::{json_rpc_success, JsonRpcId, JsonRpcResponseId};
    use serde_json::json;

    #[test]
    fn frame_starts_with_event_message_line() {
        let resp = json_rpc_success(
            JsonRpcResponseId::Id(JsonRpcId::Number(1)),
            json!({"protocolVersion": "2025-06-18"}),
        );
        let frame = encode_sse_jsonrpc(&resp);
        assert!(frame.starts_with("event: message\ndata: "));
        assert!(frame.ends_with("\n\n"));
    }

    #[test]
    fn frame_contains_full_jsonrpc_payload() {
        let resp = json_rpc_success(
            JsonRpcResponseId::Id(JsonRpcId::String("abc".into())),
            json!({"ok": true}),
        );
        let frame = encode_sse_jsonrpc(&resp);
        let data_line = frame
            .strip_prefix("event: message\ndata: ")
            .unwrap()
            .strip_suffix("\n\n")
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_str(data_line).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], "abc");
        assert_eq!(parsed["result"]["ok"], true);
    }
}
