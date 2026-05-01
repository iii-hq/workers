use serde::{Deserialize, Serialize};
use serde_json::Value;

pub mod handler;

pub const FUNCTION_ID: &str = "hooks::publish_collect";
pub const HOOK_REPLY_STREAM: &str = "agent::hook_reply";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MergeRule {
    FirstBlockWins,
    FieldMerge,
    PipelineLastWins,
}

impl MergeRule {
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "first_block_wins" => Some(Self::FirstBlockWins),
            "field_merge" => Some(Self::FieldMerge),
            "pipeline_last_wins" => Some(Self::PipelineLastWins),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishCollectRequest {
    pub topic: String,
    pub payload: Value,
    pub merge_rule: MergeRule,
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublishCollectResponse {
    pub event_id: String,
    pub replies: Vec<Value>,
    pub merged: Value,
}

pub fn merge_first_block_wins(replies: &[Value]) -> Value {
    for reply in replies {
        if reply.get("block").and_then(Value::as_bool).unwrap_or(false) {
            return serde_json::json!({
                "block": true,
                "reason": reply.get("reason").cloned().unwrap_or(Value::Null),
            });
        }
    }
    serde_json::json!({ "block": false })
}

pub fn merge_field_merge(mut initial: Value, replies: &[Value]) -> Value {
    for reply in replies {
        if let Some(content) = reply.get("content") {
            initial["content"] = content.clone();
        }
        if let Some(details) = reply.get("details") {
            if let (Some(existing), Some(incoming)) = (
                initial.get_mut("details").and_then(Value::as_object_mut),
                details.as_object(),
            ) {
                for (key, value) in incoming {
                    existing.insert(key.clone(), value.clone());
                }
            } else if details.is_object() {
                initial["details"] = details.clone();
            }
        }
        if let Some(terminate) = reply.get("terminate").and_then(Value::as_bool) {
            initial["terminate"] = Value::Bool(terminate);
        }
    }
    initial
}

pub fn merge_pipeline_last_wins(original: Value, replies: &[Value]) -> Value {
    replies
        .iter()
        .filter_map(decode_transform_messages)
        .last()
        .unwrap_or(original)
}

pub fn build_publish_envelope(topic: &str, event_id: &str, payload: Value) -> Value {
    serde_json::json!({
        "topic": topic,
        "data": {
            "event_id": event_id,
            "reply_stream": HOOK_REPLY_STREAM,
            "payload": payload,
        },
    })
}

pub fn register_with_iii(iii: &iii_sdk::III) {
    handler::register(iii);
}

fn decode_transform_messages(reply: &Value) -> Option<Value> {
    if reply.is_array() {
        return Some(reply.clone());
    }
    reply
        .get("messages")
        .filter(|messages| messages.is_array())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn merge_rule_parses_known_values() {
        assert_eq!(
            MergeRule::parse("first_block_wins"),
            Some(MergeRule::FirstBlockWins)
        );
        assert_eq!(MergeRule::parse("field_merge"), Some(MergeRule::FieldMerge));
        assert_eq!(
            MergeRule::parse("pipeline_last_wins"),
            Some(MergeRule::PipelineLastWins)
        );
        assert_eq!(MergeRule::parse("unknown"), None);
    }

    #[test]
    fn first_block_wins_returns_first_blocker() {
        let replies = vec![
            json!({}),
            json!({ "block": false }),
            json!({ "block": true, "reason": "first" }),
            json!({ "block": true, "reason": "second" }),
        ];

        let merged = merge_first_block_wins(&replies);

        assert_eq!(merged["block"], true);
        assert_eq!(merged["reason"], "first");
    }

    #[test]
    fn first_block_wins_defaults_to_no_block() {
        let replies = vec![json!({}), json!({ "block": false })];

        let merged = merge_first_block_wins(&replies);

        assert_eq!(merged, json!({ "block": false }));
    }

    #[test]
    fn field_merge_replaces_content_merges_details_and_updates_terminate() {
        let initial = json!({
            "content": [{ "type": "text", "text": "ok" }],
            "details": { "a": 1, "b": 2 },
            "terminate": false
        });
        let replies = vec![json!({
            "content": [{ "type": "text", "text": "rewritten" }],
            "details": { "b": 99, "c": 3 },
            "terminate": true
        })];

        let merged = merge_field_merge(initial, &replies);

        assert_eq!(merged["content"][0]["text"], "rewritten");
        assert_eq!(merged["details"]["a"], 1);
        assert_eq!(merged["details"]["b"], 99);
        assert_eq!(merged["details"]["c"], 3);
        assert_eq!(merged["terminate"], true);
    }

    #[test]
    fn pipeline_last_wins_uses_last_decodable_messages() {
        let original = json!([{ "role": "user", "content": [], "timestamp": 0 }]);
        let replies = vec![
            json!({ "messages": [{ "role": "assistant", "content": [], "timestamp": 1 }] }),
            json!({ "ignored": true }),
            json!([{ "role": "user", "content": [], "timestamp": 2 }]),
        ];

        let merged = merge_pipeline_last_wins(original, &replies);

        assert_eq!(merged[0]["timestamp"], 2);
    }

    #[test]
    fn pipeline_last_wins_falls_back_to_original() {
        let original = json!([{ "role": "user", "content": [], "timestamp": 0 }]);
        let replies = vec![json!({ "ignored": true })];

        let merged = merge_pipeline_last_wins(original.clone(), &replies);

        assert_eq!(merged, original);
    }

    #[test]
    fn build_publish_envelope_matches_existing_subscribers() {
        let envelope = build_publish_envelope(
            "agent::before_tool_call",
            "event-1",
            json!({"tool_call": {"id": "t1"}}),
        );

        assert_eq!(envelope["topic"], "agent::before_tool_call");
        assert_eq!(envelope["data"]["event_id"], "event-1");
        assert_eq!(envelope["data"]["reply_stream"], HOOK_REPLY_STREAM);
        assert_eq!(envelope["data"]["payload"]["tool_call"]["id"], "t1");
    }
}
