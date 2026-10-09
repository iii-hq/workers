//! Live model discovery: `GET /v1/models` is the source of truth for the
//! catalog's id list — filtered to current-generation chat/reasoning
//! families, deduplicated against dated snapshots, enriched with the local
//! metadata table (xAI's API carries no capability data), and reconciled
//! through the router's single write path.
use crate::config::DEFAULT_API_URL;
use crate::curated::{base_id, enrich, is_legacy_generation};
use crate::errors::upstream_unavailable;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{
    DiscoveryCode, DiscoveryOutcome, DiscoveryRefreshResponse, DiscoveryReport,
    RefreshModelsRequest, RefreshModelsResponse,
};
use serde_json::Value;
use std::collections::HashSet;

/// Derive the models endpoint from the configured completions endpoint
/// (`…/v1/chat/completions` → `…/v1/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => "https://api.x.ai/v1/models".to_string(),
    }
}

/// Substrings marking a non-chat modality. Applied to xAI's catalog AND
/// custom xAI-compatible endpoints, since a local server may also expose
/// image/embedding models on /v1/models. `grok-imagine*` and `grok-2-image*`
/// are xAI's image/video families.
const NON_CHAT: [&str; 5] = ["imagine", "image", "video", "embed", "audio"];

/// True when the (lowercased) id is not an obvious non-chat modality.
fn is_chat_modality(lower: &str) -> bool {
    !NON_CHAT.iter().any(|term| lower.contains(term))
}

pub fn is_chat_model(id: &str) -> bool {
    let lower = id.to_ascii_lowercase();
    // xAI's text/chat family is `grok-*`; the non-chat gate drops the
    // image/video members (grok-imagine, grok-2-image).
    lower.starts_with("grok-") && is_chat_modality(&lower)
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    let raw_ids: Vec<String> = json
        .get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter_map(|raw| {
                    raw.get("id")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string)
                })
                .collect()
        })
        .unwrap_or_default();

    let filtered: Vec<String> = raw_ids
        .iter()
        .filter(|id| is_chat_model(id) && !is_legacy_generation(id))
        .cloned()
        .collect();

    // Custom xAI-compatible servers (LMStudio, Ollama, vLLM) serve
    // arbitrarily-named models that the grok family gate and the
    // legacy denylist would discard entirely. When xAI-style filtering
    // admits nothing but the server did return models, it is plainly not
    // xAI: list everything it serves, only dropping non-chat modalities.
    // Real xAI always carries a current family, so this never triggers
    // against api.x.ai.
    let ids = if filtered.is_empty() && !raw_ids.is_empty() {
        raw_ids
            .into_iter()
            .filter(|id| is_chat_modality(&id.to_ascii_lowercase()))
            .collect()
    } else {
        filtered
    };

    // Dated snapshots are pinning artifacts: when the undated alias is also
    // live (grok-4 next to grok-4-0709), keep only the alias so the
    // picker carries one row per model.
    let live: HashSet<&str> = ids.iter().map(String::as_str).collect();
    ids.iter()
        .filter(|id| {
            let base = base_id(id);
            base == id.as_str() || !live.contains(base)
        })
        .map(|id| enrich(id))
        .collect()
}

#[derive(Debug)]
struct FetchOutcome {
    models: Vec<Model>,
    outcome: DiscoveryOutcome,
    http_status: Option<u16>,
    code: Option<DiscoveryCode>,
}

impl FetchOutcome {
    fn failure(
        outcome: DiscoveryOutcome,
        http_status: Option<u16>,
        code: Option<DiscoveryCode>,
    ) -> Self {
        Self {
            models: vec![],
            outcome,
            http_status,
            code,
        }
    }
}

