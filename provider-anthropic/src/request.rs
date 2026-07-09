//! Full Messages API request assembly: body (messages, tools, system,
//! thinking, cache markers) + headers (auth mode, version, beta).
use crate::config::{AnthropicConfig, AuthMode};
use crate::thinking::ThinkingConfig;
use crate::wire::cache::{
    apply_messages_cache_anchor, apply_tools_cache_control, build_system_field,
};
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use serde_json::{json, Value};

pub const ANTHROPIC_VERSION: &str = "2023-06-01";

pub struct BodyArgs {
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub thinking: Option<ThinkingConfig>,
    /// `output_config.effort` for the adaptive-thinking generation.
    pub effort: Option<&'static str>,
    pub cache_enabled: bool,
}

/// No `temperature`: the API default applies (required when thinking is on).
pub fn build_body(args: &BodyArgs) -> Value {
    let mut wire_messages = to_wire_messages(&args.messages);
    apply_messages_cache_anchor(&mut wire_messages, args.cache_enabled);
    let mut wire_tools = functions_to_wire(&args.tools);
    apply_tools_cache_control(&mut wire_tools, args.cache_enabled);

    let mut body = json!({
        "model": args.model,
        "max_tokens": args.max_tokens,
        "messages": wire_messages,
        "tools": wire_tools,
        "stream": true,
    });
    if let Some(system) = build_system_field(&args.system_prompt, args.cache_enabled) {
        body["system"] = system;
    }
    // Thinking and assistant prefill are mutually exclusive on the Messages API
    // ("...does not support assistant message prefill. The conversation must end
    // with a user message."). If the sanitized transcript still ends on an
    // assistant turn, honor the prefill and drop thinking rather than 400.
    let is_prefill = body["messages"]
        .as_array()
        .and_then(|m| m.last())
        .and_then(|m| m.get("role"))
        .and_then(Value::as_str)
        == Some("assistant");
    if !is_prefill {
        if let Some(t) = &args.thinking {
            body["thinking"] = serde_json::to_value(t).expect("serializable thinking config");
        }
        if let Some(effort) = args.effort {
            body["output_config"] = json!({ "effort": effort });
        }
    }
    body
}

/// The auth header for a given mode; streaming and discovery share it.
pub fn auth_header(auth_mode: AuthMode, credential_value: &str) -> (&'static str, String) {
    match auth_mode {
        AuthMode::ApiKey => ("x-api-key", credential_value.to_string()),
        AuthMode::OauthBearer => ("authorization", format!("Bearer {credential_value}")),
    }
}

/// No thinking beta header: adaptive thinking interleaves natively.
pub fn build_headers(cfg: &AnthropicConfig) -> Vec<(&'static str, String)> {
    vec![
        auth_header(cfg.auth_mode, &cfg.credential_value),
        ("anthropic-version", ANTHROPIC_VERSION.to_string()),
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
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            thinking: None,
            effort: None,
            cache_enabled: false,
        }
    }

    fn cfg(auth_mode: AuthMode) -> AnthropicConfig {
        AnthropicConfig {
            credential_value: "sk-test".into(),
            auth_mode,
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            api_url: "https://api.anthropic.com/v1/messages".into(),
        }
    }

    #[test]
    fn body_has_required_fields_and_stream_true() {
        let body = build_body(&args());
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["max_tokens"], 4096);
        assert_eq!(body["stream"], true);
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(body.get("thinking").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn empty_system_prompt_omits_the_field() {
        let mut a = args();
        a.system_prompt = String::new();
        assert!(build_body(&a).get("system").is_none());
    }

    #[test]
    fn adaptive_thinking_and_effort_serialize_into_body() {
        let mut a = args();
        a.thinking = Some(crate::thinking::ADAPTIVE);
        a.effort = Some("xhigh");
        let body = build_body(&a);
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "summarized");
        assert!(body["thinking"].get("budget_tokens").is_none());
        assert_eq!(body["output_config"]["effort"], "xhigh");
    }

    #[test]
    fn thinking_dropped_when_conversation_ends_on_assistant_prefill() {
        use llm_router::types::events::StopReason;
        use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
        let mut a = args();
        a.thinking = Some(crate::thinking::ADAPTIVE);
        a.effort = Some("high");
        // A trailing call-less assistant (aborted-stream partial or an
        // intentional prefill) makes the wire end on role:assistant. Thinking +
        // prefill are mutually exclusive, so thinking must be dropped, not 400.
        a.messages.push(AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content: vec![ContentBlock::Text {
                text: "the answer is".into(),
            }],
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "anthropic".into(),
            timestamp: 2,
        }));
        let body = build_body(&a);
        assert_eq!(
            body["messages"].as_array().unwrap().last().unwrap()["role"],
            "assistant"
        );
        assert!(
            body.get("thinking").is_none(),
            "prefill request must not carry thinking"
        );
        assert!(body.get("output_config").is_none());
    }

    #[test]
    fn headers_per_auth_mode() {
        let h = build_headers(&cfg(AuthMode::ApiKey));
        assert!(h.contains(&("x-api-key", "sk-test".to_string())));
        assert!(h.contains(&("anthropic-version", ANTHROPIC_VERSION.to_string())));
        assert!(!h.iter().any(|(k, _)| *k == "anthropic-beta"));

        let h = build_headers(&cfg(AuthMode::OauthBearer));
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
    }
}
