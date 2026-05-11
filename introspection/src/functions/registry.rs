use std::sync::Arc;

use iii_sdk::IIIError;
use serde_json::{json, Value};

use crate::config::Config;

pub async fn query(cfg: Arc<Config>, payload: Value) -> Result<Value, IIIError> {
    let q = payload
        .get("q")
        .and_then(|v| v.as_str())
        .ok_or_else(|| IIIError::Handler("missing required field: q".into()))?;
    let limit = payload.get("limit").and_then(|v| v.as_u64()).unwrap_or(20);

    let url = format!(
        "{}/registry/index.json",
        cfg.registry_url.trim_end_matches('/')
    );
    let resp = reqwest::Client::new()
        .get(&url)
        .timeout(std::time::Duration::from_millis(cfg.default_timeout_ms))
        .send()
        .await
        .map_err(|e| IIIError::Handler(format!("registry GET {url} failed: {e}")))?;

    let status = resp.status();
    if !status.is_success() {
        return Err(IIIError::Handler(format!(
            "registry GET {url} returned HTTP {status}"
        )));
    }

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !content_type.contains("json") {
        let preview = resp.text().await.unwrap_or_default();
        let preview = preview.chars().take(120).collect::<String>();
        return Err(IIIError::Handler(format!(
            "registry GET {url} returned non-JSON ({content_type}): {preview}"
        )));
    }

    let body: Value = resp
        .json()
        .await
        .map_err(|e| IIIError::Handler(format!("registry json parse failed: {e}")))?;

    let entries = body
        .get("workers")
        .or_else(|| body.get("entries"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let q_lc = q.to_lowercase();
    let matches: Vec<Value> = entries
        .into_iter()
        .filter(|e| {
            let name = e.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let desc = e.get("description").and_then(|d| d.as_str()).unwrap_or("");
            name.to_lowercase().contains(&q_lc) || desc.to_lowercase().contains(&q_lc)
        })
        .take(limit as usize)
        .collect();

    Ok(json!({
        "q": q,
        "registry_url": cfg.registry_url,
        "count": matches.len(),
        "results": matches,
    }))
}
