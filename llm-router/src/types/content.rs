use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Content blocks — the atomic units of message content (README § Content blocks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        data: String, // base64
        /// Optional link to the stored original in the session-manager
        /// attachment store. Transcript bookkeeping for lazy readers only:
        /// the harness clears it before `router::chat`, and no provider
        /// mapping reads it. Tolerated here so a block that still carries it
        /// deserializes instead of failing the request; omitted on the wire
        /// when absent.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        attachment_id: Option<String>,
    },
    /// Reference to an original upload in the session-manager attachment
    /// store. A REFERENCE ONLY (never inline bytes); the harness strips it
    /// before `router::chat`. Tolerated here so a stray block deserializes
    /// instead of failing the request; every counter/mapping emits nothing for it.
    File {
        attachment_id: String,
        name: String,
        mime: String,
        size: u64,
    },
    Thinking {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Opaque redacted thinking payload — replayed verbatim on the Anthropic wire.
    RedactedThinking {
        data: String,
    },
    FunctionCall {
        id: String,          // unique per call, echoed by the result
        function_id: String, // the iii function id to invoke
        arguments: serde_json::Value,
    },
    FunctionResult {
        function_call_id: String,
        content: Vec<ContentBlock>,
        #[serde(skip_serializing_if = "Option::is_none")]
        is_error: Option<bool>,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn file_block_round_trips_through_serde_with_the_wire_shape() {
        let wire = json!({
            "type": "file",
            "attachment_id": "a_3f2e",
            "name": "report.pdf",
            "mime": "application/pdf",
            "size": 12345
        });
        let parsed: ContentBlock = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(
            parsed,
            ContentBlock::File {
                attachment_id: "a_3f2e".into(),
                name: "report.pdf".into(),
                mime: "application/pdf".into(),
                size: 12345,
            }
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), wire);
    }

    #[test]
    fn image_attachment_id_is_optional_and_omitted_on_the_wire_when_absent() {
        // Legacy image: no link, and none invented on re-serialization.
        let legacy = json!({ "type": "image", "mime": "image/png", "data": "AAAA" });
        let parsed: ContentBlock = serde_json::from_value(legacy.clone()).unwrap();
        assert_eq!(
            parsed,
            ContentBlock::Image {
                mime: "image/png".into(),
                data: "AAAA".into(),
                attachment_id: None,
            }
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), legacy);

        // A block that still carries the link is tolerated and preserved.
        let linked = json!({
            "type": "image",
            "mime": "image/png",
            "data": "AAAA",
            "attachment_id": "a_9c1d"
        });
        let parsed: ContentBlock = serde_json::from_value(linked.clone()).unwrap();
        assert_eq!(
            parsed,
            ContentBlock::Image {
                mime: "image/png".into(),
                data: "AAAA".into(),
                attachment_id: Some("a_9c1d".into()),
            }
        );
        assert_eq!(serde_json::to_value(&parsed).unwrap(), linked);
    }

    #[test]
    fn a_linked_image_inside_a_message_list_still_deserializes() {
        let messages: Vec<crate::types::messages::AgentMessage> = serde_json::from_value(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "what is this?" },
                { "type": "image", "mime": "image/png", "data": "AAAA", "attachment_id": "a_1" }
            ],
            "timestamp": 1
        }]))
        .unwrap();
        assert_eq!(messages.len(), 1);
    }

    #[test]
    fn a_stray_file_block_inside_a_message_list_still_deserializes() {
        let messages: Vec<crate::types::messages::AgentMessage> = serde_json::from_value(json!([{
            "role": "user",
            "content": [
                { "type": "text", "text": "see attached" },
                { "type": "file", "attachment_id": "a_1", "name": "n.txt", "mime": "text/plain", "size": 3 }
            ],
            "timestamp": 1
        }]))
        .unwrap();
        assert_eq!(messages.len(), 1);
    }
}
