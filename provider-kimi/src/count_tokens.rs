//! `provider::kimi::count_tokens` — exact prompt token counting through
//! Moonshot's own estimator.
//!
//! Moonshot meters a prompt on request (`…/tokenizers/estimate-token-count`),
//! taking the same messages a generation would carry, so the count is theirs
//! rather than a local reconstruction of it. The route is a peer of the chat
//! route, not a child, so it is derived by replacing the last path segment of
//! the configured `api_url` — which keeps proxies and gateways serving both.
//!
//! Exposed behind `router::count_tokens`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::endpoint_count::{
    base_route_url, post_count, ESTIMATOR_METERED,
};
use llm_router::types::messages::AgentMessage;
use llm_router::types::model::AgentFunction;
use llm_router::types::router::ResponseFormat;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::config::{config_from_resolve, KimiConfig};
use crate::request::{build_headers, build_response_format};
use crate::wire::messages::to_wire_messages;
use crate::wire::tools::functions_to_wire;
use crate::{router_client, state};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CountTokensRequest {
    /// Model id the prompt targets (required by the upstream estimator).
    pub model: String,
    /// System prompt counted as the leading wire message when present.
    #[serde(default)]
    pub system_prompt: Option<String>,
    /// Function invocation schemas; mapped to the wire `tools` array.
    #[serde(default)]
    pub tools: Option<Vec<AgentFunction>>,
    /// Structured output the turn requests; sent as on chat (`json_object`).
    #[serde(default)]
    pub response_format: Option<ResponseFormat>,
    /// Wire agent messages, the same shape `provider::kimi::stream` accepts.
    /// Must be non-empty.
    pub messages: Vec<AgentMessage>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct CountTokensResponse {
    pub model: String,
    /// Prompt tokens the upstream estimator counted for the assembled request.
    pub tokens: u64,
    /// Always `metered`: the count came from the upstream itself.
    pub estimator: String,
}

/// The count body: the same `{model, messages, tools?, response_format?}` a
/// generation would send, with nothing that would make the upstream do work —
/// no streaming, no output ceiling.
fn build_count_body(model: &str, req: &CountTokensRequest) -> Value {
    let mut body = json!({
        "model": model,
        "messages": to_wire_messages(&req.messages, req.system_prompt.as_deref().unwrap_or("")),
    });
    if let Some(tools) = req.tools.as_deref().filter(|t| !t.is_empty()) {
        body["tools"] = json!(functions_to_wire(tools));
    }
    if let Some(rf) = &req.response_format {
        body["response_format"] = build_response_format(rf);
    }
    body
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    req: CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    // Dumb pipe: an empty request is a caller bug, never padded into a
    // countable one with placeholder messages.
    if req.messages.is_empty() {
        return Err(Error::Handler(
            "invalid_input: messages must not be empty".into(),
        ));
    }

    // Resolved the way the stream path resolves, so a count and a generation
    // never disagree about which credential they are using.
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref())
        .await
        .map_err(|e| Error::Handler(format!("router::provider::resolve failed: {e}")))?;
    let cfg = config_from_resolve(&req.model, None, &resolved)
        .map_err(|_| Error::Handler("provider/not_configured: no usable credential".into()))?;

    count(http, &cfg, &req).await
}

/// The upstream half of `handle`, for a caller that already holds a config.
pub async fn count(
    http: &reqwest::Client,
    cfg: &KimiConfig,
    req: &CountTokensRequest,
) -> Result<CountTokensResponse, Error> {
    let headers = build_headers(cfg)
        .into_iter()
        .map(|(name, value)| (name.to_string(), value))
        .collect();

    let tokens = post_count(
        http,
        base_route_url(&cfg.api_url, "tokenizers/estimate-token-count"),
        headers,
        build_count_body(&cfg.model, req),
        // Moonshot wraps the number: `{"data": {"total_tokens": N}}`.
        |reply| {
            reply
                .get("data")
                .and_then(|data| data.get("total_tokens"))
                .and_then(Value::as_u64)
        },
    )
    .await?;

    Ok(CountTokensResponse {
        model: cfg.model.clone(),
        tokens,
        estimator: ESTIMATOR_METERED.into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::{build_body, BodyArgs};
    use llm_router::types::content::ContentBlock;
    use llm_router::types::messages::{UserMessage, UserRoleTag};

    /// Whatever the chat request carries reaches the estimator the same way;
    /// only the knobs that add no prompt tokens stay out of the count.
    #[test]
    fn count_body_is_the_chat_body_without_the_generation_knobs() {
        let messages = vec![AgentMessage::User(UserMessage {
            role: UserRoleTag::User,
            content: vec![ContentBlock::Text {
                text: "Reply in JSON: hi".into(),
            }],
            timestamp: 1,
        })];
        let tools = vec![AgentFunction {
            name: "docs::read".into(),
            description: "Read one segment.".into(),
            parameters: json!({ "type": "object" }),
            label: None,
            execution_mode: None,
        }];
        let json_mode = ResponseFormat {
            r#type: "json".into(),
            schema: Some(json!({ "type": "object" })),
        };
        for response_format in [None, Some(json_mode)] {
            let mut chat = build_body(&BodyArgs {
                model: "kimi-k2-0905-preview".into(),
                max_tokens: 1,
                system_prompt: "be brief".into(),
                messages: messages.clone(),
                tools: tools.clone(),
                response_format: response_format.clone(),
                prompt_cache_key: Some("session".into()),
            });
            for knob in ["max_tokens", "stream", "stream_options", "prompt_cache_key"] {
                chat.as_object_mut().unwrap().remove(knob);
            }
            let req: CountTokensRequest = serde_json::from_value(json!({
                "model": "kimi-k2-0905-preview",
                "system_prompt": "be brief",
                "tools": tools,
                "response_format": response_format,
                "messages": messages,
            }))
            .unwrap();
            assert_eq!(build_count_body("kimi-k2-0905-preview", &req), chat);
        }
    }
}
