//! Live model discovery: `GET /models` on the Copilot API endpoint is the
//! source of truth for the catalog slice — ids AND metadata, since the
//! listing is capability-structured (see [`crate::catalog`]). Admission
//! keeps the slice harness-usable: chat models with tool support that the
//! subscription actually enables. Everything admitted is reconciled through
//! the router's single write path.
use crate::auth;
use crate::catalog::model_from_row;
use crate::errors::upstream_unavailable;
use crate::exchange::{fresh_bearer, BearerCache, ExchangeError, DEFAULT_TOKEN_URL};
use crate::request::client_headers;
use crate::{router_client, state};
use futures::future::BoxFuture;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use llm_router::types::model::Model;
use llm_router::types::router::{RefreshModelsRequest, RefreshModelsResponse};
use serde_json::Value;

/// Derive the models endpoint from the chat completions endpoint
/// (`…/chat/completions` → `…/models`).
pub fn models_url(api_url: &str) -> String {
    match api_url.strip_suffix("/chat/completions") {
        Some(base) => format!("{base}/models"),
        None => "https://api.githubcopilot.com/models".to_string(),
    }
}

/// Harness-usable models only. Four gates, each removing rows that would be
/// dead or wrong in the picker:
///
/// - chat type with `tool_calls` — the agent loop needs both.
/// - `policy.state != "disabled"` — the real per-account gate. A disabled
///   policy means the model has not been approved for this account and a
///   call fails with "model not supported"; absent policy means none needed.
///   (`model_picker_enabled` is deliberately NOT used: it reflects an
///   editor-side picker preference and reads `false` for every row on
///   accounts that have never toggled models in an editor.)
/// - `supported_endpoints` must include `/chat/completions` when the row
///   declares them — some models are Messages-API only.
/// - preview rows with no `model_picker_category` are the editor's internal
///   feature models (search, compaction, exec agents), not chat models.
pub fn admit(row: &Value) -> bool {
    let chat = row
        .pointer("/capabilities/type")
        .and_then(Value::as_str)
        .map(|t| t == "chat")
        .unwrap_or(false);
    let tools = row
        .pointer("/capabilities/supports/tool_calls")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let policy_allows = row
        .pointer("/policy/state")
        .and_then(Value::as_str)
        .map(|s| s != "disabled")
        .unwrap_or(true);
    let chat_endpoint = match row.get("supported_endpoints").and_then(Value::as_array) {
        Some(eps) if !eps.is_empty() => eps
            .iter()
            .filter_map(Value::as_str)
            .any(|e| e == "/chat/completions"),
        _ => true,
    };
    let internal_preview = row.get("preview").and_then(Value::as_bool).unwrap_or(false)
        && row
            .get("model_picker_category")
            .and_then(Value::as_str)
            .is_none();
    chat && tools && policy_allows && chat_endpoint && !internal_preview
}

/// Probe concurrency: small enough to look like a client checking its model
/// list, large enough that a refresh stays quick.
const PROBE_CONCURRENCY: usize = 4;

