use crate::catalog::parse_models;
use crate::config::{credential_value, endpoint, normalize_base_url, ConfigError, DEFAULT_API_URL};
use crate::errors::upstream_unavailable;
use crate::{PROVIDER_ID, STATE_SCOPE};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::provider_scaffold::{router_client, state};
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

pub fn models_url(api_url: &str) -> Result<String, ConfigError> {
    normalize_base_url(api_url).map(|base| endpoint(&base, "models"))
}

enum FetchOutcome {
    Ok(Vec<Model>),
    AuthFailed,
    Transient(String),
}

fn explicit_auth_failure(status: u16, body: &str) -> bool {
    if status == 401 {
        return true;
    }
    if status != 403 {
        return false;
    }
    serde_json::from_str::<Value>(body)
        .ok()
        .is_some_and(|value| {
            ["type", "code"].iter().any(|field| {
                value
                    .pointer(&format!("/error/{field}"))
                    .and_then(Value::as_str)
                    .is_some_and(|kind| matches!(kind, "authentication_error" | "invalid_api_key"))
            })
        })
}

fn parse_catalog_response(value: &Value) -> Result<Vec<Model>, String> {
    let models = parse_models(value);
    if models.is_empty() {
        Err(
            "models response contained no usable model rows; keeping last-known-good catalog"
                .to_string(),
        )
    } else {
        Ok(models)
    }
}

async fn fetch_models(http: &reqwest::Client, url: &str, credential: &str) -> FetchOutcome {
    let response = match http.get(url).bearer_auth(credential).send().await {
        Ok(response) => response,
        Err(error) => return FetchOutcome::Transient(format!("models fetch failed: {error}")),
    };
    let status = response.status().as_u16();
    if status == 401 || status == 403 {
        let body = response.text().await.unwrap_or_default();
        return if explicit_auth_failure(status, &body) {
            FetchOutcome::AuthFailed
        } else {
            FetchOutcome::Transient(format!("models fetch http {status}: {body}"))
        };
    }
    if !(200..300).contains(&status) {
        return FetchOutcome::Transient(format!("models fetch http {status}"));
    }
    match response.json::<Value>().await {
        Ok(value) => match parse_catalog_response(&value) {
            Ok(models) => FetchOutcome::Ok(models),
            Err(message) => FetchOutcome::Transient(message),
        },
        Err(error) => FetchOutcome::Transient(format!("models response not json: {error}")),
    }
}

pub async fn refresh_models(iii: &IIIClient, http: &reqwest::Client) -> Result<usize, Error> {
    let token = state::load_token(iii, STATE_SCOPE).await;
    let resolved = router_client::resolve(
        iii,
        PROVIDER_ID,
        token.as_deref(),
        Some(crate::register::CREDENTIAL_ENV_VAR),
    )
    .await?;
    let Some(credential) = resolved.credential else {
        router_client::reconcile(iii, PROVIDER_ID, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let url = models_url(resolved.api_url.as_deref().unwrap_or(DEFAULT_API_URL))
        .map_err(|error| upstream_unavailable(error.to_string()))?;
    match fetch_models(http, &url, credential_value(&credential).trim()).await {
        FetchOutcome::Ok(models) => {
            let count = models.len();
            router_client::reconcile(iii, PROVIDER_ID, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            router_client::reconcile(iii, PROVIDER_ID, vec![], token.as_deref()).await?;
            Ok(0)
        }
        FetchOutcome::Transient(message) => Err(upstream_unavailable(message)),
    }
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_request: RefreshModelsRequest| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    async fn stub(response: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut socket, _)) = listener.accept().await {
                let mut request = [0_u8; 16_384];
                let _ = socket.read(&mut request).await;
                let _ = socket.write_all(response.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
        });
        format!("http://{address}/models")
    }

    #[test]
    fn models_url_is_derived_from_base_or_either_generation_endpoint() {
        for url in [
            "https://api.commandcode.ai/provider/v1",
            "https://api.commandcode.ai/provider/v1/chat/completions",
            "https://api.commandcode.ai/provider/v1/messages",
        ] {
            assert_eq!(
                models_url(url).unwrap(),
                "https://api.commandcode.ai/provider/v1/models"
            );
        }
    }

    #[test]
    fn invalid_custom_url_never_falls_back_to_production() {
        assert!(matches!(
            models_url("localhost:8080/custom"),
            Err(ConfigError::InvalidApiUrl(_))
        ));
    }

    #[test]
    fn empty_or_malformed_success_payload_is_not_an_empty_reconcile() {
        assert!(parse_catalog_response(&serde_json::json!({})).is_err());
        assert!(parse_catalog_response(&serde_json::json!({ "data": [] })).is_err());
        assert!(parse_catalog_response(&serde_json::json!({
            "data": [{ "id": "gpt-test", "context_length": 4096 }]
        }))
        .is_ok());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_success_and_upstream_failures_preserve_last_known_good() {
        for response in [
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"data\":[]}",
            "HTTP/1.1 500 Internal Server Error\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"type\":\"server_error\"}}",
            "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"type\":\"upgrade_required\"}}",
        ] {
            let outcome = fetch_models(&reqwest::Client::new(), &stub(response).await, "test").await;
            assert!(matches!(outcome, FetchOutcome::Transient(_)));
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn only_explicitly_invalid_credentials_request_a_catalog_prune() {
        for response in [
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{}",
            "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"type\":\"authentication_error\"}}",
            "HTTP/1.1 403 Forbidden\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}",
        ] {
            let outcome = fetch_models(&reqwest::Client::new(), &stub(response).await, "test").await;
            assert!(matches!(outcome, FetchOutcome::AuthFailed));
        }
    }
}
