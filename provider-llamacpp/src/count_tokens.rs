//! `provider::llamacpp::count_tokens` — exact prompt token counting by the
//! server that holds the model.
//!
//! `llama-server` counts with the tokenizer baked into the GGUF it loaded, so
//! its answer is not an estimate of the model's tokenizer: it is the model's
//! tokenizer. That matters more here than for any hosted provider, because
//! the operator can load anything — Qwen, Llama, Mistral, a private
//! fine-tune — and no table shipped in this binary could know which
//! vocabulary is in memory right now.
//!
//! The route used is llama.cpp's `POST /v1/chat/completions/input_tokens`,
//! which parses the same body `/v1/chat/completions` bills (tools, call-only
//! assistant turns, image parts, `chat_template_kwargs`) and answers
//! `{"input_tokens": N}`. Servers built before that route existed answer 404;
//! they are counted through the Anthropic-compatible
//! `/v1/messages/count_tokens` instead, whose converter loses tool
//! definitions, call-only turns and images, so that count runs low. Counting
//! is local to the operator's machine and costs nothing.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::cache::ScaffoldCache;
use llm_router::provider_scaffold::endpoint_count::{post_count, ESTIMATOR_METERED};
use llm_router::types::events::ErrorKind;
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::{AgentFunction, ThinkingLevel};
use llm_router::types::router::ResponseFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::config_from_resolve;
use crate::errors::classify_bus_error;
use crate::request::{build_body, build_headers, enable_thinking, BodyArgs};
use crate::state;
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;

/// The server root the counting routes hang off, like `/props`: they are not
/// children of the configured chat route.
fn server_root(api_url: &str) -> &str {
    api_url
        .strip_suffix("/v1/chat/completions")
        .unwrap_or_else(|| api_url.trim_end_matches('/'))
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets; echoed in the reply, since `llama-server`
    /// counts with whichever model it has loaded.
    pub model: String,
    /// System prompt counted as the leading wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; mapped to the wire `tools` array.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Wire agent messages, the same shape `provider::llamacpp::stream`
    /// accepts. Must be non-empty.
    pub messages: Vec<AgentMessage>,
    /// The turn's reasoning level, mapped onto
    /// `chat_template_kwargs.enable_thinking` exactly as the stream does;
    /// absent counts the thinking-off prompt the stream sends by default.
    #[serde(default)]
    pub thinking_level: Option<ThinkingLevel>,
    /// The turn's structured-output request, sent as the stream sends it.
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens the server counted with the loaded model's tokenizer.
    pub tokens: u64,
    /// Always `metered`: the server that holds the model produced the count.
    pub estimator: String,
}

/// The chat body less what only steers generation: the counting route
/// renders and tokenizes the same prompt the chat route bills.
fn count_body(args: &BodyArgs) -> Value {
    let mut body = build_body(args);
    if let Some(fields) = body.as_object_mut() {
        for knob in ["stream", "stream_options", "max_tokens"] {
            fields.remove(knob);
        }
    }
    body
}

/// Body for the Anthropic-compatible fallback route.
fn anthropic_count_body(args: &BodyArgs) -> Value {
    let mut body = json!({
        "model": args.model,
        "messages": to_wire_messages(&args.messages, &args.system_prompt),
    });
    if !args.tools.is_empty() {
        body["tools"] = json!(functions_to_wire(&args.tools));
    }
    body
}

