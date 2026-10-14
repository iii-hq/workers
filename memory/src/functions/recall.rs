//! `memory::recall` — the read path. Zero LLM; the same scorer the
//! pre-generate hook uses, exposed so humans and agents can preview
//! exactly what a turn would be given.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::WorkerConfig;
use crate::deps::Deps;
use crate::error::MemoryError;
use crate::store::Bank;
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
    /// silent: `bm25-entity`, or `bm25-entity-semantic` when the query
    /// was embedded through `router::embed`.
    pub retrieval: String,
}

pub async fn recall(deps: &Deps, req: RecallRequest) -> Result<RecallResponse, MemoryError> {
    let cfg = deps.config().await;
    let bank_name = req.bank.clone().unwrap_or_else(|| cfg.default_bank.clone());
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let query_vec = crate::embed_client::query_vector(deps, &req.query).await;
    let retrieval = if query_vec.is_some() {
        "bm25-entity-semantic"
    } else {
        "bm25-entity"
    };
    let memories = rank(&bank, &req, query_vec.as_deref(), &cfg).await;
    Ok(RecallResponse {
        bank: bank_name,
        memories,
        retrieval: retrieval.into(),
    })
}

/// Score, tag-filter, then cap — in that order: capping first would drop
/// tagged hits ranked past `limit` before the filter ever saw them.
async fn rank(
    bank: &Bank,
    req: &RecallRequest,
    query_vec: Option<&[f32]>,
    cfg: &WorkerConfig,
) -> Vec<ScoredMemory> {
    let limit = req.limit.unwrap_or(cfg.recall_limit).min(100);
    let fetch = if req.tag.is_some() { usize::MAX } else { limit };
    bank.recall(
        &req.query,
        query_vec,
        &cfg.embedding_model,
        fetch,
        cfg.decay_half_life_days,
        req.include_superseded.unwrap_or(false),
    )
    .await
    .into_iter()
    .filter(|(m, _)| {
        req.tag
            .as_deref()
            .is_none_or(|t| m.tags.iter().any(|x| x == t))
    })
    .take(limit)
    .map(|(memory, score)| ScoredMemory { memory, score })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;
    use crate::types::{fingerprint, now_ms, Confidence};

    fn memory(text: &str, tags: &[&str], age_days: u64) -> Memory {
        let at = now_ms() - age_days * 86_400_000;
        Memory {
            id: fingerprint(text),
            text: text.into(),
            entities: vec![],
            tags: tags.iter().map(|t| t.to_string()).collect(),
            confidence: Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: at,
            updated_at: at,
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        }
    }

    fn request(v: serde_json::Value) -> RecallRequest {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn tag_filter_reaches_hits_ranked_past_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        // Three equal lexical matches; the tagged one is oldest, so it
        // ranks last — past `limit: 1`.
        bank.commit(memory("coffee one", &[], 0)).await.unwrap();
        bank.commit(memory("coffee two", &[], 0)).await.unwrap();
        bank.commit(memory("coffee three", &["x"], 200))
            .await
            .unwrap();
        let cfg = WorkerConfig::default();

        let req = request(serde_json::json!({"query": "coffee", "limit": 1, "tag": "x"}));
        let hits = rank(&bank, &req, None, &cfg).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.text, "coffee three");

        // Untagged: `limit` still caps.
        let req = request(serde_json::json!({"query": "coffee", "limit": 2}));
        assert_eq!(rank(&bank, &req, None, &cfg).await.len(), 2);
    }
}
