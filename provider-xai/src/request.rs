//! Full Chat Completions request assembly: body (messages, tools,
//! reasoning_effort, response_format) + headers.
use crate::config::XaiConfig;
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
    /// Pre-resolved effort string (Task 6); None omits the param.
    pub reasoning_effort: Option<&'static str>,
    pub response_format: Option<ResponseFormat>,
}

/// `ResponseFormat { type: "json", schema? }` → native xAI knob.
/// With a schema: strict json_schema mode (constrained decoding — a schema
/// that violates xAI's strict-mode rules 400s as `permanent`, which is
/// correct: retrying cannot fix the schema). Without: json_object mode
/// (xAI requires the word "JSON" somewhere in the messages — the
/// caller's contract per spec § Model capabilities).
pub fn build_response_format(rf: &ResponseFormat) -> Value {
    match &rf.schema {
        Some(schema) => json!({
            "type": "json_schema",
            "json_schema": { "name": "response", "strict": true, "schema": schema }
        }),
        None => json!({ "type": "json_object" }),
    }
}

/// `max_completion_tokens`, not the deprecated `max_tokens`: the o-series
/// and grok-4 families reject the old param. Reasoning tokens count toward
/// it; the router-clamped budget leaves ample room. No `temperature`: the
/// API default applies (reasoning models reject non-default values).
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_completion_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        "stream_options": { "include_usage": true },
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

pub fn build_headers(cfg: &XaiConfig) -> Vec<(&'static str, String)> {
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
            model: "grok-4".into(),
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

    #[test]
    fn body_has_required_fields_and_stream_options() {
        let body = build_body(&args());
        assert_eq!(body["model"], "grok-4");
        assert_eq!(body["max_completion_tokens"], 4096);
        assert!(
            body.get("max_tokens").is_none(),
            "deprecated param never sent"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("reasoning_effort").is_none());
        assert!(body.get("response_format").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn reasoning_effort_and_tools_serialize_when_present() {
        let mut a = args();
        a.reasoning_effort = Some("high");
        a.tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: serde_json::json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let body = build_body(&a);
        assert_eq!(body["reasoning_effort"], "high");
        assert_eq!(body["tools"][0]["function"]["name"], "agent__trigger");
    }

    #[test]
    fn response_format_maps_to_json_schema_or_json_object() {
        let with_schema = build_response_format(&ResponseFormat {
            r#type: "json".into(),
            schema: Some(serde_json::json!({ "type": "object", "additionalProperties": false })),
        });
        assert_eq!(with_schema["type"], "json_schema");
        assert_eq!(with_schema["json_schema"]["name"], "response");
        assert_eq!(with_schema["json_schema"]["strict"], true);
        assert_eq!(with_schema["json_schema"]["schema"]["type"], "object");

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
        let cfg = XaiConfig {
            credential_value: "sk-test".into(),
            model: "grok-4".into(),
            max_tokens: 4096,
            api_url: "https://api.x.ai/v1/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
    }
}
