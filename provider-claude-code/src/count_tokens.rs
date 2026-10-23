//! `provider::claude-code::count_tokens` — exact prompt token counting through
//! the upstream count_tokens metering endpoint (the sibling of the configured
//! messages endpoint), authenticated with the Claude Code subscription OAuth
//! token exactly as the stream path is: same credential lookup, same headers
//! (`authorization: Bearer`, `anthropic-beta: oauth-2025-04-20`), and the same
//! Claude Code identity block first in `system`. The request is assembled with
//! the SAME wire mappers the stream path uses, so the count matches what a
//! real turn would send; the endpoint meters without generating, so it never
//! runs the model and costs nothing. Exposed behind `router::count_tokens`.

use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{build_config, ClaudeCodeConfig};
use crate::errors::{classify_bus_error, invalid_request};
use crate::request::build_headers;
use crate::stream_fn::default_resolve;
use crate::wire::cache::build_system_field;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use crate::{auth, state};

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
    /// Model id the prompt targets (required by the upstream endpoint); a
    /// namespaced router id (`claude-code/<id>`) is mapped to the upstream id.
    pub model: String,
    /// System prompt counted as the second wire `system` block when present
    /// (the first is always the Claude Code identity line).
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; mapped to the wire `tools` array.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::claude-code::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    /// Upstream model id the count was computed for.
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

/// The count body: `{model, system, tools?, messages}` — no `max_tokens`, no
/// `stream`, and no cache markers (`cache_enabled=false`), because a count
/// must never write a cache entry. `system` is always present: block 0 is the
/// Claude Code identity line the subscription backend requires, exactly as on
/// the stream path.
fn build_count_body(
    model: &str,
    system_prompt: &str,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
) -> Value {
    let mut body = json!({
        "model": model,
        "messages": to_wire_messages(messages),
        "system": build_system_field(system_prompt, false),
    });
    let wire_tools = functions_to_wire(tools);
    if !wire_tools.is_empty() {
        body["tools"] = Value::Array(wire_tools);
    }
    body
}