/// Inspect bounded upstream data locally; emit only enums from the allowlist.
fn classify_error(status: u16, body: &Value) -> FetchOutcome {
    let error = body.get("error").unwrap_or(body);
    let code = error
        .get("code")
        .and_then(Value::as_str)
        .or_else(|| body.get("code").and_then(Value::as_str))
        .or_else(|| error.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .or_else(|| body.get("message").and_then(Value::as_str))
        .or_else(|| error.as_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let safe_code = match code.as_str() {
        "permission-denied" | "permission_denied" => Some(DiscoveryCode::PermissionDenied),
        "invalid-api-key" | "invalid_api_key" => Some(DiscoveryCode::InvalidApiKey),
        "insufficient-credits" | "insufficient_credits" => Some(DiscoveryCode::InsufficientCredits),
        "spending-limit-exceeded" | "spending_limit_exceeded" => {
            Some(DiscoveryCode::SpendingLimitExceeded)
        }
        "rate-limit-exceeded" | "rate_limit_exceeded" => Some(DiscoveryCode::RateLimitExceeded),
        _ => None,
    };
    let billing = matches!(
        safe_code,
        Some(DiscoveryCode::InsufficientCredits | DiscoveryCode::SpendingLimitExceeded)
    ) || [
        "no credits",
        "out of credits",
        "insufficient credits",
        "credits exhausted",
        "used all available credits",
        "spending limit",
        "spend limit",
    ]
    .iter()
    .any(|term| message.contains(term));
    let outcome = match status {
        401 => DiscoveryOutcome::Authentication,
        402 | 403 if billing => DiscoveryOutcome::Billing,
        403 => DiscoveryOutcome::Permission,
        429 => DiscoveryOutcome::RateLimit,
        _ => DiscoveryOutcome::Unavailable,
    };
    FetchOutcome::failure(outcome, Some(status), safe_code)
}

async fn fetch_live_models(
    http: &reqwest::Client,
    url: &str,
    credential_value: &str,
) -> FetchOutcome {
    let mut resp = match http
        .get(url)
        .header("authorization", format!("Bearer {credential_value}"))
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return FetchOutcome::failure(DiscoveryOutcome::Unavailable, None, None),
    };
    let status = resp.status().as_u16();
    // Bound buffering, including malicious compatible endpoints. Never log body or URL.
    let mut bytes = Vec::new();
    loop {
        match resp.chunk().await {
            Ok(Some(chunk)) if bytes.len() + chunk.len() <= 1_048_576 => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            Ok(Some(_)) => {
                return FetchOutcome::failure(DiscoveryOutcome::InvalidResponse, Some(status), None)
            }
            Err(_) => {
                return FetchOutcome::failure(DiscoveryOutcome::Unavailable, Some(status), None)
            }
        }
    }
    let json = serde_json::from_slice::<Value>(&bytes);
    if !(200..300).contains(&status) {
        return classify_error(status, &json.unwrap_or(Value::Null));
    }
    let Ok(json) = json else {
        return FetchOutcome::failure(DiscoveryOutcome::InvalidResponse, Some(status), None);
    };
    let valid = json
        .get("data")
        .and_then(Value::as_array)
        .is_some_and(|rows| {
            rows.iter().all(|row| {
                row.get("id")
                    .and_then(Value::as_str)
                    .is_some_and(|id| !id.is_empty())
            })
        });
    if !valid {
        return FetchOutcome::failure(DiscoveryOutcome::InvalidResponse, Some(status), None);
    }
    let models = parse_live_models(&json);
    FetchOutcome {
        outcome: if models.is_empty() {
            DiscoveryOutcome::Empty
        } else {
            DiscoveryOutcome::Success
        },
        models,
        http_status: Some(status),
        code: None,
    }
}

pub async fn refresh_models(
    iii: &IIIClient,
    http: &reqwest::Client,
) -> Result<DiscoveryRefreshResponse, Error> {
    let token = state::load_token(iii).await;
    let (resolved, attempt) = router_client::begin_discovery(iii, token.as_deref()).await?;
    let fetched = if let Some(credential) = resolved.credential {
        let url = models_url(resolved.api_url.as_deref().unwrap_or(DEFAULT_API_URL));
        fetch_live_models(http, &url, crate::config::credential_parts(&credential)).await
    } else {
        FetchOutcome::failure(DiscoveryOutcome::NotConfigured, None, None)
    };
    let outcome = fetched.outcome;
    let count = if let Some(attempt) = attempt {
        router_client::complete_discovery(
            iii,
            fetched.models,
            DiscoveryReport {
                attempt,
                outcome,
                http_status: fetched.http_status,
                code: fetched.code,
            },
            token.as_deref(),
        )
        .await?
    } else {
        // Rolling-upgrade fallback: older routers cannot store diagnostics. Do not
        // disguise a failure as empty success or destroy their last good catalog.
        if outcome.preserves_catalog() {
            return Err(upstream_unavailable(format!(
                "model discovery: {outcome:?}"
            )));
        }
        let count = fetched.models.len();
        router_client::reconcile(iii, fetched.models, token.as_deref()).await?;
        count
    };
    Ok(DiscoveryRefreshResponse {
        refreshed: RefreshModelsResponse {
            ok: outcome.is_success(),
            count,
        },
        discovery: outcome,
    })
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<DiscoveryRefreshResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req| {
        let (iii, http) = (iii.clone(), http.clone());
        Box::pin(async move { refresh_models(&iii, &http).await })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_without_exposing_upstream_data() {
        use serde_json::json;
        let cases = [
            (
                403,
                json!({"code":"permission-denied", "error":"Your team team-SENSITIVE has no credits or has reached its monthly spending limit"}),
                DiscoveryOutcome::Billing,
            ),
            (
                403,
                json!({"error":{"code":"permission-denied", "message":"Team team-SENSITIVE has used all available credits; key sk-SENSITIVE https://sensitive.invalid"}}),
                DiscoveryOutcome::Billing,
            ),
            (
                403,
                json!({"error":{"code":"permission-denied", "message":"Forbidden"}}),
                DiscoveryOutcome::Permission,
            ),
            (
                401,
                json!({"error":{"code":"invalid_api_key"}}),
                DiscoveryOutcome::Authentication,
            ),
            (
                429,
                json!({"error":{"code":"rate_limit_exceeded"}}),
                DiscoveryOutcome::RateLimit,
            ),
            (
                503,
                json!({"error":"private upstream details"}),
                DiscoveryOutcome::Unavailable,
            ),
        ];
        for (status, body, expected) in cases {
            let fetched = classify_error(status, &body);
            assert_eq!(fetched.outcome, expected);
            let report = DiscoveryReport {
                attempt: "test".into(),
                outcome: fetched.outcome,
                http_status: fetched.http_status,
                code: fetched.code,
            };
            let wire = serde_json::to_string(&report).unwrap();
            for secret in ["SENSITIVE", "https://", "private upstream"] {
                assert!(!wire.contains(secret));
            }
        }
    }

    #[tokio::test]
    async fn real_http_empty_invalid_success_and_transport_are_distinct() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (body, expected) in [
            ("{\"data\":[]}", DiscoveryOutcome::Empty),
            ("{}", DiscoveryOutcome::InvalidResponse),
            ("not json", DiscoveryOutcome::InvalidResponse),
            ("{\"data\":[{}]}", DiscoveryOutcome::InvalidResponse),
            (
                "{\"data\":[{\"id\":\"grok-4\"}]}",
                DiscoveryOutcome::Success,
            ),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 2048];
                socket.read(&mut request).await.unwrap();
                socket.write_all(format!("HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).await.unwrap();
            });
            let fetched = fetch_live_models(
                &reqwest::Client::new(),
                &format!("http://{addr}/models"),
                "fixture-key",
            )
            .await;
            assert_eq!(fetched.outcome, expected);
            server.await.unwrap();
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        drop(listener);
        assert_eq!(
            fetch_live_models(
                &reqwest::Client::new(),
                &format!("http://{addr}/models"),
                "fixture-key"
            )
            .await
            .outcome,
            DiscoveryOutcome::Unavailable
        );
    }

    #[test]
    fn models_url_derives_from_completions_endpoint() {
        assert_eq!(
            models_url("https://api.x.ai/v1/chat/completions"),
            "https://api.x.ai/v1/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.x.ai/v1/models"
        );
    }

    #[test]
    fn chat_family_filter_admits_grok_only() {
        assert!(is_chat_model("grok-4.3"));
        assert!(is_chat_model("grok-4"));
        assert!(is_chat_model("grok-3-mini"));
        assert!(is_chat_model("grok-code-fast-1"));
        assert!(is_chat_model("grok-build-0.1"));
        assert!(!is_chat_model("text-embedding-3-large"));
        assert!(!is_chat_model("grok-imagine-image"));
        assert!(!is_chat_model("grok-imagine-video-1.5"));
        assert!(!is_chat_model("grok-2-image-1212"));
        assert!(!is_chat_model("gpt-5.2"));
    }

    #[test]
    fn parses_ids_skipping_malformed_non_chat_and_legacy_rows() {
        let json = serde_json::json!({
            "data": [
                { "id": "grok-4.3", "object": "model" },
                { "id": "" },
                { "object": "model" },
                { "id": "grok-imagine-image", "object": "model" },
                { "id": "grok-2-1212", "object": "model" },
                { "id": "grok-2-vision-1212", "object": "model" },
            ]
        });
        let models = parse_live_models(&json);
        // grok-2 generations are legacy; imagine is non-chat; only grok-4.3 stays.
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id, "grok-4.3");
        assert_eq!(models[0].display_name.as_deref(), Some("Grok 4.3"));
    }

    #[test]
    fn dated_snapshot_drops_when_undated_alias_is_live() {
        let json = serde_json::json!({
            "data": [
                { "id": "grok-4", "object": "model" },
                { "id": "grok-4-0709", "object": "model" },
                { "id": "grok-3-mini-0625", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["grok-4", "grok-3-mini-0625"]);
    }

    #[test]
    fn custom_compatible_endpoint_lists_all_when_xai_filter_admits_nothing() {
        // An LMStudio/Ollama-style catalog: no id passes the grok family gate,
        // so xAI filtering would yield an empty list. The fallback lists
        // everything the server serves, dropping only the embedding model.
        let json = serde_json::json!({
            "data": [
                { "id": "qwen2.5-coder-7b-instruct", "object": "model" },
                { "id": "llama-3.2-3b-instruct", "object": "model" },
                { "id": "nomic-embed-text-v1.5", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["qwen2.5-coder-7b-instruct", "llama-3.2-3b-instruct"]);
        // Unknown families get conservative defaults, never vanish.
        assert_eq!(parse_live_models(&json)[0].context_window, 131_072);
    }

    #[test]
    fn xai_filter_still_applies_when_some_current_model_is_present() {
        // Mixed catalog with a current grok model present → normal xAI
        // filtering (legacy + non-chat dropped), NOT the fallback.
        let json = serde_json::json!({
            "data": [
                { "id": "grok-4.3", "object": "model" },
                { "id": "grok-2-1212", "object": "model" },
                { "id": "text-embedding-3-large", "object": "model" },
            ]
        });
        let ids: Vec<String> = parse_live_models(&json).into_iter().map(|m| m.id).collect();
        assert_eq!(ids, ["grok-4.3"]);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&serde_json::json!({})).is_empty());
        assert!(parse_live_models(&serde_json::json!({ "data": "nope" })).is_empty());
    }
}
