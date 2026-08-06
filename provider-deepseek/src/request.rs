//! Full Chat Completions request assembly: body (messages, tools, thinking,
//! reasoning_effort, response_format) + headers.
use crate::config::DeepseekConfig;
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
    /// Pre-resolved `thinking.type` ("enabled"); None omits the param so the
    /// model's own default applies (V4: thinking on at high effort).
    pub thinking: Option<&'static str>,
    /// Pre-resolved effort string; None omits the param.
    pub reasoning_effort: Option<&'static str>,
    pub response_format: Option<ResponseFormat>,
}

/// `ResponseFormat { type: "json", schema? }` → `json_object` mode, the only
/// structured-output knob DeepSeek documents (no strict json_schema mode). A
/// schema, when present, is dropped — the caller is warned upstream and the
/// catalog advertises `supports_structured_output: false`. DeepSeek requires
/// the word "json" somewhere in the messages (the caller's contract per spec
/// § Model capabilities).
///
/// That `false` means the router's own gate rejects `response_format` for a
/// model it has in its catalog (`llm-router::chat` § structured-output gate),
/// so this path serves the fail-open case: a model the catalog does not know
/// — a cold catalog, or an `api_url` override onto an unreconciled endpoint —
/// where the provider is the final arbiter.
pub fn build_response_format(_rf: &ResponseFormat) -> Value {
    json!({ "type": "json_object" })
}

/// DeepSeek documents `max_tokens` (not OpenAI's `max_completion_tokens`
/// replacement). No `temperature`/`top_p`: the API ignores both while
/// thinking is on, and the default applies otherwise.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        // Without this there is no usage chunk at all — DeepSeek documents
        // `include_usage` as the way to get token stats before `[DONE]`.
        "stream_options": { "include_usage": true },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(t) = args.thinking {
        body["thinking"] = json!({ "type": t });
    }
    // Top-level, NOT nested in `thinking` (guides/thinking_mode).
    if let Some(effort) = args.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

pub fn build_headers(cfg: &DeepseekConfig) -> Vec<(&'static str, String)> {
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
            model: "deepseek-v4-pro".into(),
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
        assert_eq!(body["model"], "deepseek-v4-pro");
        assert_eq!(body["max_tokens"], 4096);
        assert!(
            body.get("max_completion_tokens").is_none(),
            "DeepSeek documents max_tokens, not the OpenAI replacement"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
        assert!(body.get("top_p").is_none());
    }

    #[test]
    fn thinking_and_effort_are_separate_top_level_params() {
        let mut a = args();
        a.thinking = Some("enabled");
        a.reasoning_effort = Some("max");
        a.tools = vec![tool()];
        let body = build_body(&a);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["reasoning_effort"], "max");
        assert!(
            body["thinking"].get("reasoning_effort").is_none(),
            "effort is top-level, never nested in thinking"
        );
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn absent_thinking_level_omits_both_reasoning_params() {
        // No level → both params omitted → the model's own default applies
        // (V4: thinking on at high effort), so an unconfigured console chat
        // still streams its chain of thought; a legacy non-thinking alias
        // keeps the behavior its name encodes.
        let body = build_body(&args());
        assert!(body.get("thinking").is_none());
        assert!(body.get("reasoning_effort").is_none());
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
        let cfg = DeepseekConfig {
            credential_value: "sk-test".into(),
            model: "deepseek-v4-pro".into(),
            max_tokens: 4096,
            api_url: crate::config::DEFAULT_API_URL.into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}
