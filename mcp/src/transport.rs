//! MCP Streamable HTTP transport: one SSE event whose data line is the
//! JSON-RPC reply, and on a session's `GET` stream one SSE event per
//! observation, as a JSON-RPC notification.

use serde_json::{json, Value};

use crate::jsonrpc::JsonRpcResponse;

/// The notification an SSE observer's observations arrive as.
pub const OBSERVATION_METHOD: &str = "notifications/iii/observation";

/// The notification for observations the buffer cap dropped before a stream
/// could send them.
pub const GAP_METHOD: &str = "notifications/iii/observations_dropped";

fn event(id: u64, note: &Value) -> String {
    format!("id: {id}\nevent: message\ndata: {note}\n\n")
}

/// One observation, as a JSON-RPC notification with SSE event id `id` (the
/// `Last-Event-ID` a reconnecting client resumes after).
pub fn encode_sse_observation(id: u64, observation: &Value) -> String {
    event(
        id,
        &json!({ "jsonrpc": "2.0", "method": OBSERVATION_METHOD, "params": observation }),
    )
}

/// Event ids after `after`, up to `through`, were dropped. Its own id is
/// `through`, so a client resuming from it skips nothing more.
pub fn encode_sse_gap(after: u64, through: u64) -> String {
    event(
        through,
        &json!({
            "jsonrpc": "2.0",
            "method": GAP_METHOD,
            "params": { "after_event_id": after, "through_event_id": through }
        }),
    )
}

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

    fn parse(frame: &str) -> (u64, serde_json::Value) {
        let rest = frame.strip_prefix("id: ").expect("an id line first");
        let (id, rest) = rest.split_once('\n').unwrap();
        let data = rest
            .strip_prefix("event: message\ndata: ")
            .unwrap()
            .strip_suffix("\n\n")
            .unwrap();
        assert!(!data.contains('\n'), "one data line");
        (id.parse().unwrap(), serde_json::from_str(data).unwrap())
    }

    #[test]
    fn an_observation_is_a_notification_with_its_event_id() {
        let (id, v) = parse(&encode_sse_observation(
            7,
            &json!({ "observer_id": "o-1", "input": { "k": 1 } }),
        ));
        assert_eq!(id, 7);
        assert_eq!(
            v,
            json!({
                "jsonrpc": "2.0",
                "method": OBSERVATION_METHOD,
                "params": { "observer_id": "o-1", "input": { "k": 1 } }
            })
        );
        assert_eq!(OBSERVATION_METHOD, "notifications/iii/observation");
    }

    #[test]
    fn a_gap_says_which_event_ids_were_dropped() {
        let (id, v) = parse(&encode_sse_gap(3, 9));
        // Its id is the last one dropped, so resuming from it skips nothing.
        assert_eq!(id, 9);
        assert_eq!(
            v,
            json!({
                "jsonrpc": "2.0",
                "method": GAP_METHOD,
                "params": { "after_event_id": 3, "through_event_id": 9 }
            })
        );
        assert_eq!(GAP_METHOD, "notifications/iii/observations_dropped");
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
