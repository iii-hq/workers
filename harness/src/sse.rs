//! Server-Sent Events helpers for the `/bridge/events` endpoint.

use serde_json::Value;

/// Format a single SSE frame: `id: <id>\ndata: <json>\n\n`.
pub fn format_frame(id: &str, data: &Value) -> String {
    format!("id: {id}\ndata: {data}\n\n")
}

/// Heartbeat comment line. Sent every ~15s to defeat proxy idle timeouts.
pub const fn heartbeat() -> &'static str {
    ": keepalive\n\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn frame_carries_id_and_serialized_data() {
        let f = format_frame("evt-1", &json!({"type": "agent_start"}));
        assert!(f.starts_with("id: evt-1\n"));
        assert!(f.contains("data: {\"type\":\"agent_start\"}"));
        assert!(f.ends_with("\n\n"));
    }

    #[test]
    fn heartbeat_is_a_comment_line() {
        assert!(heartbeat().starts_with(':'));
        assert!(heartbeat().ends_with("\n\n"));
    }
}
