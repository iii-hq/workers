//! `provider::claude-code::count_tokens` — exact prompt token counting through
//! the upstream count_tokens metering endpoint (the sibling of the configured
//! messages endpoint), authenticated with the Claude Code subscription OAuth
//! token exactly as the stream path is: same credential lookup, same headers
//! (`authorization: Bearer`, `anthropic-beta: oauth-2025-04-20`), and the same
//! Claude Code identity block first in `system`. The request is assembled with
//! the SAME wire mappers the stream path uses, so the count matches what a
//! real turn would send; the endpoint meters without generating, so it never
//! runs the model and costs nothing. Exposed behind `router::count_tokens`.
//!
//! A count has no turn: while an account source holds the accounts, it uses
//! the global account's token, and a 401 asks the source once more.

use std::time::Duration;

use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::{AgentFunction, Model, ThinkingLevel};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::accounts::{self, AccountSource, AccountToken, Credential};
use crate::config::{ClaudeCodeConfig, build_config};
use crate::errors::{classify_bus_error, invalid_request};
use crate::request::{BodyArgs, build_body, build_headers};
use crate::stream_fn::default_resolve;
use crate::thinking::build_thinking_config;
use crate::{auth, router_client, state};

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
    /// Reasoning level the turn would stream with; mapped to the same
    /// `thinking` / `output_config.effort` fields, which Anthropic renders
    /// into the prompt. Absent behaves like the stream's absent level.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
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

/// The count body is the stream body (`request::build_body`: thinking config,
/// effort, the trailing-assistant drop, the Claude Code identity block first
/// in `system`) minus `stream` and `max_tokens`, with cache markers off
/// because a count must never write a cache entry. Anthropic renders the
/// thinking config and effort into the prompt, so leaving them out would
/// meter a different prompt than the turn bills. The level is mapped here,
/// with the stream's `build_thinking_config`, so `handle` cannot skip it:
/// with no level, adaptive thinking is still sent on thinking models.
fn build_count_body(
    model: String,
    system_prompt: String,
    tools: Vec<AgentFunction>,
    messages: Vec<AgentMessage>,
    thinking_level: Option<ThinkingLevel>,
    model_meta: Option<&Model>,
) -> Value {
    let thinking = build_thinking_config(thinking_level, model_meta);
    let mut body = build_body(
        &BodyArgs {
            model,
            max_tokens: 0,
            system_prompt,
            system_sections: None,
            messages,
            tools,
            thinking: thinking.config,
            effort: thinking.effort,
            cache_enabled: false,
            cache_ttl: None,
            messages_cache_ttl: None,
        },
        // The stream reports these on the turn; a count has no channel for them.
        &mut Vec::new(),
    );
    let obj = body.as_object_mut().expect("build_body returns an object");
    obj.remove("stream");
    obj.remove("max_tokens");
    if obj
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(Vec::is_empty)
    {
        obj.remove("tools");
    }
    body
}

/// A count that failed, with the upstream status when there was one.
#[derive(Debug)]
struct CountFailure {
    status: Option<u16>,
    error: Error,
}

impl CountFailure {
    fn new(status: Option<u16>, error: Error) -> Self {
        Self { status, error }
    }
}

