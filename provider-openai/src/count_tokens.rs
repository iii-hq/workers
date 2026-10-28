//! `provider::openai::count_tokens` — prompt token counting behind
//! `router::count_tokens`. On the Responses API the count comes from OpenAI's
//! own metering route, `POST …/responses/input_tokens`, fed the same body the
//! stream sends, so images, per-item call framing, the tool preamble and the
//! `text.format` schema are counted the way they are billed. That route never
//! runs the model and costs nothing.
//!
//! Chat Completions gateways have no such route, and a Responses gateway may
//! not serve it, so the fallback is the local tiktoken estimate shared with
//! `provider-openai-codex` (`llm_router::provider_scaffold::tiktoken_count`).

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::endpoint_count::{post_count, ESTIMATOR_METERED};
use llm_router::provider_scaffold::tiktoken_count::{
    count_chat_tokens, encoder_for, ESTIMATOR_TIKTOKEN,
};
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::config::{config_from_resolve, ApiMode, OpenaiConfig};
use crate::errors::classify_bus_error;
use crate::request::{build_body, build_chat_response_format, build_headers, BodyArgs};
use crate::state;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets. The metering route takes it as is; the
    /// tiktoken fallback selects its tokenizer from it (o200k_base for
    /// gpt-4o/gpt-5/o-series and anything unknown-modern, cl100k_base for
    /// gpt-3.5 and non-o gpt-4 families).
    pub model: String,
    /// System prompt counted as its own wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; each serialized schema counts toward
    /// the total.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::openai::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
    /// Structured-output format the turn sends; its schema is part of the
    /// billed prompt.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens for the assembled request.
    pub tokens: u64,
    /// `metered` when OpenAI's input_tokens route counted the request,
    /// `tiktoken` when the local tokenizer estimated it.
    pub estimator: String,
}

/// Metering route for the configured Responses `api_url`: its
/// `/input_tokens` child (`…/v1/responses` → `…/v1/responses/input_tokens`),
/// so proxies and gateways stay on their own host.
fn input_tokens_url(api_url: &str) -> String {
    format!("{}/input_tokens", api_url.trim_end_matches('/'))
}

/// The stream's Responses body without what the metering route rejects or
/// what would touch the cache: no `stream`, `store`, output ceiling, cache
/// key or retention. Reasoning effort is left out because it does not change
/// the count (measured: `hi` is 7 tokens at every effort).
fn build_count_body(req: &CountTokensRequest) -> Value {
    let mut body = build_body(
        &BodyArgs {
            model: req.model.clone(),
            max_tokens: None,
            system_prompt: req.system_prompt.clone().unwrap_or_default(),
            messages: req.messages.clone(),
            tools: req.tools.clone().unwrap_or_default(),
            reasoning_effort: None,
            response_format: req.response_format.clone(),
            prompt_cache_key: None,
            system_sections: None,
            explicit_cache_breakpoints: false,
            cache_retention: None,
        },
        ApiMode::Responses,
    );
    if let Some(fields) = body.as_object_mut() {
        fields.remove("stream");
        fields.remove("store");
    }
    body
}

fn local_count(req: CountTokensRequest) -> CountTokensResponse {
    let format_tokens = req.response_format.as_ref().map_or(0, |rf| {
        encoder_for(&req.model)
            .encode_ordinary(&build_chat_response_format(rf).to_string())
            .len() as u64
    });
    let tokens = count_chat_tokens(
        &req.model,
        req.system_prompt.as_deref(),
        req.tools.as_deref().unwrap_or(&[]),
        &req.messages,
    ) + format_tokens;
    CountTokensResponse {
        model: req.model,
        tokens,
        estimator: ESTIMATOR_TIKTOKEN.into(),
    }
}