/// Ask the upstream whether this account may actually call `model`.
///
/// Entitlement is per-plan and NOT exposed anywhere in the listing (two
/// models from the same vendor with identical `policy`, `capabilities`, and
/// picker category differ: one answers, the other returns
/// `model_not_supported`). A one-token request is the only authoritative
/// check. Refusals are rejected before generation, so they consume no quota,
/// and successes cost a single token on models the plan already includes.
async fn probe_model(http: &reqwest::Client, api_url: &str, bearer: &str, model: &str) -> bool {
    let mut req = http
        .post(api_url)
        .header("authorization", format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .header("x-initiator", "agent");
    for (name, value) in client_headers() {
        req = req.header(name, value);
    }
    let body = serde_json::json!({
        "model": model,
        "max_tokens": 1,
        "messages": [{ "role": "user", "content": "hi" }],
    });
    let Ok(resp) = req.json(&body).send().await else {
        // Network trouble is not evidence of unavailability — keep the model
        // and let the real call surface (and self-heal) any refusal.
        return true;
    };
    if resp.status().is_success() {
        return true;
    }
    let text = resp.text().await.unwrap_or_default();
    !crate::errors::is_model_not_supported(&text)
}

/// Keep only the models this account can actually call. Preserves listing
/// order so the picker stays stable.
async fn retain_callable(
    http: &reqwest::Client,
    api_url: &str,
    bearer: &str,
    models: Vec<Model>,
) -> Vec<Model> {
    let mut callable = Vec::with_capacity(models.len());
    for chunk in models.chunks(PROBE_CONCURRENCY) {
        let checks = chunk.iter().map(|m| {
            let id = crate::catalog::upstream_id(&m.id).to_string();
            async move { probe_model(http, api_url, bearer, &id).await }
        });
        for (model, ok) in chunk.iter().zip(futures::future::join_all(checks).await) {
            if ok {
                callable.push(model.clone());
            }
        }
    }
    callable
}

pub fn parse_live_models(json: &Value) -> Vec<Model> {
    json.get("data")
        .and_then(Value::as_array)
        .map(|rows| {
            rows.iter()
                .filter(|row| admit(row))
                .filter_map(model_from_row)
                .collect()
        })
        .unwrap_or_default()
}

enum FetchOutcome {
    Ok(Vec<Model>),
    AuthFailed,
    Transient(String),
}

async fn fetch_live_models(http: &reqwest::Client, url: &str, bearer: &str) -> FetchOutcome {
    let mut req = http
        .get(url)
        .header("authorization", format!("Bearer {bearer}"));
    for (name, value) in client_headers() {
        req = req.header(name, value);
    }
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return FetchOutcome::Transient(format!("models fetch failed: {e}")),
    };
    let status = resp.status().as_u16();
    if status == 401 || status == 403 {
        return FetchOutcome::AuthFailed;
    }
    if !(200..300).contains(&status) {
        return FetchOutcome::Transient(format!("models fetch http {status}"));
    }
    match resp.json::<Value>().await {
        Ok(v) => FetchOutcome::Ok(parse_live_models(&v)),
        Err(e) => FetchOutcome::Transient(format!("models response not json: {e}")),
    }
}

/// The refresh flow; returns the reconciled slice size.
pub async fn refresh_models(
    iii: &IIIClient,
    http: &reqwest::Client,
    cache: &BearerCache,
) -> Result<usize, Error> {
    let token = state::load_token(iii).await;
    let resolved = router_client::resolve(iii, token.as_deref()).await?;

    let Some(credential) = auth::resolve_credential(iii).await else {
        // Never signed in anywhere this worker can see: prune the slice so
        // the picker reflects reality instead of stale rows.
        router_client::reconcile(iii, vec![], token.as_deref()).await?;
        return Ok(0);
    };
    let bearer = match fresh_bearer(http, cache, &credential, DEFAULT_TOKEN_URL).await {
        Ok(b) => b,
        Err(ExchangeError::Unauthorized(_)) => {
            // Revoked login / no Copilot access: the models are unusable.
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            return Ok(0);
        }
        Err(ExchangeError::Transient(msg)) => return Err(upstream_unavailable(msg)),
    };

    // Operator api_url override wins over the exchange endpoint (same
    // precedence the stream path uses).
    let api_url = resolved
        .api_url
        .clone()
        .or_else(|| bearer.api_url.clone())
        .unwrap_or_else(|| crate::exchange::DEFAULT_API_URL.to_string());
    match fetch_live_models(http, &models_url(&api_url), &bearer.token).await {
        FetchOutcome::Ok(models) => {
            let listed = models.len();
            // The listing advertises models this plan cannot call; verify
            // rather than guess, so the picker only ever offers what works.
            let models = retain_callable(http, &api_url, &bearer.token, models).await;
            let count = models.len();
            if count < listed {
                println!(
                    "[provider-github-copilot] {} of {listed} listed models are available on this plan",
                    count
                );
            }
            router_client::reconcile(iii, models, token.as_deref()).await?;
            Ok(count)
        }
        FetchOutcome::AuthFailed => {
            router_client::reconcile(iii, vec![], token.as_deref()).await?;
            Ok(0)
        }
        // Blip: keep the previous slice (spec § reconcile-to-empty guidance).
        FetchOutcome::Transient(msg) => Err(upstream_unavailable(msg)),
    }
}

