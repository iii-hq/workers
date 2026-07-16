//! `provider::llamacpp::embed` — batch text embeddings against the
//! configured llama-server, which exposes an OpenAI-compatible
//! `/v1/embeddings` when started with `--embeddings` and an
//! embedding-capable GGUF. Consumers (llm-router's `router::embed`, and
//! through it the memory worker's semantic recall) treat this as the
//! provider protocol's embedding surface. Fully local: no cloud, no
//! credential unless the server was started with `--api-key`.

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{config_from_resolve, ConfigError};
use crate::{router_client, state};

/// Embeddings requests are bounded and non-streaming; a tight budget keeps
/// hook callers (memory recall) inside their own timeouts.
const EMBED_TIMEOUT_SECS: u64 = 20;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EmbedRequest {
    /// Model name passed through to the server. llama-server embeds with
    /// its loaded model regardless; the field is echoed for parity with
    /// the other providers.
    #[serde(default)]
    pub model: Option<String>,
    /// Texts to embed, one vector returned per input, order preserved.
    #[schemars(length(min = 1, max = 512))]
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
    #[serde(default)]
    model: Option<String>,
}

/// Embeddings endpoint for the configured chat `api_url`: the sibling
/// `/embeddings` on the same host and version prefix, so a nonstandard
/// port or path prefix keeps working.
fn embed_url(chat_api_url: &str) -> String {
    let fallback = || crate::config::DEFAULT_API_URL.replace("/chat/completions", "/embeddings");
    let Ok(mut url) = reqwest::Url::parse(chat_api_url) else {
        return fallback();
    };
    let path = url.path().trim_end_matches('/').to_string();
    let new_path = if let Some(base) = path.strip_suffix("/chat/completions") {
        format!("{base}/embeddings")
    } else {
        format!("{path}/embeddings")
    };
    url.set_path(&new_path);
    url.set_query(None);
    url.to_string()
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
        .unwrap_or_else(|| "default".to_string());

    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;
    let cfg = config_from_resolve(&model, None, &resolved).map_err(|e| match e {
        ConfigError::InvalidApiUrl(u) => {
            Error::Handler(format!("provider/config: invalid endpoint url {u:?}"))
        }
    })?;

    let mut request = http
        .post(embed_url(&cfg.api_url))
        .timeout(std::time::Duration::from_secs(EMBED_TIMEOUT_SECS))
        .json(&serde_json::json!({ "model": model, "input": req.input }));
    // Local llama-server usually runs with no --api-key; only send the
    // header when a credential resolved.
    if let Some(credential) = &cfg.credential_value {
        request = request.bearer_auth(credential);
    }
    let response = request
        .send()
        .await
        .map_err(|e| Error::Handler(format!("provider/upstream: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let excerpt: String = body.chars().take(300).collect();
        return Err(Error::Handler(format!(
            "provider/upstream_status: {status}: {excerpt} \
             (llama-server needs --embeddings and an embedding-capable model)"
        )));
    }
    let wire: WireResponse = response
        .json()
        .await
        .map_err(|e| Error::Handler(format!("provider/bad_response: {e}")))?;

    // One-vector-per-input contract: a silent gap would attach the wrong
    // vector to the wrong text downstream.
    let expected = req.input.len();
    if wire.data.len() != expected {
        return Err(Error::Handler(format!(
            "provider/bad_response: {} embeddings for {expected} inputs",
            wire.data.len()
        )));
    }
    let mut data = wire.data;
    data.sort_by_key(|d| d.index);
    if data.iter().enumerate().any(|(i, d)| d.index != i) {
        return Err(Error::Handler(
            "provider/bad_response: duplicate or out-of-range embedding indices".into(),
        ));
    }
    Ok(EmbedResponse {
        model: wire.model.unwrap_or(model),
        embeddings: data.into_iter().map(|d| d.embedding).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embed_url_follows_the_configured_endpoint() {
        assert_eq!(
            embed_url("http://127.0.0.1:8080/v1/chat/completions"),
            "http://127.0.0.1:8080/v1/embeddings"
        );
        assert_eq!(
            embed_url("http://gpu-box:9000/llama/v1/chat/completions"),
            "http://gpu-box:9000/llama/v1/embeddings"
        );
        assert_eq!(
            embed_url("not a url"),
            "http://127.0.0.1:8080/v1/embeddings"
        );
    }

    #[test]
    fn wire_response_orders_and_validates() {
        let raw = r#"{"data":[{"index":1,"embedding":[0.3]},{"index":0,"embedding":[0.1]}],"model":"nomic"}"#;
        let wire: WireResponse = serde_json::from_str(raw).unwrap();
        let mut data = wire.data;
        data.sort_by_key(|d| d.index);
        assert_eq!(data[0].embedding, vec![0.1]);
        assert!(!data.iter().enumerate().any(|(i, d)| d.index != i));

        let gap = r#"{"data":[{"index":0,"embedding":[0.1]},{"index":2,"embedding":[0.2]}]}"#;
        let wire: WireResponse = serde_json::from_str(gap).unwrap();
        let mut data = wire.data;
        data.sort_by_key(|d| d.index);
        assert!(data.iter().enumerate().any(|(i, d)| d.index != i));
        assert!(wire.model.is_none());
    }
}