/// Count `req` against `cfg`'s endpoint, or locally when there is no
/// config, the endpoint speaks Chat Completions, or the metering call fails.
pub async fn count(
    http: &reqwest::Client,
    cfg: Option<&OpenaiConfig>,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }
    let Some(cfg) = cfg.filter(|cfg| cfg.api_mode == ApiMode::Responses) else {
        return Ok(local_count(req));
    };
    let headers = build_headers(cfg)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();
    match post_count(
        http,
        input_tokens_url(&cfg.api_url),
        headers,
        build_count_body(&req),
        |reply| reply.get("input_tokens").and_then(Value::as_u64),
    )
    .await
    {
        Ok(tokens) => Ok(CountTokensResponse {
            model: req.model,
            tokens,
            estimator: ESTIMATOR_METERED.into(),
        }),
        Err(e) => {
            tracing::warn!(error = %e, "openai input_tokens failed; counting locally");
            Ok(local_count(req))
        }
    }
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &ScaffoldCache,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    if req.messages.is_empty() {
        return count(http, None, req).await;
    }
    // Resolved the way the stream resolves, so a count and a generation
    // agree on the endpoint and credential. A failed resolve still counts,
    // locally: the estimate needs neither.
    let token = cache.load_token(iii, state::STATE_SCOPE).await;
    let cfg = match cache
        .resolve(
            iii,
            crate::PROVIDER_ID,
            token.as_deref(),
            Some(crate::register::CREDENTIAL_ENV_VAR),
        )
        .await
    {
        Ok(resolved) => config_from_resolve(&req.model, None, &resolved).ok(),
        Err(e) => {
            if classify_bus_error(&e) == ErrorKind::AuthExpired {
                cache.invalidate();
            }
            None
        }
    };
    count(http, cfg.as_ref(), req).await
}

#[cfg(test)]
mod tests {
    use super::*;
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

