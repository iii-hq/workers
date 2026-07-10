//! Dynamic Codex model discovery through the authenticated ChatGPT backend.
//!
//! The backend's `/models` response is the authoritative provider slice. A
//! successful non-empty refresh replaces the router slice wholesale, so newly
//! published models appear and retired models disappear. Failed or empty
//! refreshes leave the last known router catalog untouched.
use crate::config::{build_backend_config, CodexBackendConfig};
use crate::request::{build_backend_headers, CODEX_COMPAT_VERSION};
use crate::{auth, router_client, state as registration_state, PROVIDER_ID};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{
    CredentialSource, ProviderResolveResponse, RefreshModelsRequest, RefreshModelsResponse,
};
use reqwest::header::ETAG;
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// Match the Codex app's refresh cadence.
pub const MODELS_REFRESH_INTERVAL: Duration = Duration::from_secs(3 * 60);

/// The backend does not currently advertise an output-token cap. The Responses
/// endpoint owns the real limit and rejects `max_output_tokens`, so this remains
/// conservative router metadata rather than a request parameter.
const DEFAULT_MAX_OUTPUT_TOKENS: u64 = 128_000;
const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;

#[derive(Debug, Default)]
pub struct CatalogRefreshState {
    snapshot: Mutex<Option<CatalogSnapshot>>,
}

#[derive(Debug)]
struct CatalogSnapshot {
    etag: Option<String>,
    count: usize,
}

#[derive(Debug)]
struct FetchedCatalog {
    models: Vec<Model>,
    etag: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    models: Vec<CodexModel>,
}

