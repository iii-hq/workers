//! Full Chat Completions request assembly: body (messages, tools,
//! reasoning_effort, response_format) + headers.
use crate::config::SarvamConfig;
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
    /// Pre-resolved effort string; None omits the param.
    pub reasoning_effort: Option<&'static str>,
    pub response_format: Option<ResponseFormat>,
}

/// `ResponseFormat { type: "json", schema? }` → `json_object` mode. Sarvam
/// documents `response_format` without a strict json_schema mode, so a
/// schema, when present, is dropped; the caller is warned upstream and the
/// catalog advertises `supports_structured_output: false`.
pub fn build_response_format(_rf: &ResponseFormat) -> Value {
    json!({ "type": "json_object" })
}

/// Sarvam documents `max_tokens` (not OpenAI's `max_completion_tokens`
/// replacement) and drops unknown OpenAI knobs such as `stream_options`,
/// so none are sent. No `temperature`: the API default applies.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(effort) = args.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

/// Sarvam's chat endpoint takes bearer auth and every other endpoint takes
/// `api-subscription-key`; both carry the same key, so both go on every
/// request and a gateway in front of either shape is satisfied.
pub fn build_headers(cfg: &SarvamConfig) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", cfg.credential_value)),
        ("api-subscription-key", cfg.credential_value.clone()),
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
            model: "sarvam-105b".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            reasoning_effort: None,
            response_format: None,
        }
    }

    fn tool() -> AgentFunction {
        AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }
    }

    #[test]
    fn body_has_required_fields_and_documented_max_tokens_param() {
        let body = build_body(&args());
        assert_eq!(body["model"], "sarvam-105b");
        assert_eq!(body["max_tokens"], 4096);
        assert!(body.get("max_completion_tokens").is_none());
        assert!(
            body.get("stream_options").is_none(),
            "Sarvam drops unknown OpenAI knobs; usage arrives on the final chunk"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
    }

    #[test]
    fn tools_and_effort_ride_when_present() {
        let mut a = args();
        a.reasoning_effort = Some("high");
        a.tools = vec![tool()];
        let body = build_body(&a);
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
        assert!(body.get("tool_stream").is_none());
    }

    #[test]
    fn response_format_always_maps_to_json_object() {
        let with_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(serde_json::json!({ "type": "object" })),
        });
        assert_eq!(with_schema["type"], "json_object");
        assert!(with_schema.get("json_schema").is_none());
    }

    #[test]
    fn headers_carry_both_auth_shapes() {
        let cfg = SarvamConfig {
            credential_value: "sk_test".into(),
            model: "sarvam-105b".into(),
            max_tokens: 4096,
            api_url: "https://api.sarvam.ai/v1/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk_test".to_string())));
        assert!(h.contains(&("api-subscription-key", "sk_test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}
