//! Full Chat Completions request assembly: body (messages, tools,
//! response_format, reasoning) + headers. OpenRouter is OpenAI Chat
//! Completions-compatible with three relevant additions: the unified
//! `reasoning: {effort}` parameter (normalized across vendors), native
//! strict `json_schema` structured outputs on models that declare
//! `structured_outputs`, and `usage: {include: true}` usage accounting —
//! the final chunk then carries native token counts plus the actual billed
//! cost (cache discounts included).
use crate::config::OpenRouterConfig;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use serde_json::{json, Value};

/// App attribution headers OpenRouter reads for its rankings; identifies the
/// harness stack, never the end user.
pub const ATTRIBUTION_REFERER: &str = "https://iii.dev";
pub const ATTRIBUTION_TITLE: &str = "iii";

pub struct BodyArgs {
    /// The id OpenRouter expects — already stripped of the catalog prefix.
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub response_format: Option<ResponseFormat>,
    /// Effort string for the unified `reasoning` parameter, already resolved
    /// against the model's advertised efforts (stream_fn's concern).
    pub reasoning_effort: Option<String>,
    /// Whether the model declares strict `json_schema` support; without it a
    /// requested schema degrades to `json_object` (with a warning upstream).
    pub allow_json_schema: bool,
}

/// A requested schema maps to OpenRouter's strict `json_schema` mode when the
/// model declares `structured_outputs`; otherwise — and when no schema was
/// given — plain `json_object` mode (the caller's prompt must mention JSON
/// per the OpenAI-compatible contract).
pub fn build_response_format(rf: &ResponseFormat, allow_json_schema: bool) -> Value {
    match &rf.schema {
        Some(schema) if allow_json_schema => json!({
            "type": "json_schema",
            "json_schema": {
                "name": "response",
                "strict": true,
                "schema": schema,
            }
        }),
        _ => json!({ "type": "json_object" }),
    }
}

/// The classic `max_tokens` param (OpenRouter accepts it directly across
/// vendors). `usage.include` turns on usage accounting; `stream_options`
/// stays for OpenAI-compatible gateways pointed at via `api_url`.
pub fn build_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        "stream_options": { "include_usage": true },
        "usage": { "include": true },
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(rf) = &args.response_format {
        body["response_format"] = build_response_format(rf, args.allow_json_schema);
    }
    if let Some(effort) = &args.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
    }
    body
}

pub fn build_headers(cfg: &OpenRouterConfig) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", cfg.credential_value)),
        ("content-type", "application/json".to_string()),
        ("http-referer", ATTRIBUTION_REFERER.to_string()),
        ("x-openrouter-title", ATTRIBUTION_TITLE.to_string()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn args() -> BodyArgs {
        BodyArgs {
            model: "anthropic/claude-x".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            response_format: None,
            reasoning_effort: None,
            allow_json_schema: true,
        }
    }

    #[test]
    fn body_has_required_fields_and_usage_accounting() {
        let body = build_body(&args());
        assert_eq!(body["model"], "anthropic/claude-x");
        assert_eq!(body["max_tokens"], 4096);
        assert!(
            body.get("max_completion_tokens").is_none(),
            "openai-only param never sent"
        );
        assert_eq!(body["stream"], true);
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["usage"]["include"], true);
        assert_eq!(body["messages"][0]["role"], "system");
        assert_eq!(body["messages"][1]["role"], "user");
        assert!(body.get("tools").is_none(), "empty tools array omitted");
        assert!(body.get("reasoning").is_none(), "no effort → no param");
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
    fn reasoning_effort_rides_the_unified_parameter() {
        let mut a = args();
        a.reasoning_effort = Some("high".into());
        let body = build_body(&a);
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn schema_maps_to_strict_json_schema_when_supported() {
        let schema = serde_json::json!({ "type": "object", "additionalProperties": false });
        let rf = ResponseFormat {
            r#type: "json".into(),
            schema: Some(schema.clone()),
        };
        let wire = build_response_format(&rf, true);
        assert_eq!(wire["type"], "json_schema");
        assert_eq!(wire["json_schema"]["strict"], true);
        assert_eq!(wire["json_schema"]["schema"], schema);

        // model without structured_outputs → degrade to json_object
        let wire = build_response_format(&rf, false);
        assert_eq!(wire["type"], "json_object");
        assert!(wire.get("json_schema").is_none());

        // no schema at all → json_object regardless
        let rf = ResponseFormat {
            r#type: "json".into(),
            schema: None,
        };
        assert_eq!(build_response_format(&rf, true)["type"], "json_object");
    }

    #[test]
    fn headers_carry_bearer_auth_and_attribution() {
        let cfg = OpenRouterConfig {
            credential_value: "sk-test".into(),
            model: "anthropic/claude-x".into(),
            max_tokens: 4096,
            api_url: "https://openrouter.ai/api/v1/chat/completions".into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("content-type", "application/json".to_string())));
        assert!(h.contains(&("http-referer", ATTRIBUTION_REFERER.to_string())));
        assert!(h.contains(&("x-openrouter-title", ATTRIBUTION_TITLE.to_string())));
    }
}
