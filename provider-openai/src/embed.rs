//! `provider::openai::embed` — batch text embeddings through the OpenAI
//! embeddings endpoint, using the same router-resolved credential as the
//! chat path. Consumers (llm-router's `router::embed`, and through it the
//! memory worker's semantic recall) treat this as the provider protocol's
//! embedding surface.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{config_from_resolve, ConfigError};
use crate::{router_client, state};

pub const DEFAULT_EMBED_MODEL: &str = "text-embedding-3-small";
const EMBED_URL: &str = "https://api.openai.com/v1/embeddings";
/// Embeddings requests are bounded and non-streaming; a tight budget keeps
/// hook callers (memory recall) inside their own timeouts.
const EMBED_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmbedRequest {
    /// Embedding model id; the provider default when omitted.
    #[serde(default)]
    pub model: Option<String>,
    /// Texts to embed, one vector returned per input, order preserved.
    pub input: Vec<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct EmbedResponse {
    pub model: String,
    /// One embedding per input, in input order.
    pub embeddings: Vec<Vec<f32>>,
}

#[derive(Debug, Deserialize)]
struct WireEmbedding {
    index: usize,
    embedding: Vec<f32>,
}

#[derive(Debug, Deserialize)]
struct WireResponse {
    data: Vec<WireEmbedding>,
    model: String,
}

pub async fn handle(
    iii: &IIIClient,
    http: &reqwest::Client,
    req: EmbedRequest,
) -> Result<EmbedResponse, Error> {
    if req.input.is_empty() || req.input.len() > 512 {
        return Err(Error::Handler(
            "invalid_input: input must contain 1..=512 texts".into(),
        ));
    }
    let model = req
        .model
        .filter(|m| !m.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_EMBED_MODEL.to_string());

    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;
    // Reuse the chat-path credential validation; the api_url from resolve
    // targets the chat endpoint, so embeddings always use their own URL.
    let cfg = config_from_resolve(&model, None, &resolved).map_err(|e| match e {
        ConfigError::NotConfigured => Error::Handler(
            "provider/not_configured: no api_key in the llm-router entry for openai".into(),
        ),
        other => Error::Handler(format!("provider/config: {other:?}")),
    })?;

    let response = http
        .post(EMBED_URL)
        .bearer_auth(&cfg.credential_value)
        .timeout(std::time::Duration::from_secs(EMBED_TIMEOUT_SECS))
        .json(&serde_json::json!({ "model": model, "input": req.input }))
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
    let wire: WireResponse = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;

    let mut data = wire.data;
    data.sort_by_key(|d| d.index);
    Ok(EmbedResponse {
        model: wire.model,
        embeddings: data.into_iter().map(|d| d.embedding).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_response_parses_and_orders() {
        let raw = r#"{"data":[{"index":1,"embedding":[0.3,0.4]},{"index":0,"embedding":[0.1,0.2]}],"model":"text-embedding-3-small","usage":{"prompt_tokens":2,"total_tokens":2}}"#;
        let wire: WireResponse = serde_json::from_str(raw).unwrap();
        let mut data = wire.data;
        data.sort_by_key(|d| d.index);
        assert_eq!(data[0].embedding, vec![0.1, 0.2]);
        assert_eq!(data[1].embedding, vec![0.3, 0.4]);
    }
}
