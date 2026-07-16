//! `memory::recall` — the read path. Zero LLM; the same scorer the
//! pre-generate hook uses, exposed so humans and agents can preview
//! exactly what a turn would be given.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::MemoryError;
use crate::types::Memory;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RecallRequest {
    /// Bank to search; the configured default when omitted.
    #[serde(default)]
    pub bank: Option<String>,
    pub query: String,
    /// Max memories returned (default: the configured recall_limit).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Also rank superseded/tombstoned records (the history view).
    #[serde(default)]
    pub include_superseded: Option<bool>,
    /// Only rank memories carrying this tag.
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ScoredMemory {
    pub memory: Memory,
    pub score: f32,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct RecallResponse {
    pub bank: String,
    pub memories: Vec<ScoredMemory>,
    /// Retrieval mode actually used — degradation is explicit, never
    /// silent. Currently always `bm25-entity`; a semantic signal joins
    /// when the router grows an embeddings surface.
    pub retrieval: String,
}

pub async fn recall(deps: &Deps, req: RecallRequest) -> Result<RecallResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let limit = req.limit.unwrap_or(cfg.recall_limit).min(100);
    let query_vec = crate::embed_client::query_vector(deps, &req.query).await;
    let retrieval = if query_vec.is_some() {
        "bm25-entity-semantic"
    } else {
        "bm25-entity"
    };
    let hits = bank
        .recall(
            &req.query,
            query_vec.as_deref(),
            limit,
            cfg.decay_half_life_days,
            req.include_superseded.unwrap_or(false),
        )
        .await;
    Ok(RecallResponse {
        bank: bank_name,
        memories: hits
            .into_iter()
            .filter(|(m, _)| {
                req.tag
                    .as_deref()
                    .is_none_or(|t| m.tags.iter().any(|x| x == t))
            })
            .map(|(memory, score)| ScoredMemory { memory, score })
            .collect(),
        retrieval: retrieval.into(),
    })
}
