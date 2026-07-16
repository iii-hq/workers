//! Full Chat Completions request assembly: body (messages, tools,
//! response_format) + headers. Moonshot is OpenAI Chat Completions-compatible
//! with two relevant differences from OpenAI: it accepts the classic
//! `max_tokens` param, and it has no `reasoning_effort` knob (thinking is
//! implicit per model — surfaced via the `reasoning_content` stream).
use crate::config::KimiConfig;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{json, Value};

pub struct BodyArgs {
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub response_format: Option<ResponseFormat>,
}

/// Moonshot supports only JSON mode (`{"type":"json_object"}`) — there is no
/// strict `json_schema` constrained-decoding mode like OpenAI's. A requested
/// schema is therefore advisory: we still ask for json_object (and stream_fn
/// emits a report-and-continue warning so the caller knows the schema was not
/// enforced). json_object mode requires the word "JSON" somewhere in the
/// messages — the caller's contract per spec § Model capabilities.
pub fn build_response_format(_rf: &ResponseFormat) -> Value {
    json!({ "type": "json_object" })
}

/// The classic `max_tokens` param (Moonshot accepts it directly). No
/// `reasoning_effort`: Moonshot has no such knob. No `temperature`: the API
/// default applies.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        "stream_options": { "include_usage": true },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

pub fn build_headers(cfg: &KimiConfig) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", cfg.credential_value)),
        ("content-type", "application/json".to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn args() -> BodyArgs {
        BodyArgs {
            model: "kimi-k2-0905-preview".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            response_format: None,
        }
    }

    #[test]
    fn body_has_required_fields_and_stream_options() {
        let body = build_body(&args());
        assert_eq!(body["model"], "kimi-k2-0905-preview");
        // Moonshot uses the classic max_tokens, not max_completion_tokens.
        assert_eq!(body["max_tokens"], 4096);
        assert!(
            body.get("max_completion_tokens").is_none(),
            "openai-only param never sent"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(
            body.get("reasoning_effort").is_none(),
            "moonshot has no reasoning_effort knob"
        );
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn tools_serialize_when_present() {
        let mut a = args();
        a.tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let body = build_body(&a);
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn response_format_always_maps_to_json_object() {
        // schema present and absent both degrade to json_object (Moonshot has
        // no strict json_schema mode).
        let with_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(serde_json::json!({ "type": "object", "additionalProperties": false })),
        });
        assert_eq!(with_schema["type"], "json_object");
        assert!(with_schema.get("json_schema").is_none());

        let without = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(without["type"], "json_object");

        let mut a = args();
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(build_body(&a)["response_format"]["type"], "json_object");
    }

    #[test]
    fn headers_carry_bearer_auth() {
        let cfg = KimiConfig {
            credential_value: "sk-test".into(),
            model: "kimi-k2-0905-preview".into(),
            max_tokens: 4096,
            api_url: "https://api.moonshot.ai/v1/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}
