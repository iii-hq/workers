//! OpenAI Responses request assembly: body (input items, tools, reasoning) +
//! headers (Bearer + ChatGPT account id + Codex originator).
use crate::config::{CodexBackendConfig, CodexConfig};
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

/// Codex client version whose Responses contract this worker mirrors.
///
/// The ChatGPT backend uses this header to gate newly released models. Omitting
/// it makes supported models such as GPT-5.6 Luna fail as "Model not found".
pub(crate) const CODEX_COMPAT_VERSION: &str = "0.144.1";

const SUMMARY_UNSUPPORTED_MODEL: &str = "gpt-5.3-codex-spark";

pub struct BodyArgs {
    pub model: String, // upstream model id
    pub max_tokens: u64,
    pub system_prompt: String,
    pub messages: Vec<AgentMessage>,
    pub tools: Vec<AgentFunction>,
    /// Resolved catalog capability, including the model-name fallback.
    pub supports_thinking: bool,
    /// Pre-resolved reasoning effort; None omits the param.
    pub reasoning_effort: Option<String>,
    /// Cache-routing key for the Responses backend; None omits the field.
    pub prompt_cache_key: Option<String>,
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
    let mut reasoning = serde_json::Map::new();
    if args.supports_thinking && args.model != SUMMARY_UNSUPPORTED_MODEL {
        reasoning.insert("summary".into(), json!("auto"));
    }
    if let Some(effort) = &args.reasoning_effort {
        reasoning.insert("effort".into(), json!(effort));
    }
    if !reasoning.is_empty() {
        body["reasoning"] = Value::Object(reasoning);
    }
    if let Some(key) = &args.prompt_cache_key {
        body["prompt_cache_key"] = json!(key);
    }
    body
}

/// Header-safe UUID derived from a stable conversation identity.
pub fn derive_affinity_id(conversation_id: &str) -> Option<String> {
    if conversation_id.trim().is_empty() {
        return None;
    }
    let digest = Sha256::digest(conversation_id.as_bytes());
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;
    Some(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9], b[10], b[11], b[12], b[13],
        b[14], b[15]
    ))
}

