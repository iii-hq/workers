//! OpenAI Responses request assembly: body (input items, tools, reasoning) +
//! headers (Bearer + ChatGPT account id + Codex originator).
use crate::config::{CodexBackendConfig, CodexConfig};
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use serde_json::{json, Value};

/// Codex client version whose Responses contract this worker mirrors.
///
/// The ChatGPT backend uses this header to gate newly released models. Omitting
/// it makes supported models such as GPT-5.6 Luna fail as "Model not found".
pub(crate) const CODEX_COMPAT_VERSION: &str = "0.144.1";

pub struct BodyArgs {
    pub model: String, // upstream model id
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    /// Pre-resolved reasoning effort; None omits the param.
    pub reasoning_effort: Option<&'static str>,
}

/// Responses body: `input` items, `stream`, `store:false` (stateless — the
/// full transcript is sent each turn), `max_output_tokens`, optional tools and
/// reasoning. No `temperature` (reasoning models reject non-default values).
pub fn build_body(args: &BodyArgs) -> Value {
    // NOTE: the Codex backend rejects `max_output_tokens` ("Unsupported
    // parameter") — verified live. The subscription backend caps output itself.
    let _ = args.max_tokens;
    let mut body = json!({
        "model": args.model,
        "input": to_wire_messages(&args.messages, &args.system_prompt),
        "stream": true,
        "store": false,
    });
    let wire_tools = functions_to_wire(&args.tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(effort) = args.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
    }
    body
}

/// Headers shared by every authenticated ChatGPT/Codex backend request.
pub fn build_backend_headers(cfg: &CodexBackendConfig) -> Vec<(&'static str, String)> {
    vec![
        ("authorization", format!("Bearer {}", cfg.access_token)),
        ("chatgpt-account-id", cfg.account_id.clone()),
        ("version", CODEX_COMPAT_VERSION.to_string()),
        ("originator", "codex_cli_rs".to_string()),
    ]
}

/// Headers for streaming Responses calls. `openai-beta` and SSE negotiation
/// are endpoint-specific; auth, account, version, and originator are shared
/// with model discovery.
pub fn build_headers(cfg: &CodexConfig) -> Vec<(&'static str, String)> {
    let backend = CodexBackendConfig {
        access_token: cfg.access_token.clone(),
        account_id: cfg.account_id.clone(),
        api_url: cfg.api_url.clone(),
    };
    let mut headers = build_backend_headers(&backend);
    headers.extend([
        ("openai-beta", "responses=experimental".to_string()),
        ("accept", "text/event-stream".to_string()),
        ("content-type", "application/json".to_string()),
    ]);
    headers
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn args() -> BodyArgs {
        BodyArgs {
            model: "gpt-5.5".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![AgentMessage::User(UserMessage {
                role: UserRoleTag::User,
                content: vec![ContentBlock::Text { text: "hi".into() }],
                timestamp: 1,
            })],
            tools: vec![],
            reasoning_effort: None,
        }
    }

    #[test]
    fn body_has_responses_shape() {
        let body = build_body(&args());
        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["stream"], true);
        assert_eq!(body["store"], false);
        assert!(
            body.get("max_output_tokens").is_none(),
            "backend rejects max_output_tokens"
        );
        assert!(
            body.get("messages").is_none(),
            "Responses uses input, not messages"
        );
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][1]["role"], "user");
        assert!(body.get("tools").is_none());
        assert!(body.get("reasoning").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn reasoning_and_tools_serialize_when_present() {
        let mut a = args();
        a.reasoning_effort = Some("high");
        a.tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let body = build_body(&a);
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["tools"][0]["name"], "agent__trigger");
    }

    #[test]
    fn headers_carry_codex_auth_set() {
        let cfg = CodexConfig {
            access_token: "at".into(),
            account_id: "acc-1".into(),
            model: "gpt-5.5".into(),
            max_tokens: 4096,
            api_url: crate::config::DEFAULT_API_URL.into(),
        };
        let h = build_headers(&cfg);
        assert!(h.contains(&("authorization", "Bearer at".to_string())));
        assert!(h.contains(&("chatgpt-account-id", "acc-1".to_string())));
        assert!(h.contains(&("version", CODEX_COMPAT_VERSION.to_string())));
        assert!(h.contains(&("originator", "codex_cli_rs".to_string())));
        assert!(h.iter().any(|(k, _)| *k == "openai-beta"));
        assert!(h.contains(&("accept", "text/event-stream".to_string())));
    }

    #[test]
    fn backend_headers_omit_stream_only_fields() {
        let cfg = CodexBackendConfig {
            access_token: "at".into(),
            account_id: "acc-1".into(),
            api_url: crate::config::DEFAULT_API_URL.into(),
        };
        let h = build_backend_headers(&cfg);
        assert!(h.contains(&("version", CODEX_COMPAT_VERSION.to_string())));
        assert!(!h.iter().any(|(key, _)| *key == "openai-beta"));
        assert!(!h.iter().any(|(key, _)| *key == "accept"));
    }
}