async fn count(
    http: &reqwest::Client,
    api_url: &str,
    headers: Vec<(String, String)>,
    args: &BodyArgs,
) -> Result<u64, Error> {
    let root = server_root(api_url);
    let pick = |reply: &Value| reply.get("input_tokens").and_then(Value::as_u64);
    let counted = post_count(
        http,
        format!("{root}/v1/chat/completions/input_tokens"),
        headers.clone(),
        count_body(args),
        pick,
    )
    .await;
    match counted {
        // `post_count` reports a non-2xx as `provider/upstream_status: <code> …`.
        Err(Error::Handler(msg)) if msg.starts_with("provider/upstream_status: 404 ") => {
            post_count(
                http,
                format!("{root}/v1/messages/count_tokens"),
                headers,
                anthropic_count_body(args),
                pick,
            )
            .await
        }
        counted => counted,
    }
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

    let args = BodyArgs {
        model: cfg.model.clone(),
        max_tokens: cfg.max_tokens,
        system_prompt: req.system_prompt.unwrap_or_default(),
        messages: req.messages,
        tools: req.tools.unwrap_or_default(),
        response_format: req.response_format,
        enable_thinking: enable_thinking(req.thinking_level),
    };
    let headers = build_headers(&cfg)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();

    let tokens = count(http, &cfg.api_url, headers, &args).await?;

    Ok(CountTokensResponse {
        model: cfg.model,
        tokens,
        estimator: ESTIMATOR_METERED.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_router::types::content::ContentBlock;
    use llm_router::types::events::StopReason;
    use llm_router::types::messages::{
        AssistantMessage, AssistantRoleTag, FunctionResultMessage, FunctionResultRoleTag,
        UserMessage, UserRoleTag,
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// The agent-loop shape the Anthropic route lost: a tool, an image, and
    /// an assistant turn that is only a function call.
    fn args() -> BodyArgs {
        BodyArgs {
            model: "qwen3-8b".into(),
            max_tokens: 4096,
            system_prompt: "be brief".into(),
            messages: vec![
                AgentMessage::User(UserMessage {
                    role: UserRoleTag::User,
                    content: vec![
                        ContentBlock::Text {
                            text: "list it".into(),
                        },
                        ContentBlock::Image {
                            mime: "image/png".into(),
                            data: "QUJD".into(),
                            attachment_id: None,
                        },
                    ],
                    timestamp: 1,
                }),
                AgentMessage::Assistant(AssistantMessage {
                    role: AssistantRoleTag::Assistant,
                    content: vec![ContentBlock::FunctionCall {
                        id: "c1".into(),
                        function_id: "shell::exec".into(),
                        arguments: json!({ "cmd": "ls" }),
                    }],
                    stop_reason: StopReason::FunctionCall,
                    native_stop_reason: None,
                    error_message: None,
                    error_kind: None,
                    warnings: None,
                    usage: None,
                    model: "qwen3-8b".into(),
                    provider: "llamacpp".into(),
                    timestamp: 2,
                }),
                AgentMessage::FunctionResult(FunctionResultMessage {
                    role: FunctionResultRoleTag::FunctionResult,
                    function_call_id: "c1".into(),
                    function_id: "shell::exec".into(),
                    content: vec![ContentBlock::Text { text: "a b".into() }],
                    details: json!({}),
                    is_error: false,
                    timestamp: 3,
                }),
            ],
            tools: vec![AgentFunction {
                name: "shell::exec".into(),
                description: "run a command".into(),
                parameters: json!({ "type": "object", "properties": { "cmd": { "type": "string" } } }),
                label: None,
                execution_mode: None,
            }],
            response_format: None,
            enable_thinking: false,
        }
    }

    /// HTTP stub answering one connection per entry in `responses`, in order,
    /// and handing back each request line it saw.
    async fn stub(responses: Vec<&'static str>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let mut seen = Vec::new();
            for response in responses {
                let (mut sock, _) = listener.accept().await.unwrap();
                // Read the whole request so closing never resets the client.
                let mut buf = Vec::new();
                let mut chunk = [0u8; 65536];
                loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).into_owned();
                    let done = text.split_once("\r\n\r\n").is_some_and(|(head, body)| {
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
                    if n == 0 || done {
                        seen.push(text.lines().next().unwrap_or_default().to_string());
                        break;
                    }
                }
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
            seen
        });
        (format!("http://{addr}/v1/chat/completions"), handle)
    }

    const NOT_FOUND: &str =
        "HTTP/1.1 404 Not Found\r\ncontent-type: text/plain\r\ncontent-length: 14\r\n\r\nFile Not Found";

    #[test]
    fn the_count_body_is_the_chat_body_without_the_generation_knobs() {
        // The chat route bills exactly what this body renders, so anything
        // that shapes the prompt (tools, call-only turns, image parts,
        // chat_template_kwargs, response_format) must be identical.
        let mut chat = build_body(&args());
        for knob in ["stream", "stream_options", "max_tokens"] {
            chat.as_object_mut().unwrap().remove(knob);
        }
        assert_eq!(count_body(&args()), chat);
    }

    #[tokio::test]
    async fn counts_through_the_openai_native_route() {
        let (url, seen) = stub(vec![
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 52\r\n\r\n{\"input_tokens\":42,\"object\":\"response.input_tokens\"}",
        ])
        .await;
        let tokens = count(&reqwest::Client::new(), &url, vec![], &args())
            .await
            .unwrap();
        assert_eq!(tokens, 42);
        assert_eq!(
            seen.await.unwrap(),
            ["POST /v1/chat/completions/input_tokens HTTP/1.1"]
        );
    }

    #[tokio::test]
    async fn a_server_without_the_route_falls_back_to_the_anthropic_one() {
        // llama-server builds before June 2026 have no chat counting route.
        let (url, seen) = stub(vec![
            NOT_FOUND,
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 18\r\n\r\n{\"input_tokens\":7}",
        ])
        .await;
        let tokens = count(&reqwest::Client::new(), &url, vec![], &args())
            .await
            .unwrap();
        assert_eq!(tokens, 7);
        assert_eq!(
            seen.await.unwrap(),
            [
                "POST /v1/chat/completions/input_tokens HTTP/1.1",
                "POST /v1/messages/count_tokens HTTP/1.1",
            ]
        );
    }

    #[tokio::test]
    async fn any_other_failure_is_reported_not_retried() {
        let (url, seen) = stub(vec![
            "HTTP/1.1 500 Internal Server Error\r\ncontent-length: 4\r\n\r\nboom",
        ])
        .await;
        let err = count(&reqwest::Client::new(), &url, vec![], &args())
            .await
            .unwrap_err();
        assert!(err.to_string().contains("500"), "{err}");
        assert_eq!(
            seen.await.unwrap(),
            ["POST /v1/chat/completions/input_tokens HTTP/1.1"]
        );
    }

    #[test]
    fn the_count_routes_are_server_root_paths() {
        // The counting routes sit at the server root, so the whole
        // `/v1/chat/completions` tail goes before they are appended.
        for api_url in [
            "http://127.0.0.1:8080/v1/chat/completions",
            "http://127.0.0.1:8080/",
        ] {
            assert_eq!(server_root(api_url), "http://127.0.0.1:8080");
        }
    }
}
