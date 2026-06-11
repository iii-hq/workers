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
/// Lets thinking interleave with tool calls.
pub const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";

pub struct BodyArgs {
    pub model: String,
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    pub thinking: Option<ThinkingConfig>,
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
    if let Some(t) = &args.thinking {
        body["thinking"] = serde_json::to_value(t).expect("serializable thinking config");
    }
    body
}

pub fn build_headers(cfg: &AnthropicConfig, thinking: bool) -> Vec<(&'static str, String)> {
    let mut headers = vec![
        match cfg.auth_mode {
            AuthMode::ApiKey => ("x-api-key", cfg.credential_value.clone()),
            AuthMode::OauthBearer => ("authorization", format!("Bearer {}", cfg.credential_value)),
        },
        ("anthropic-version", ANTHROPIC_VERSION.to_string()),
        ("content-type", "application/json".to_string()),
    ];
    if thinking {
        headers.push(("anthropic-beta", INTERLEAVED_THINKING_BETA.to_string()));
    }
    headers
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
    fn thinking_config_serializes_into_body() {
        let mut a = args();
        a.thinking = Some(ThinkingConfig {
            mode: "enabled",
            budget_tokens: 2048,
        });
        let body = build_body(&a);
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 2048);
    }

    #[test]
    fn headers_per_auth_mode_and_beta_when_thinking() {
        let h = build_headers(&cfg(AuthMode::ApiKey), false);
        assert!(h.contains(&("x-api-key", "sk-test".to_string())));
        assert!(h.contains(&("anthropic-version", ANTHROPIC_VERSION.to_string())));
        assert!(!h.iter().any(|(k, _)| *k == "anthropic-beta"));

        let h = build_headers(&cfg(AuthMode::OauthBearer), true);
        assert!(h.contains(&("authorization", "Bearer sk-test".to_string())));
        assert!(h.contains(&("anthropic-beta", INTERLEAVED_THINKING_BETA.to_string())));
    }
}
