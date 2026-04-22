use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use iii_sdk::{IIIError, III};
use serde_json::Value;

use super::state::{state_get, state_set};
use crate::config::IntrospectConfig;

pub fn build_handler(
    iii: Arc<III>,
    config: Arc<IntrospectConfig>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        let config = config.clone();
        Box::pin(async move { handle(&iii, config.cache_ttl_seconds).await })
    }
}

pub fn build_refresh_handler(
    iii: Arc<III>,
) -> impl Fn(Value) -> Pin<Box<dyn Future<Output = Result<Value, IIIError>> + Send>>
       + Send
       + Sync
       + 'static {
    move |_payload: Value| {
        let iii = iii.clone();
        Box::pin(async move {
            let fresh = build_topology(&iii).await?;
            state_set(&iii, "cache:topology", fresh.clone()).await?;
            Ok(fresh)
        })
    }
}

pub async fn handle(iii: &III, cache_ttl: u64) -> Result<Value, IIIError> {
    get_topology_cached(iii, cache_ttl).await
}

async fn get_topology_cached(iii: &III, cache_ttl: u64) -> Result<Value, IIIError> {
    let cached = state_get(iii, "cache:topology").await;
    if let Ok(ref val) = cached {
        if let Some(ts) = val
            .get("value")
            .and_then(|v| v.get("cached_at"))
            .and_then(|v| v.as_i64())
        {
            let now = chrono::Utc::now().timestamp();
            if now - ts < cache_ttl as i64 {
                return Ok(val.get("value").cloned().unwrap_or_default());
            }
        }
    }

    let fresh = build_topology(iii).await?;
    if let Err(e) = state_set(iii, "cache:topology", fresh.clone()).await {
        tracing::warn!(error = %e, "failed to update topology cache");
    }
    Ok(fresh)
}

pub async fn build_topology(iii: &III) -> Result<Value, IIIError> {
    let (functions_result, workers_result, triggers_result) = tokio::join!(
        iii.list_functions(),
        iii.list_workers(),
        iii.list_triggers(false),
    );

    let functions = functions_result?;
    let workers = workers_result?;
    let triggers = triggers_result?;

    let named_workers: Vec<_> = workers
        .iter()
        .filter(|w| w.name.is_some() || w.function_count > 0)
        .collect();

    let anonymous_count = workers.len() - named_workers.len();

    // Build one entry per worker keyed by the stable worker id, not by
    // display name. Two workers sharing the same name (common with
    // replicas or when `name` is unset and we fall back to id) would
    // otherwise collapse into a single row and hide the rest.
    let fpw_entries: Vec<Value> = named_workers
        .iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "worker": w.name.clone().unwrap_or_else(|| w.id.clone()),
                "function_count": w.function_count,
            })
        })
        .collect();

    let functions_json: Vec<Value> = functions
        .iter()
        .map(|f| {
            serde_json::json!({
                "id": f.function_id,
                "description": f.description,
                "request_format": f.request_format,
                "response_format": f.response_format,
            })
        })
        .collect();

    let workers_json: Vec<Value> = named_workers
        .iter()
        .map(|w| {
            serde_json::json!({
                "id": w.id,
                "name": w.name,
                "function_count": w.function_count,
                "functions": w.functions,
                "status": w.status,
            })
        })
        .collect();

    let triggers_json: Vec<Value> = triggers
        .iter()
        .map(|t| {
            serde_json::json!({
                "id": t.id,
                "trigger_type": t.trigger_type,
                "function_id": t.function_id,
                "config": t.config,
            })
        })
        .collect();

    let now = chrono::Utc::now().timestamp();

    Ok(serde_json::json!({
        "functions": functions_json,
        "workers": workers_json,
        "triggers": triggers_json,
        "stats": {
            "total_functions": functions.len(),
            "total_workers": named_workers.len(),
            "total_triggers": triggers.len(),
            "functions_per_worker": fpw_entries,
            "anonymous_connections": anonymous_count,
        },
        "cached_at": now,
    }))
}