/// POST the count body with the OAuth headers the stream path sends. A non-2xx
/// answer (including an endpoint that rejects the token or does not exist) is
/// an error, never a panic, so the router can still fall back to an estimate.
async fn post_count(
    http: &reqwest::Client,
    cfg: &ClaudeCodeConfig,
    body: &Value,
) -> Result<u64, CountFailure> {
    let mut request = http
        .post(count_tokens_url(&cfg.api_url))
        .timeout(Duration::from_secs(COUNT_TOKENS_TIMEOUT_SECS));
    for (name, value) in build_headers(cfg) {
        request = request.header(name, value);
    }
    let response =
        request.json(body).send().await.map_err(|e| {
            CountFailure::new(None, Error::Handler(format!("provider/upstream: {e}")))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(300).collect();
        return Err(CountFailure::new(
            Some(status.as_u16()),
            Error::Handler(format!("provider/upstream_status: {status}: {excerpt}")),
        ));
    }
    let wire: WireCountResponse = response.json().await.map_err(|e| {
        CountFailure::new(None, Error::Handler(format!("provider/bad_response: {e}")))
    })?;
    Ok(wire.input_tokens)
}

/// One count; with an account source's token, one more after a 401 with the
/// token the source gives when asked again (D140). A token refused twice is
/// dropped from the source's cache.
async fn post_count_for_account(
    http: &reqwest::Client,
    cfg: &mut ClaudeCodeConfig,
    body: &Value,
    account: Option<&AccountToken>,
    accounts: &dyn AccountSource,
) -> Result<u64, Error> {
    match post_count(http, cfg, body).await {
        Err(failure) if failure.status == Some(401) => {
            let Some(refused) = account else {
                return Err(failure.error);
            };
            let Some(fresh) = accounts::ask_again(accounts, refused).await else {
                return Err(failure.error);
            };
            cfg.credential_value = fresh.access_token;
            post_count(http, cfg, body).await.map_err(|failure| {
                if failure.status == Some(401) {
                    accounts.invalidate(&fresh.account);
                }
                failure.error
            })
        }
        other => other.map_err(|f| f.error),
    }
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    accounts: &dyn AccountSource,
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
    // The global account's token while an account source holds the accounts.
    let (credential, account) =
        match accounts::credential(accounts, None, || auth::fetch_fresh_credential(iii))
            .await
            .map_err(|e| Error::Handler(e.message()))?
        {
            Credential::Account(token) => (Some(token.credential()), Some(token)),
            Credential::Legacy(credential) => (credential, None),
        };
    let mut cfg = build_config(&req.model, None, &resolved, credential.as_ref())
        .map_err(|e| Error::Handler(e.to_string()))?;

    // Same model_meta lookup as the stream feeds its thinking mapping.
    let model_meta = router_client::models_get(iii, &req.model).await;
    let body = build_count_body(
        cfg.model.clone(),
        req.system_prompt.unwrap_or_default(),
        req.tools.unwrap_or_default(),
        req.messages,
        req.thinking_level,
        model_meta.as_ref(),
    );
    let tokens = post_count_for_account(http, &mut cfg, &body, account.as_ref(), accounts).await?;

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
    use serde_json::json;
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
            name: "agent::call".into(),
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
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            "be brief".into(),
            vec![],
            vec![user("hi")],
            None,
            None,
        );
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
        let body = build_count_body(
            "m".into(),
            String::new(),
            vec![tool()],
            vec![user("hi")],
            None,
            None,
        );
        let system = body["system"].as_array().expect("system is an array");
        assert_eq!(system.len(), 1);
        assert_eq!(system[0]["text"], CLAUDE_CODE_SYSTEM);
        assert_eq!(body["tools"][0]["name"], "agent__call");
    }

    #[test]
    fn body_never_carries_cache_markers() {
        let long = "p".repeat(CACHE_MIN_CHARS * 2);
        let body = build_count_body(
            "m".into(),
            long.clone(),
            vec![tool()],
            vec![user(&long)],
            None,
            None,
        );
        assert!(
            !body.to_string().contains("cache_control"),
            "a count must never write a cache entry: {body}"
        );
    }

    fn assistant(content: Vec<ContentBlock>) -> AgentMessage {
        use llm_router::types::events::StopReason;
        use llm_router::types::messages::{AssistantMessage, AssistantRoleTag};
        AgentMessage::Assistant(AssistantMessage {
            role: AssistantRoleTag::Assistant,
            content,
            stop_reason: StopReason::End,
            native_stop_reason: None,
            error_message: None,
            error_kind: None,
            error_details: None,
            warnings: None,
            usage: None,
            model: "m".into(),
            provider: "claude-code".into(),
            timestamp: 2,
        })
    }

    #[test]
    fn body_carries_the_streams_thinking_config_and_effort() {
        // Anthropic renders the thinking config and effort into the prompt,
        // so a count without them meters a different prompt than the turn.
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            String::new(),
            vec![],
            vec![user("hi")],
            Some(ThinkingLevel::High),
            None,
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "summarized");
        assert_eq!(body["output_config"]["effort"], "high");
    }

    #[test]
    fn body_drops_a_trailing_assistant_like_the_stream() {
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            String::new(),
            vec![],
            vec![
                user("hi"),
                assistant(vec![ContentBlock::Text {
                    text: "the answer is".into(),
                }]),
            ],
            None,
            None,
        );
        let messages = body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1, "{body}");
        assert_eq!(messages[0]["role"], "user");
    }

    fn thinking_model() -> Model {
        Model {
            id: "claude-sonnet-4-6".into(),
            provider: "claude-code".into(),
            display_name: None,
            context_window: 200_000,
            max_output_tokens: 64_000,
            input_limit: None,
            supports_thinking: Some(true),
            supports_xhigh: Some(false),
            supports_thinking_off: None,
            reasoning_efforts: None,
            supports_tools: Some(true),
            supports_vision: Some(true),
            supports_cache: Some(true),
            supports_structured_output: Some(false),
            thinking_budgets: None,
            pricing: None,
            speech: None,
        }
    }

    #[test]
    fn body_is_the_stream_body_minus_stream_max_tokens_and_cache_markers() {
        let messages = vec![
            user("hi"),
            assistant(vec![
                ContentBlock::Thinking {
                    text: "plan".into(),
                    signature: Some("sig".into()),
                },
                ContentBlock::FunctionCall {
                    id: "t1".into(),
                    function_id: "shell::exec".into(),
                    arguments: json!({ "cmd": "ls" }),
                },
            ]),
            user("go on"),
            assistant(vec![ContentBlock::Text {
                text: "partial".into(),
            }]),
        ];
        let meta = thinking_model();
        let levels = [
            None,
            Some(ThinkingLevel::Off),
            Some(ThinkingLevel::Low),
            Some(ThinkingLevel::High),
            Some(ThinkingLevel::Xhigh),
        ];
        for model_meta in [None, Some(&meta)] {
            for level in levels {
                let thinking = build_thinking_config(level, model_meta);
                let mut stream = build_body(
                    &BodyArgs {
                        model: "claude-sonnet-4-6".into(),
                        max_tokens: 4096,
                        system_prompt: "be brief".into(),
                        system_sections: None,
                        messages: messages.clone(),
                        tools: vec![tool()],
                        thinking: thinking.config,
                        effort: thinking.effort,
                        cache_enabled: false,
                        cache_ttl: None,
                        messages_cache_ttl: None,
                    },
                    &mut Vec::new(),
                );
                let obj = stream.as_object_mut().unwrap();
                obj.remove("stream");
                obj.remove("max_tokens");
                let count = build_count_body(
                    "claude-sonnet-4-6".into(),
                    "be brief".into(),
                    vec![tool()],
                    messages.clone(),
                    level,
                    model_meta,
                );
                assert_eq!(
                    count,
                    stream,
                    "level {level:?}, meta {}",
                    model_meta.is_some()
                );
            }
        }
    }

    #[test]
    fn absent_level_on_a_thinking_model_counts_adaptive_thinking() {
        // The reachable router case: no level forwarded, catalog says thinking.
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            String::new(),
            vec![],
            vec![user("hi")],
            None,
            Some(&thinking_model()),
        );
        assert_eq!(body["thinking"]["type"], "adaptive", "{body}");
        assert!(body.get("output_config").is_none(), "{body}");
    }

    #[test]
    fn request_accepts_a_thinking_level() {
        let req: CountTokensRequest = serde_json::from_value(json!({
            "model": "claude-code/claude-sonnet-4-6",
            "thinking_level": "high",
            "messages": [{"role": "user", "content": [{"type": "text", "text": "hi"}], "timestamp": 1}],
        }))
        .unwrap();
        assert_eq!(req.thinking_level, Some(ThinkingLevel::High));
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
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            String::new(),
            vec![],
            vec![user("hi")],
            None,
            None,
        );
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
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            String::new(),
            vec![],
            vec![user("hi")],
            None,
            None,
        );
        let err = post_count(&reqwest::Client::new(), &cfg(url), &body)
            .await
            .expect_err("401 is an error");
        assert_eq!(err.status, Some(401));
        let msg = err.error.to_string();
        assert!(msg.contains("provider/upstream_status: 401"), "{msg}");
    }

    /// Count stub for many requests: `{"input_tokens":7}` for `Bearer
    /// <good>`, 401 otherwise; records each request's bearer.
    async fn bearer_count_stub(
        good: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = vec![0u8; 65536];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let bearer = head
                    .lines()
                    .find_map(|line| line.strip_prefix("authorization: bearer "))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let (status, body) = if bearer == good {
                    ("200 OK", r#"{"input_tokens":7}"#)
                } else {
                    ("401 Unauthorized", "{}")
                };
                record.lock().unwrap().push(bearer);
                let response = format!(
                    "HTTP/1.1 {status}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/v1/messages"), seen)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_401_on_the_global_accounts_token_asks_the_source_once_more() {
        use crate::accounts::testing::{FakeEngine, in_one_hour};
        let asked = std::sync::atomic::AtomicUsize::new(0);
        let accounts = crate::accounts::TmuxAccounts::new(FakeEngine::new(move |id, payload| {
            assert_eq!(id, crate::accounts::CREDENTIAL_FN);
            let n = asked.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
            Ok(json!({
                "account": payload["account"],
                "access_token": format!("work-{n}"),
                "expires_at_ms": in_one_hour()
            }))
        }));
        let body = build_count_body(
            "claude-sonnet-4-6".into(),
            String::new(),
            vec![],
            vec![user("hi")],
            None,
            None,
        );
        let token = accounts.token("work").await.unwrap();
        let (url, seen) = bearer_count_stub("work-2").await;
        let mut config = ClaudeCodeConfig {
            credential_value: token.access_token.clone(),
            ..cfg(url)
        };
        let tokens = post_count_for_account(
            &reqwest::Client::new(),
            &mut config,
            &body,
            Some(&token),
            &accounts,
        )
        .await
        .expect("the second token counts");
        assert_eq!(tokens, 7);
        assert_eq!(*seen.lock().unwrap(), ["work-1", "work-2"]);

        // A second 401 is final, and that token leaves the cache.
        let token = accounts.token("work").await.unwrap();
        let (url, seen) = bearer_count_stub("never").await;
        let mut config = ClaudeCodeConfig {
            credential_value: token.access_token.clone(),
            ..cfg(url)
        };
        post_count_for_account(
            &reqwest::Client::new(),
            &mut config,
            &body,
            Some(&token),
            &accounts,
        )
        .await
        .expect_err("refused twice");
        assert_eq!(*seen.lock().unwrap(), ["work-2", "work-3"]);
        assert_eq!(accounts.token("work").await.unwrap().access_token, "work-4");

        // The vault path keeps a single attempt.
        let (url, seen) = bearer_count_stub("never").await;
        let mut config = ClaudeCodeConfig {
            credential_value: "vault-token".into(),
            ..cfg(url)
        };
        let err =
            post_count_for_account(&reqwest::Client::new(), &mut config, &body, None, &accounts)
                .await
                .expect_err("a 401 without an account source is final");
        assert!(err.to_string().contains("401"), "{err}");
        assert_eq!(*seen.lock().unwrap(), ["vault-token"]);
    }
}