#[derive(Debug, Deserialize)]
struct CodexModel {
    slug: String,
    display_name: String,
    visibility: String,
    #[serde(default)]
    priority: i32,
    #[serde(default)]
    context_window: Option<u64>,
    #[serde(default)]
    max_context_window: Option<u64>,
    #[serde(default)]
    max_output_tokens: Option<u64>,
    #[serde(default)]
    supported_reasoning_levels: Vec<ReasoningLevel>,
    #[serde(default)]
    input_modalities: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ReasoningLevel {
    effort: String,
}

fn provider_error(code: &str, message: impl Into<String>) -> Error {
    Error::Remote {
        code: format!("provider/{code}"),
        message: message.into(),
        stacktrace: None,
    }
}

fn default_resolve() -> ProviderResolveResponse {
    ProviderResolveResponse {
        configured: false,
        source: CredentialSource::None,
        credential: None,
        api_url: None,
        max_tokens: None,
    }
}

/// Turn the configured Responses endpoint into its sibling models endpoint.
fn models_url(api_url: &str) -> Result<reqwest::Url, Error> {
    let mut url = reqwest::Url::parse(api_url)
        .map_err(|e| provider_error("invalid_models_url", e.to_string()))?;
    let path = url.path().trim_end_matches('/');
    let base = path.strip_suffix("/responses").unwrap_or(path);
    url.set_path(&format!("{base}/models"));
    url.set_query(None);
    url.set_fragment(None);
    url.query_pairs_mut()
        .append_pair("client_version", CODEX_COMPAT_VERSION);
    Ok(url)
}

fn map_models(mut remote: Vec<CodexModel>) -> Vec<Model> {
    remote.sort_by(|a, b| a.priority.cmp(&b.priority).then(a.slug.cmp(&b.slug)));
    let mut seen = HashSet::new();
    remote
        .into_iter()
        .filter(|model| model.visibility == "list")
        .filter(|model| !model.slug.trim().is_empty())
        .filter(|model| seen.insert(model.slug.clone()))
        .map(|model| {
            let supports_thinking = !model.supported_reasoning_levels.is_empty();
            let supports_xhigh = model
                .supported_reasoning_levels
                .iter()
                .any(|level| level.effort == "xhigh");
            let supports_vision = model
                .input_modalities
                .iter()
                .any(|modality| modality == "image");
            Model {
                id: format!("codex/{}", model.slug),
                provider: PROVIDER_ID.to_string(),
                display_name: Some(format!("{} (Codex)", model.display_name)),
                context_window: model
                    .context_window
                    .or(model.max_context_window)
                    .unwrap_or(DEFAULT_CONTEXT_WINDOW),
                max_output_tokens: model.max_output_tokens.unwrap_or(DEFAULT_MAX_OUTPUT_TOKENS),
                input_limit: None,
                supports_thinking: Some(supports_thinking),
                supports_xhigh: Some(supports_xhigh),
                supports_tools: Some(true),
                supports_vision: Some(supports_vision),
                supports_cache: Some(true),
                supports_structured_output: None,
                thinking_budgets: None,
                pricing: None,
            }
        })
        .collect()
}

async fn fetch_catalog(
    http: &reqwest::Client,
    cfg: &CodexBackendConfig,
) -> Result<FetchedCatalog, Error> {
    let url = models_url(&cfg.api_url)?;
    let mut request = http.get(url);
    for (name, value) in build_backend_headers(cfg) {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .map_err(|e| provider_error("models_fetch_failed", e.to_string()))?;
    let status = response.status();
    let etag = response
        .headers()
        .get(ETAG)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = response
        .bytes()
        .await
        .map_err(|e| provider_error("models_body_failed", e.to_string()))?;
    if !status.is_success() {
        let text = String::from_utf8_lossy(&body);
        return Err(provider_error(
            "models_http_error",
            format!("Codex models endpoint returned {status}: {text}"),
        ));
    }
    let response: ModelsResponse = serde_json::from_slice(&body).map_err(|e| {
        provider_error(
            "bad_models_response",
            format!("failed to decode Codex models response: {e}"),
        )
    })?;
    let models = map_models(response.models);
    if models.is_empty() {
        return Err(provider_error(
            "empty_models_catalog",
            "Codex models endpoint returned no picker-visible models; keeping the last known catalog",
        ));
    }
    Ok(FetchedCatalog { models, etag })
}

/// Fetch and reconcile the complete provider catalog.
///
/// `force_reconcile` is required after router registration/restart because the
/// router may need its persisted slice restored even when the upstream ETag is
/// unchanged. Periodic refreshes can skip an unchanged ETag.
pub async fn refresh_models(
    iii: &IIIClient,
    http: &reqwest::Client,
    refresh_state: &CatalogRefreshState,
    force_reconcile: bool,
) -> Result<usize, Error> {
    // Serialize refreshes so a slower older response cannot overwrite a newer one.
    let mut snapshot = refresh_state.snapshot.lock().await;
    let token = registration_state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref())
        .await
        .unwrap_or_else(|_| default_resolve());
    let credential = auth::fetch_fresh_credential(iii).await;
    let cfg = build_backend_config(&resolved, credential.as_ref())
        .map_err(|e| provider_error("models_not_configured", e.to_string()))?;
    let fetched = fetch_catalog(http, &cfg).await?;

    if !force_reconcile
        && fetched.etag.is_some()
        && snapshot.as_ref().and_then(|current| current.etag.as_ref()) == fetched.etag.as_ref()
    {
        return Ok(snapshot.as_ref().map_or(fetched.models.len(), |s| s.count));
    }

    let count = fetched.models.len();
    router_client::reconcile(iii, fetched.models, token.as_deref()).await?;
    *snapshot = Some(CatalogSnapshot {
        etag: fetched.etag,
        count,
    });
    Ok(count)
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
    refresh_state: Arc<CatalogRefreshState>,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let (iii, http, refresh_state) = (iii.clone(), http.clone(), refresh_state.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http, &refresh_state, true).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

/// Keep the provider catalog aligned with Codex while the worker is running.
pub async fn refresh_models_periodically(
    iii: IIIClient,
    http: reqwest::Client,
    refresh_state: Arc<CatalogRefreshState>,
) {
    loop {
        tokio::time::sleep(MODELS_REFRESH_INTERVAL).await;
        match refresh_models(&iii, &http, &refresh_state, false).await {
            Ok(count) => println!("[provider-openai-codex] periodic catalog refresh: {count} models"),
            Err(e) => eprintln!(
                "[provider-openai-codex] periodic catalog refresh failed ({e}); keeping last known catalog"
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::sync::oneshot;

    fn model(slug: &str, visibility: &str, priority: i32) -> CodexModel {
        CodexModel {
            slug: slug.into(),
            display_name: slug.into(),
            visibility: visibility.into(),
            priority,
            context_window: Some(272_000),
            max_context_window: Some(1_000_000),
            max_output_tokens: None,
            supported_reasoning_levels: vec![ReasoningLevel {
                effort: "xhigh".into(),
            }],
            input_modalities: vec!["text".into(), "image".into()],
        }
    }

    #[test]
    fn dynamic_mapping_filters_hidden_sorts_and_namespaces() {
        let models = map_models(vec![
            model("old-hidden", "hide", 0),
            model("new-second", "list", 2),
            model("new-first", "list", 1),
        ]);
        assert_eq!(
            models
                .iter()
                .map(|model| model.id.as_str())
                .collect::<Vec<_>>(),
            vec!["codex/new-first", "codex/new-second"]
        );
        assert_eq!(models[0].context_window, 272_000);
        assert_eq!(models[0].supports_xhigh, Some(true));
        assert_eq!(models[0].supports_vision, Some(true));
    }

    #[test]
    fn models_url_replaces_responses_and_sets_compat_version() {
        let url = models_url("https://chatgpt.com/backend-api/codex/responses").unwrap();
        assert_eq!(
            url.as_str(),
            format!(
                "https://chatgpt.com/backend-api/codex/models?client_version={CODEX_COMPAT_VERSION}"
            )
        );
    }

    async fn stub(body: String) -> (String, oneshot::Receiver<String>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (request_tx, request_rx) = oneshot::channel();
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut buffer = vec![0; 16 * 1024];
            let read = socket.read(&mut buffer).await.unwrap();
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let _ = request_tx.send(request);
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\netag: \"catalog-1\"\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            socket.write_all(response.as_bytes()).await.unwrap();
            socket.shutdown().await.unwrap();
        });
        (
            format!("http://{addr}/backend-api/codex/responses"),
            request_rx,
        )
    }

    #[tokio::test]
    async fn fetches_authenticated_dynamic_catalog() {
        let body = serde_json::json!({
            "models": [{
                "slug": "future-model",
                "display_name": "Future Model",
                "visibility": "list",
                "priority": 1,
                "context_window": 300000,
                "supported_reasoning_levels": [{"effort": "xhigh"}],
                "input_modalities": ["text", "image"],
                "supported_in_api": false
            }]
        })
        .to_string();
        let (api_url, request_rx) = stub(body).await;
        let cfg = CodexBackendConfig {
            access_token: "at".into(),
            account_id: "acc-1".into(),
            api_url,
        };
        let fetched = fetch_catalog(&reqwest::Client::new(), &cfg).await.unwrap();
        assert_eq!(fetched.etag.as_deref(), Some("\"catalog-1\""));
        assert_eq!(fetched.models[0].id, "codex/future-model");

        let request = request_rx.await.unwrap().to_ascii_lowercase();
        assert!(request.contains(&format!(
            "get /backend-api/codex/models?client_version={code_version} http/1.1",
            code_version = CODEX_COMPAT_VERSION
        )));
        assert!(request.contains("authorization: bearer at"));
        assert!(request.contains("chatgpt-account-id: acc-1"));
        assert!(request.contains(&format!("version: {CODEX_COMPAT_VERSION}")));
    }

    #[tokio::test]
    async fn rejects_empty_visible_catalog() {
        let body = serde_json::json!({
            "models": [{
                "slug": "hidden-model",
                "display_name": "Hidden",
                "visibility": "hide"
            }]
        })
        .to_string();
        let (api_url, _request_rx) = stub(body).await;
        let cfg = CodexBackendConfig {
            access_token: "at".into(),
            account_id: "acc-1".into(),
            api_url,
        };
        let error = fetch_catalog(&reqwest::Client::new(), &cfg)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no picker-visible models"));
    }
}