pub fn make_refresh_models(
    iii: IIIClient,
    http: reqwest::Client,
    cache: BearerCache,
) -> impl Fn(RefreshModelsRequest) -> BoxFuture<'static, Result<RefreshModelsResponse, Error>>
       + Send
       + Sync
       + 'static {
    move |_req: RefreshModelsRequest| {
        let (iii, http, cache) = (iii.clone(), http.clone(), cache.clone());
        Box::pin(async move {
            let count = refresh_models(&iii, &http, &cache).await?;
            Ok(RefreshModelsResponse { ok: true, count })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn row(id: &str, model_type: &str, tools: bool) -> Value {
        json!({
            "id": id,
            "capabilities": {
                "type": model_type,
                "supports": { "tool_calls": tools }
            },
            // every row on an account that never toggled models in an editor
            // reads false here; admission must not gate on it
            "model_picker_enabled": false
        })
    }

    #[test]
    fn models_url_derives_from_completions_endpoint() {
        assert_eq!(
            models_url("https://api.githubcopilot.com/chat/completions"),
            "https://api.githubcopilot.com/models"
        );
        assert_eq!(
            models_url("http://127.0.0.1:9999/v1/chat/completions"),
            "http://127.0.0.1:9999/v1/models"
        );
        // unrecognized shape falls back to the public endpoint
        assert_eq!(
            models_url("https://proxy.example/custom"),
            "https://api.githubcopilot.com/models"
        );
    }

    #[test]
    fn admission_requires_chat_and_tools() {
        assert!(admit(&row("gpt-5.2", "chat", true)));
        // embeddings and other non-chat types are dead in the agent loop
        assert!(!admit(&row("text-embedding-3-small", "embeddings", false)));
        // no tool support → dead in the agent loop
        assert!(!admit(&row("chatty", "chat", false)));
        // missing capabilities tree → not admitted
        assert!(!admit(&json!({ "id": "bare" })));
    }

    #[test]
    fn admission_honors_the_account_policy_gate() {
        let mut disabled = row("claude-x", "chat", true);
        disabled["policy"] = json!({ "state": "disabled" });
        assert!(!admit(&disabled), "policy-disabled model would 404");

        let mut enabled = row("claude-y", "chat", true);
        enabled["policy"] = json!({ "state": "enabled" });
        assert!(admit(&enabled));

        // absent policy = none required
        assert!(admit(&row("gpt-4o", "chat", true)));
    }

    #[test]
    fn admission_requires_a_chat_completions_endpoint_when_declared() {
        let mut messages_only = row("claude-z", "chat", true);
        messages_only["supported_endpoints"] = json!(["/v1/messages"]);
        assert!(!admit(&messages_only));

        let mut both = row("claude-w", "chat", true);
        both["supported_endpoints"] = json!(["/v1/messages", "/chat/completions"]);
        assert!(admit(&both));

        // absent or empty list: no constraint declared
        let mut empty = row("gpt-4o", "chat", true);
        empty["supported_endpoints"] = json!([]);
        assert!(admit(&empty));
    }

    #[test]
    fn admission_drops_the_editors_internal_preview_models() {
        // preview + no picker category = an editor feature model (search,
        // compaction, exec agents), not a chat model
        let mut internal = row("copilot-search-a", "chat", true);
        internal["preview"] = json!(true);
        assert!(!admit(&internal));

        // a real preview model the picker categorizes stays
        let mut public_preview = row("gemini-3.1-pro-preview", "chat", true);
        public_preview["preview"] = json!(true);
        public_preview["model_picker_category"] = json!("versatile");
        assert!(admit(&public_preview));
    }

    #[test]
    fn parses_rows_applying_admission_and_prefix() {
        let listing = json!({ "data": [
            row("gpt-5.2", "chat", true),
            row("claude-sonnet-4.6", "chat", true),
            row("text-embedding-3-small", "embeddings", false),
            { "id": "", "capabilities": { "type": "chat",
              "supports": { "tool_calls": true } } },
        ]});
        let ids: Vec<String> = parse_live_models(&listing)
            .into_iter()
            .map(|m| m.id)
            .collect();
        assert_eq!(ids, ["copilot/gpt-5.2", "copilot/claude-sonnet-4.6"]);
    }

    #[test]
    fn missing_or_malformed_data_yields_empty() {
        assert!(parse_live_models(&json!({})).is_empty());
        assert!(parse_live_models(&json!({ "data": "nope" })).is_empty());
    }
}
