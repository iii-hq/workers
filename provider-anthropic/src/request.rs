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
pub fn build_body(args: &BodyArgs, warnings: &mut Vec<String>) -> Value {
    let mut wire_messages = to_wire_messages(&args.messages);
    // Claude 4.6+/Sonnet 5/Fable/Mythos reject any request whose final message
    // is an assistant turn — "Prefilling assistant messages is not supported
    // for this model" — with or without thinking. A trailing call-less
    // assistant here is always a dead partial from an aborted stream (a
    // tool_use turn always gets a tool_result/placeholder user flushed after
    // it), so drop it instead of failing the whole turn.
    let mut dropped_prefill = false;
    while wire_messages
        .last()
        .and_then(|m| m.get("role"))
        .and_then(Value::as_str)
        == Some("assistant")
    {
        wire_messages.pop();
        dropped_prefill = true;
    }
    if dropped_prefill {
        warnings.push(
            "trailing partial assistant message dropped: assistant prefill is not supported \
             (conversation must end with a user message)"
                .to_string(),
        );
    }
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
    if let Some(effort) = args.effort {
        body["output_config"] = json!({ "effort": effort });
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
        let body = build_body(&args(), &mut Vec::new());
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
        assert!(build_body(&a, &mut Vec::new()).get("system").is_none());
    }

    #[test]
    fn adaptive_thinking_and_effort_serialize_into_body() {
        let mut a = args();
        a.thinking = Some(crate::thinking::ADAPTIVE);
        a.effort = Some("xhigh");
        let body = build_body(&a, &mut Vec::new());
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "summarized");
        assert!(body["thinking"].get("budget_tokens").is_none());
        assert_eq!(body["output_config"]["effort"], "xhigh");
    }

    fn assistant_msg(content: Vec<ContentBlock>) -> AgentMessage {
        use llm_router::types::events::StopReason;
        use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "anthropic".into(),
            timestamp: 2,
        })
    }

    #[test]
    fn trailing_assistant_prefill_is_dropped_with_warning_and_thinking_kept() {
        // Claude 4.6+/Sonnet 5/Fable/Mythos 400 on assistant prefill with or
        // without thinking, so the dead partial tail (aborted-stream retry)
        // must be dropped — and thinking must survive.
        let mut a = args();
        a.thinking = Some(crate::thinking::ADAPTIVE);
        a.effort = Some("high");
        a.messages.push(assistant_msg(vec![ContentBlock::Text {
            text: "the answer is".into(),
        }]));
        let mut warnings = Vec::new();
        let body = build_body(&a, &mut warnings);
        assert_eq!(
            body["messages"].as_array().unwrap().last().unwrap()["role"],
            "user"
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(warnings.len(), 1, "one warning expected: {warnings:?}");
        assert!(warnings[0].contains("prefill"));
    }

    #[test]
    fn trailing_tool_use_assistant_survives_the_prefill_drop() {
        // A tool_use turn is always followed by a flushed tool_result /
        // placeholder user message, so the prefill drop never eats it and
        // signed-thinking-before-tool_use replay stays intact.
        let mut a = args();
        a.thinking = Some(crate::thinking::ADAPTIVE);
        a.messages.push(assistant_msg(vec![
            ContentBlock::Thinking {
                text: "plan".into(),
                signature: Some("sig".into()),
            },
            ContentBlock::FunctionCall {
                id: "t1".into(),
                function_id: "shell::exec".into(),
                arguments: json!({ "cmd": "ls" }),
            },
        ]));
        let mut warnings = Vec::new();
        let body = build_body(&a, &mut warnings);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.last().unwrap()["role"], "user");
        let assistant = &msgs[msgs.len() - 2];
        assert_eq!(assistant["role"], "assistant");
        assert_eq!(assistant["content"][0]["type"], "thinking");
        assert_eq!(assistant["content"][1]["type"], "tool_use");
        assert!(warnings.is_empty(), "no warning expected: {warnings:?}");
        assert_eq!(body["thinking"]["type"], "adaptive");
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
