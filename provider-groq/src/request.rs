//! Full Chat Completions request assembly: body (messages, tools,
//! reasoning_effort, response_format) + headers.
use crate::config::GroqConfig;
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
    /// Pre-resolved `reasoning_effort`; None omits the param so the model's
    /// own default applies.
    pub reasoning_effort: Option<&'static str>,
    pub response_format: Option<ResponseFormat>,
}

/// `ResponseFormat { type: "json", schema? }` → Groq's `response_format`.
///
/// A schema rides as `json_schema`, which Groq's OpenAI-compatible surface
/// documents alongside `json_object` and plain text. Without one there is no
/// schema to enforce, so the request asks only for valid JSON — and in that
/// mode the caller must mention "json" in the prompt, which is their contract
/// per spec § Model capabilities.
pub fn build_response_format(rf: &ResponseFormat) -> Value {
    match &rf.schema {
        Some(schema) => json!({
            "type": "json_schema",
            "json_schema": { "name": "response", "schema": schema, "strict": true },
        }),
        None => json!({ "type": "json_object" }),
    }
}

/// Groq documents `max_completion_tokens`, not the legacy `max_tokens`. No
/// `temperature`/`top_p`: each model's own default applies unless a caller
/// has a reason to move it, and nothing upstream of here expresses one.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_completion_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        // Without this there is no usage chunk at all — Groq documents
        // `include_usage` as the way to get token stats before `[DONE]`.
        "stream_options": { "include_usage": true },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    // Groq requests reasoning with `reasoning_effort` alone; there is no
    // `thinking` object to enable first.
    if let Some(effort) = args.reasoning_effort {
        body["reasoning_effort"] = json!(effort);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

pub fn build_headers(cfg: &GroqConfig) -> Vec<(&'static str, String)> {
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
            model: "llama-3.3-70b-versatile".into(),
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
    fn body_has_required_fields_and_the_documented_output_cap_param() {
        let body = build_body(&args());
        assert_eq!(body["model"], "llama-3.3-70b-versatile");
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(
            body.get("max_tokens").is_none(),
            "Groq documents max_completion_tokens, not the legacy spelling"
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
    fn reasoning_rides_as_one_top_level_param() {
        // Groq has no `thinking` object to enable first: sending one would be
        // an unknown parameter on every reasoning request.
        let mut a = args();
        a.reasoning_effort = Some("high");
        a.tools = vec![tool()];
        let body = build_body(&a);
        assert_eq!(body["reasoning_effort"], "high");
        assert!(body.get("thinking").is_none());
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn absent_thinking_level_omits_the_reasoning_param() {
        // No level → the param is omitted → the model's own default applies.
        let body = build_body(&args());
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("thinking").is_none());
    }

    #[test]
    fn a_schema_rides_as_json_schema_and_bare_json_asks_only_for_json() {
        let with_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(serde_json::json!({ "type": "object" })),
        });
        assert_eq!(with_schema["type"], "json_schema");
        assert_eq!(with_schema["json_schema"]["schema"]["type"], "object");
        assert_eq!(with_schema["json_schema"]["strict"], true);

        let mut a = args();
        a.response_format = Some(ResponseFormat {
            r#type: "json".into(),
            schema: None,
        });
        assert_eq!(build_body(&a)["response_format"]["type"], "json_object");
    }

    #[test]
    fn headers_carry_bearer_auth() {
        let cfg = GroqConfig {
            credential_value: "sk-test".into(),
            model: "llama-3.3-70b-versatile".into(),
            max_tokens: 4096,
            api_url: crate::config::DEFAULT_API_URL.into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}