/// Resolve header affinity and body cache keys independently. The durable
/// session id wins; direct router callers fall back to per-request affinity.
/// A caller's `prompt_cache_key` override affects only the JSON body.
pub fn resolve_cache_routing(
    provider_options: Option<&Value>,
    session_id: Option<&str>,
    resolution_key: Option<&str>,
) -> (Vec<(&'static str, String)>, Option<String>) {
    let affinity_id = session_id
        .filter(|id| !id.trim().is_empty())
        .or_else(|| resolution_key.filter(|id| !id.trim().is_empty()))
        .and_then(derive_affinity_id);
    let prompt_cache_key = provider_options
        .and_then(|options| options.get("prompt_cache_key"))
        .and_then(Value::as_str)
        .filter(|key| !key.trim().is_empty())
        .map(str::to_string)
        .or_else(|| affinity_id.clone());
    let headers = affinity_id
        .as_deref()
        .map(session_affinity_headers)
        .unwrap_or_default();
    (headers, prompt_cache_key)
}

/// Conversation-affinity headers, mirroring upstream Codex CLI's
/// `ResponsesClient::stream_request` (`session-id`, `thread-id`, and
/// `x-client-request-id`, all carrying the conversation UUID; our
/// single-threaded conversations use one value for all three). The ChatGPT
/// codex backend's cache affinity follows these headers;
/// the body `prompt_cache_key` alone showed luck-rate hits in live sessions.
/// Callers pass only derived UUIDs, never arbitrary body cache-key overrides.
pub fn session_affinity_headers(key: &str) -> Vec<(&'static str, String)> {
    vec![
        ("session-id", key.to_string()),
        ("thread-id", key.to_string()),
        ("x-client-request-id", key.to_string()),
    ]
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
            supports_thinking: true,
            reasoning_effort: None,
            prompt_cache_key: None,
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
        assert_eq!(body["reasoning"]["summary"], "auto");
        assert!(body["reasoning"].get("effort").is_none());
        assert!(body.get("temperature").is_none());
    }

    #[test]
    fn reasoning_and_tools_serialize_when_present() {
        let mut a = args();
        a.reasoning_effort = Some("high".into());
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
    fn spark_omits_the_unsupported_reasoning_summary() {
        let mut a = args();
        a.model = "gpt-5.3-codex-spark".into();
        a.reasoning_effort = Some("high".into());
        let body = build_body(&a);
        assert!(body["reasoning"].get("summary").is_none());
        assert_eq!(body["reasoning"]["effort"], "high");
    }

    #[test]
    fn non_prefix_reasoning_model_includes_summary() {
        let mut a = args();
        a.model = "catalog-reasoner".into();
        a.reasoning_effort = Some("high".into());
        let body = build_body(&a);
        assert_eq!(body["reasoning"]["summary"], "auto");
    }

    #[test]
    fn catalog_capability_false_omits_summary_for_prefix_model() {
        let mut a = args();
        a.supports_thinking = false;
        a.reasoning_effort = Some("high".into());
        let body = build_body(&a);
        assert!(body["reasoning"].get("summary").is_none());
        assert_eq!(body["reasoning"]["effort"], "high");
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

    /// RFC 4122 v4 shape: 36 chars, hyphens at 8/13/18/23, version nibble
    /// `4`, variant nibble in `[89ab]` — what the backend's UUID parsers
    /// accept anywhere a session id is expected.
    fn assert_uuid_format(key: &str) {
        assert_eq!(key.len(), 36, "not UUID-length: {key}");
        for idx in [8, 13, 18, 23] {
            assert_eq!(key.as_bytes()[idx], b'-', "hyphen missing at {idx}: {key}");
        }
        assert!(
            key.chars()
                .enumerate()
                .all(|(i, c)| matches!(i, 8 | 13 | 18 | 23) || c.is_ascii_hexdigit()),
            "non-hex char: {key}"
        );
        assert_eq!(key.as_bytes()[14], b'4', "version nibble: {key}");
        assert!(
            matches!(key.as_bytes()[19], b'8' | b'9' | b'a' | b'b'),
            "variant nibble: {key}"
        );
    }

    #[test]
    fn affinity_headers_carry_every_codex_routing_id() {
        let headers = session_affinity_headers("11112222-3333-4444-8555-666677778888");
        assert_eq!(
            headers,
            vec![
                (
                    "session-id",
                    "11112222-3333-4444-8555-666677778888".to_string()
                ),
                (
                    "thread-id",
                    "11112222-3333-4444-8555-666677778888".to_string()
                ),
                (
                    "x-client-request-id",
                    "11112222-3333-4444-8555-666677778888".to_string()
                ),
            ]
        );
    }

    #[test]
    fn affinity_id_is_stable_for_the_session() {
        let one = derive_affinity_id("s_conversation").unwrap();
        let two = derive_affinity_id("s_conversation").unwrap();
        assert_uuid_format(&one);
        assert_eq!(one, two);
        assert_ne!(one, derive_affinity_id("s_other").unwrap());
    }

    #[test]
    fn cache_routing_follows_the_session_across_turns() {
        let before = resolve_cache_routing(None, Some("s_conversation"), Some("turn_1"));
        let after = resolve_cache_routing(None, Some("s_conversation"), Some("turn_2"));
        assert_eq!(before, after);
        assert_eq!(
            before.0[0].1,
            before.1.unwrap(),
            "the default body key matches affinity"
        );
    }

    #[test]
    fn prompt_cache_override_never_becomes_a_session_header() {
        let provider_options = json!({ "prompt_cache_key": "shared\ncache" });
        let (headers, prompt_cache_key) = resolve_cache_routing(
            Some(&provider_options),
            Some("s_conversation"),
            Some("turn_1"),
        );
        assert_eq!(prompt_cache_key.as_deref(), Some("shared\ncache"));
        assert_eq!(headers.len(), 3);
        assert!(headers.iter().all(|(_, value)| !value.contains('\n')));
    }

    #[test]
    fn body_carries_prompt_cache_key() {
        let mut a = args();
        a.prompt_cache_key = derive_affinity_id("s_conversation");
        let body = build_body(&a);
        let key = body["prompt_cache_key"].as_str().unwrap();
        assert_uuid_format(key);
    }

    #[test]
    fn missing_routing_identity_omits_the_default_key() {
        assert_eq!(resolve_cache_routing(None, None, None), (vec![], None));
        let mut a = args();
        a.prompt_cache_key = None;
        let body = build_body(&a);
        assert!(body.get("prompt_cache_key").is_none());
    }

    #[test]
    fn invalid_override_falls_back_to_affinity_key() {
        let provider_options = json!({ "prompt_cache_key": 12345 });
        let (headers, prompt_cache_key) =
            resolve_cache_routing(Some(&provider_options), Some("s_conversation"), None);
        assert_eq!(prompt_cache_key.as_deref(), Some(headers[0].1.as_str()));
    }

    #[test]
    fn blank_override_falls_back_to_affinity_key() {
        for blank in ["", "   "] {
            let provider_options = json!({ "prompt_cache_key": blank });
            let (headers, prompt_cache_key) =
                resolve_cache_routing(Some(&provider_options), Some("s_conversation"), None);
            assert_eq!(
                prompt_cache_key.as_deref(),
                Some(headers[0].1.as_str()),
                "blank override {blank:?}"
            );
        }
    }
}
