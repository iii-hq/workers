//! `provider::anthropic::count_tokens` — exact prompt token counting through
//! the upstream count_tokens metering endpoint (the sibling of the configured
//! messages endpoint). The request is assembled with the SAME wire mappers
//! the stream path uses, so the count matches what a real turn would send;
//! the endpoint meters without generating, so it never runs the model and
//! costs nothing. Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::build_headers;
use crate::state;
use crate::wire::cache::build_system_field;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

/// A count is bounded and non-streaming; a tight budget keeps this inside
/// the router's 30s count_tokens bus timeout.
const COUNT_TOKENS_TIMEOUT_SECS: u64 = 20;

/// The count returned came from the provider's metering API.
const ESTIMATOR_PROVIDER: &str = "provider";

/// Counting endpoint for the configured messages `api_url`: the
/// `/count_tokens` child of the same path (`…/v1/messages` →
/// `…/v1/messages/count_tokens`), the way discovery derives its models
/// sibling. Deriving from the configured url keeps proxies and gateways on
/// the right host.
fn count_tokens_url(api_url: &str) -> String {
    format!("{}/count_tokens", api_url.trim_end_matches('/'))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets (required by the upstream endpoint).
    pub model: String,
    /// System prompt counted as the wire `system` field when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; mapped to the wire `tools` array.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::anthropic::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens the upstream endpoint counted for the assembled request.
    pub tokens: u64,
    /// Always `provider`: the count came from the upstream metering API.
    pub estimator: String,
}

#[derive(Debug, Deserialize)]
struct WireCountResponse {
    input_tokens: u64,
}

/// The count body: `{model, system?, tools?, messages}` — no `max_tokens`,
/// no `stream`, and no cache markers (`cache_enabled=false`), because a count
/// must never write a cache entry.
fn build_count_body(
    model: &str,
    system_prompt: &str,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": to_wire_messages(messages),
    });
    let wire_tools = functions_to_wire(tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    if let Some(system) = build_system_field(system_prompt, false) {
        body["system"] = system;
    }
    body
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }

    // Token + resolve are cached (ScaffoldCache): zero engine round trips
    // on the hot path within the TTL. An auth-classified resolve failure
    // drops the cache so the next attempt re-resolves fresh — retrying
    // stays the router's job.
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(
            iii,
            crate::PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            return Err(e);
        }
    };
    let cfg = config_from_resolve(&req.model, None, &resolved)
        .map_err(|e| Error::Handler(e.to_string()))?;

    let body = build_count_body(
        &cfg.model,
        req.system_prompt.as_deref().unwrap_or(""),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    let mut request = http
        .post(count_tokens_url(&cfg.api_url))
        .timeout(std::time::Duration::from_secs(COUNT_TOKENS_TIMEOUT_SECS));
    for (name, value) in build_headers(&cfg) {
        request = request.header(name, value);
    }
    let response = request
        .json(&body)
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(300).collect();
        return Err(Error::Handler(format!(
            "provider/upstream_status: {status}: {excerpt}"
        )));
    }
    let wire: WireCountResponse = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;

    Ok(CountTokensResponse {
        model: cfg.model,
        tokens: wire.input_tokens,
        estimator: ESTIMATOR_PROVIDER.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    #[test]
    fn count_tokens_url_is_the_messages_sibling() {
        assert_eq!(
            count_tokens_url("https://api.anthropic.com/v1/messages"),
            "https://api.anthropic.com/v1/messages/count_tokens"
        );
        assert_eq!(
            count_tokens_url("http://127.0.0.1:9999/v1/messages/"),
            "http://127.0.0.1:9999/v1/messages/count_tokens"
        );
    }

    #[test]
    fn body_has_no_max_tokens_and_no_stream() {
        let body = build_count_body("claude-sonnet-4-6", "be brief", &[], &[user("hi")]);
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["system"], "be brief");
        assert_eq!(body["messages"][0]["role"], "user");
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("stream").is_none());
        assert!(body.get("tools").is_none(), "empty tools array is omitted");
    }

    #[test]
    fn empty_system_prompt_omits_the_field_and_tools_map_to_wire() {
        let tools = vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let body = build_count_body("m", "", &tools, &[user("hi")]);
        assert!(body.get("system").is_none());
        assert_eq!(body["tools"][0]["name"], "agent__trigger");
    }

    #[test]
    fn wire_count_response_parses_input_tokens() {
        let wire: WireCountResponse = serde_json::from_str(r#"{"input_tokens": 2095}"#).unwrap();
        assert_eq!(wire.input_tokens, 2095);
    }
}