    fn with_image() -> AgentMessage {
        AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![
                ContentBlock::Text { text: "hi".into() },
                ContentBlock::Image {
                    mime: "image/png".into(),
                    data: "AAAA".into(),
                    attachment_id: None,
                },
            ],
            timestamp: 1,
        })
    }

    fn request(model: &str, messages: Vec<AgentMessage>) -> CountTokensRequest {
        CountTokensRequest {
            model: model.into(),
            system_prompt: None,
            tools: None,
            messages,
            response_format: None,
        }
    }

    fn schema_format() -> ResponseFormat {
        ResponseFormat {
            r#type: "json".into(),
            schema: Some(json!({
                "type": "object",
                "properties": { "answer": { "type": "string" } },
                "required": ["answer"],
                "additionalProperties": false
            })),
        }
    }

    fn cfg(api_url: String) -> OpenaiConfig {
        OpenaiConfig {
            credential_value: "sk-test".into(),
            model: "gpt-5-nano".into(),
            max_tokens: Some(4096),
            api_mode: ApiMode::from_url(&api_url),
            api_url,
        }
    }

    /// One-shot upstream: answers `response` and hands back the raw request.
    async fn stub(response: &'static str) -> (String, tokio::sync::oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut raw = Vec::new();
                let mut buf = [0u8; 65536];
                // Read until the declared body has arrived.
                loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    raw.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&raw).to_string();
                    let complete = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        body.len() >= len
                    });
                    if n == 0 || complete {
                        break;
                    }
                }
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
                let _ = tx.send(String::from_utf8_lossy(&raw).to_string());
            }
        });
        (format!("http://{addr}/v1/responses"), rx)
    }

    const METERED: &str = "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"object\":\"response.input_tokens\",\"input_tokens\":1545}";
    const NOT_FOUND: &str = "HTTP/1.1 404 Not Found\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"no route\"}}";

    #[tokio::test]
    async fn empty_messages_are_rejected() {
        assert!(
            count(&reqwest::Client::new(), None, request("gpt-5", vec![]))
                .await
                .is_err()
        );
    }

    #[test]
    fn input_tokens_route_is_the_responses_child() {
        assert_eq!(
            input_tokens_url("https://api.openai.com/v1/responses"),
            "https://api.openai.com/v1/responses/input_tokens"
        );
        assert_eq!(
            input_tokens_url("http://127.0.0.1:9/v1/responses/"),
            "http://127.0.0.1:9/v1/responses/input_tokens"
        );
    }

    #[test]
    fn count_body_is_the_stream_body_without_what_the_route_rejects() {
        let mut req = request("gpt-5-nano", vec![with_image()]);
        req.system_prompt = Some("be brief".into());
        req.tools = Some(vec![AgentFunction {
            name: "agent::trigger".into(),
            description: "d".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }]);
        req.response_format = Some(schema_format());
        let body = build_count_body(&req);
        assert_eq!(body["model"], "gpt-5-nano");
        assert_eq!(body["input"][0]["role"], "system");
        assert_eq!(body["input"][1]["content"][1]["type"], "input_image");
        assert_eq!(body["tools"][0]["name"], "agent__trigger");
        assert_eq!(body["text"]["format"]["type"], "json_schema");
        for absent in [
            "stream",
            "store",
            "max_output_tokens",
            "prompt_cache_key",
            "prompt_cache_retention",
            "reasoning",
        ] {
            assert!(body.get(absent).is_none(), "{absent} must not be sent");
        }
    }

    #[tokio::test]
    async fn responses_endpoint_counts_through_input_tokens() {
        let (url, seen) = stub(METERED).await;
        let resp = count(
            &reqwest::Client::new(),
            Some(&cfg(url)),
            request("gpt-5-nano", vec![with_image()]),
        )
        .await
        .unwrap();
        assert_eq!(resp.tokens, 1545);
        assert_eq!(resp.estimator, "metered");
        let raw = seen.await.unwrap();
        assert!(raw.starts_with("POST /v1/responses/input_tokens "), "{raw}");
        assert!(raw
            .to_ascii_lowercase()
            .contains("authorization: bearer sk-test"));
        let body: Value = serde_json::from_str(raw.split_once("\r\n\r\n").unwrap().1).unwrap();
        assert_eq!(body["input"][0]["content"][1]["type"], "input_image");
        assert!(body.get("stream").is_none());
    }

    #[tokio::test]
    async fn a_failing_metering_route_falls_back_to_tiktoken() {
        let (url, _seen) = stub(NOT_FOUND).await;
        let resp = count(
            &reqwest::Client::new(),
            Some(&cfg(url)),
            request("gpt-5", vec![user("hello world")]),
        )
        .await
        .unwrap();
        assert_eq!(resp.estimator, "tiktoken");
        assert_eq!(
            resp.tokens,
            count_chat_tokens("gpt-5", None, &[], &[user("hello world")])
        );
    }

    #[tokio::test]
    async fn chat_completions_and_unconfigured_count_locally() {
        // A port nothing listens on: the local path must not call out.
        let chat = cfg("http://127.0.0.1:9/v1/chat/completions".into());
        for cfg in [Some(&chat), None] {
            let resp = count(
                &reqwest::Client::new(),
                cfg,
                request("gpt-5", vec![user("hello world")]),
            )
            .await
            .unwrap();
            assert_eq!(resp.estimator, "tiktoken");
            assert_eq!(resp.model, "gpt-5");
            assert_eq!(
                resp.tokens,
                count_chat_tokens("gpt-5", None, &[], &[user("hello world")])
            );
        }
    }

    #[tokio::test]
    async fn the_local_estimate_counts_the_response_format_schema() {
        let plain = count(
            &reqwest::Client::new(),
            None,
            request("gpt-5", vec![user("hi")]),
        )
        .await
        .unwrap();
        let mut req = request("gpt-5", vec![user("hi")]);
        req.response_format = Some(schema_format());
        let formatted = count(&reqwest::Client::new(), None, req).await.unwrap();
        assert!(
            formatted.tokens > plain.tokens + 10,
            "{} vs {}",
            formatted.tokens,
            plain.tokens
        );
    }
}