/// POST the count body with the OAuth headers the stream path sends. A non-2xx
/// answer (including an endpoint that rejects the token or does not exist) is
/// an error, never a panic, so the router can still fall back to an estimate.
async fn post_count(
    http: &reqwest::Client,
    cfg: &ClaudeCodeConfig,
    body: &Value,
) -> Result<u64, Error> {
    let mut request = http
        .post(count_tokens_url(&cfg.api_url))
        .timeout(Duration::from_secs(COUNT_TOKENS_TIMEOUT_SECS));
    for (name, value) in build_headers(cfg) {
        request = request.header(name, value);
    }
    let response = request
        .json(body)
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
    Ok(wire.input_tokens)
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
        return Err(invalid_request("count_tokens: messages must not be empty"));
    }

    // Same credential path as `stream_fn`: the cached resolve only carries
    // api_url/max_tokens, and a missing router degrades to defaults so the
    // ~/.claude dev fallback still works. The vault credential lookup is NOT
    // cached — the vault refreshes expiring OAuth tokens on its own resolve.
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let resolved = match cache
        .resolve(iii, crate::PROVIDER_ID, token.as_deref(), None)
        .await
    {
        Ok(r) => r,
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            default_resolve()
        }
    };
    let credential = auth::fetch_fresh_credential(iii).await;
    let cfg = build_config(&req.model, None, &resolved, credential.as_ref())
        .map_err(|e| Error::Handler(e.to_string()))?;

    let body = build_count_body(
        &cfg.model,
        req.system_prompt.as_deref().unwrap_or(""),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    );
    let tokens = post_count(http, &cfg, &body).await?;

    Ok(CountTokensResponse {
        model: cfg.model,
        tokens,
        estimator: ESTIMATOR_PROVIDER.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{ANTHROPIC_VERSION, OAUTH_BETA};
    use crate::wire::cache::{CACHE_MIN_CHARS, CLAUDE_CODE_SYSTEM};
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 1,
        })
    }

    fn tool() -> AgentFunction {
        AgentFunction {
            name: "agent::trigger".into(),
            description: "Invoke an iii function".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }
    }

    fn cfg(api_url: String) -> ClaudeCodeConfig {
        ClaudeCodeConfig {
            credential_value: "sk-ant-oat01-test".into(),
            model: "claude-sonnet-4-6".into(),
            max_tokens: 4096,
            api_url,
        }
    }

    /// One-shot HTTP stub: answers `response` and hands back the raw request.
    async fn stub(response: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = vec![0u8; 65536];
            let n = sock.read(&mut buf).await.unwrap_or(0);
            let _ = sock.write_all(response.as_bytes()).await;
            let _ = sock.shutdown().await;
            String::from_utf8_lossy(&buf[..n]).into_owned()
        });
        (format!("http://{addr}/v1/messages"), handle)
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
    fn body_has_no_max_tokens_no_stream_and_the_identity_block_first() {
        let body = build_count_body("claude-sonnet-4-6", "be brief", &[], &[user("hi")]);
        assert_eq!(body["model"], "claude-sonnet-4-6");
        assert_eq!(body["messages"][0]["role"], "user");
        let system = body["system"].as_array().expect("system is an array");
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM);
        assert_eq!(system[1]["text"], "be brief");
        assert!(body.get("max_tokens").is_none());
        assert!(body.get("stream").is_none());
        assert!(body.get("tools").is_none(), "empty tools array is omitted");
    }

    #[test]
    fn empty_system_prompt_keeps_only_the_identity_block_and_tools_map_to_wire() {
        let body = build_count_body("m", "", &[tool()], &[user("hi")]);
        let system = body["system"].as_array().expect("system is an array");
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM);
        assert_eq!(body["tools"][0]["name"], "agent__trigger");
    }

    #[test]
    fn body_never_carries_cache_markers() {
        let long = "p".repeat(CACHE_MIN_CHARS * 2);
        let body = build_count_body("m", &long, &[tool()], &[user(&long)]);
        assert!(
            !body.to_string().contains("cache_control"),
            "a count must never write a cache entry: {body}"
        );
    }

    #[test]
    fn wire_count_response_parses_input_tokens() {
        let wire: WireCountResponse = serde_json::from_str(r#"{"input_tokens": 2095}"#).unwrap();
        assert_eq!(wire.input_tokens, 2095);
    }

    #[tokio::test]
    async fn post_count_sends_oauth_headers_to_the_count_endpoint() {
        let (url, request) = stub(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 20\r\nconnection: close\r\n\r\n{\"input_tokens\":42}\n",
        )
        .await;
        let body = build_count_body("claude-sonnet-4-6", "", &[], &[user("hi")]);
        let tokens = post_count(&reqwest::Client::new(), &cfg(url), &body)
            .await
            .expect("count succeeds");
        assert_eq!(tokens, 42);

        let raw = request.await.unwrap().to_lowercase();
        assert!(raw.starts_with("post /v1/messages/count_tokens "), "{raw}");
        assert!(
            raw.contains("authorization: bearer sk-ant-oat01-test"),
            "{raw}"
        );
        assert!(
            raw.contains(&format!("anthropic-beta: {OAUTH_BETA}")),
            "{raw}"
        );
        assert!(
            raw.contains(&format!("anthropic-version: {ANTHROPIC_VERSION}")),
            "{raw}"
        );
        assert!(!raw.contains("x-api-key"), "{raw}");
    }

    #[tokio::test]
    async fn post_count_turns_a_non_2xx_answer_into_an_error() {
        let (url, _request) = stub(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: 50\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"authentication\"}}",
        )
        .await;
        let body = build_count_body("claude-sonnet-4-6", "", &[], &[user("hi")]);
        let err = post_count(&reqwest::Client::new(), &cfg(url), &body)
            .await
            .expect_err("401 is an error");
        let msg = err.to_string();
        assert!(msg.contains("provider/upstream_status: 401"), "{msg}");
    }
}
