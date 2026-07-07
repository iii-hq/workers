//! Full Chat Completions request assembly: body (messages, tools, thinking,
//! reasoning_effort, response_format) + headers.
use crate::config::ZaiConfig;
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
    /// Pre-resolved GLM `thinking.type` ("enabled"/"disabled"); None omits.
    pub thinking: Option<&'static str>,
    /// Pre-resolved effort string; None omits the param.
    pub reasoning_effort: Option<&'static str>,
    pub response_format: Option<ResponseFormat>,
}

/// `ResponseFormat { type: "json", schema? }` → `json_object` mode, the only
/// structured-output knob Z.AI documents (no strict json_schema mode). A
/// schema, when present, is dropped — the caller is warned upstream and the
/// catalog advertises `supports_structured_output: false`. Z.AI requires the
/// word "JSON" somewhere in the messages (the caller's contract per spec
/// § Model capabilities).
pub fn build_response_format(_rf: &ResponseFormat) -> Value {
    json!({ "type": "json_object" })
}

/// Tool-call argument streaming (`tool_stream`) exists on GLM-4.6 and newer;
/// older families get whole-chunk tool arguments, which the SSE decoder
/// handles either way.
fn supports_tool_stream(model: &str) -> bool {
    let id = model.to_ascii_lowercase();
    id.starts_with("glm-5") || id.starts_with("glm-4.6") || id.starts_with("glm-4.7")
}

/// Z.AI documents `max_tokens` (not OpenAI's `max_completion_tokens`
/// replacement). No `temperature`: the API default applies.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        // Z.AI reports usage natively on the final chunk and tolerates the
        // OpenAI knob; custom OpenAI-compatible endpoints behind an api_url
        // override (vLLM, gateways) emit NO usage chunk without it.
        "stream_options": { "include_usage": true },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        if supports_tool_stream(&args.model) {
            body["tool_stream"] = json!(true);
        }
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(t) = args.thinking {
        body["thinking"] = json!({ "type": t });
    }
    if let Some(effort) = args.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

pub fn build_headers(cfg: &ZaiConfig) -> Vec<(&'static str, String)> {
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
            model: "glm-4.7".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            thinking: None,
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
        assert_eq!(body["model"], "glm-4.7");
        assert_eq!(body["max_tokens"], 4096);
        assert!(
            body.get("max_completion_tokens").is_none(),
            "Z.AI documents max_tokens, not the OpenAI replacement"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("tool_stream").is_none());
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn thinking_effort_and_tools_serialize_when_present() {
        let mut a = args();
        a.thinking = Some("enabled");
        a.reasoning_effort = Some("high");
        a.tools = vec![tool()];
        let body = build_body(&a);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
        assert_eq!(body["tool_stream"], true, "glm-4.7 streams tool args");
    }

    #[test]
    fn tool_stream_is_gated_to_glm_46_and_newer() {
        let mut a = args();
        a.tools = vec![tool()];
        a.model = "glm-4.5-air".into();
        assert!(build_body(&a).get("tool_stream").is_none());
        a.model = "glm-5.2".into();
        assert_eq!(build_body(&a)["tool_stream"], true);
        // never sent without tools
        a.tools = vec![];
        assert!(build_body(&a).get("tool_stream").is_none());
    }

    #[test]
    fn response_format_always_maps_to_json_object() {
        let with_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(serde_json::json!({ "type": "object" })),
        });
        assert_eq!(with_schema["type"], "json_object");
        assert!(with_schema.get("json_schema").is_none());

        let mut a = args();
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(build_body(&a)["response_format"]["type"], "json_object");
    }

    #[test]
    fn headers_carry_bearer_auth() {
        let cfg = ZaiConfig {
            credential_value: "sk-test".into(),
            model: "glm-4.7".into(),
            max_tokens: 4096,
            api_url: "https://api.z.ai/api/paas/v4/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}
