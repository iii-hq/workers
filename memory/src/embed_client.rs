//! Fail-soft client for `router::embed` plus the vector backfill task.
//! Embeddings are derived data: every failure here degrades recall to
//! BM25 + entities (and recall responses say so), never blocks a turn,
//! and never touches memory integrity.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::deps::Deps;

/// Query-path budget: the pre-generate hook runs inside a 3s harness
/// timeout, so the embed call must fail fast and fail open.
const QUERY_EMBED_TIMEOUT_MS: u64 = 2_000;
/// Backfill batches are off every hot path; give the provider room.
const BACKFILL_TIMEOUT_MS: u64 = 20_000;
const BACKFILL_BATCH: usize = 64;
/// Backfill rounds per pass — caps one pass at ~640 embeddings so a huge
/// legacy bank fills over several passes instead of one giant burst.
const BACKFILL_ROUNDS: usize = 10;

pub async fn embed_texts(
    iii: &IIIClient,
    model: &str,
    texts: &[String],
    timeout_ms: u64,
) -> Option<Vec<Vec<f32>>> {
    let mut payload = json!({ "input": texts });
    if !model.trim().is_empty() {
        payload["model"] = json!(model);
    }
    let request = TriggerRequest {
        function_id: "router::embed".into(),
        payload,
        action: None,
        timeout_ms: Some(timeout_ms),
    };
    let reply = match iii.namespace() {
        Some(ns) => iii.trigger(request.namespace(ns)).await,
        None => iii.trigger(request).await,
    }
    .ok()?;
    let embeddings: Vec<Vec<f32>> =
        serde_json::from_value(reply.get("embeddings").cloned().unwrap_or(Value::Null)).ok()?;
    if embeddings.len() == texts.len() {
        Some(embeddings)
    } else {
        None
    }
}

/// Embed the query for one recall. `None` (disabled, router absent, no
/// embed provider, timeout) means lexical-only recall this turn.
pub async fn query_vector(deps: &Deps, query: &str) -> Option<Vec<f32>> {
    let cfg = deps.config().await;
    if !cfg.embeddings_enabled {
        return None;
    }
    embed_texts(
        &deps.iii,
        &cfg.embedding_model,
        &[query.to_string()],
        QUERY_EMBED_TIMEOUT_MS,
    )
    .await
    .and_then(|mut v| v.pop())
}

/// One backfill pass over a single bank: embed live memories that have no
/// vector yet, in bounded batches. Spawn-and-forget.
pub async fn backfill_bank(deps: Arc<Deps>, bank_name: String) {
    let cfg = deps.config().await;
    if !cfg.embeddings_enabled {
        return;
    }
    let store = deps.store().await;
    let Ok(bank) = store.bank(&bank_name).await else {
        return;
    };
    let mut total = 0usize;
    for _ in 0..BACKFILL_ROUNDS {
        let pending = bank.unembedded(BACKFILL_BATCH, &cfg.embedding_model).await;
        if pending.is_empty() {
            break;
        }
        let texts: Vec<String> = pending.iter().map(|f| f.text.clone()).collect();
        let Some(vectors) =
            embed_texts(&deps.iii, &cfg.embedding_model, &texts, BACKFILL_TIMEOUT_MS).await
        else {
            // Router or provider unavailable; a later pass retries.
            return;
        };
        for (memory, v) in pending.iter().zip(vectors) {
            if let Err(e) = bank.set_vector(&memory.id, &cfg.embedding_model, v).await {
                tracing::warn!(bank = %bank_name, error = %e, "vector write failed");
                return;
            }
            total += 1;
        }
    }
    if total > 0 {
        tracing::info!(bank = %bank_name, embedded = total, "vector backfill pass complete");
    }
}

/// Boot-time catch-up across every bank, delayed so llm-router and its
/// providers finish their own boot first.
pub fn backfill_all(deps: Arc<Deps>) {
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
        let banks = deps.store().await.bank_names().await;
        for bank in banks {
            backfill_bank(deps.clone(), bank).await;
        }
    });
}
