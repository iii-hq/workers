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
    /// Bank to search; omitted: the calling session's bank, else the configured default.
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
    /// `hybrid` (default) is the fused scorer the hook uses. `lexical` and
    /// `semantic` return ONE raw arm (plain BM25, or cosine as the score)
    /// for callers that fuse arms themselves — consolidation interleaves
    /// both so a near-duplicate is never buried by the fused rank.
    #[serde(default)]
    pub mode: RecallMode,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum RecallMode {
    #[default]
    Hybrid,
    Lexical,
    Semantic,
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
    /// silent. `hybrid`: `bm25-entity`, or `bm25-entity-semantic` when the
    /// query was embedded through `router::embed`. `lexical`: `lexical`.
    /// `semantic`: `semantic`, or `semantic-unavailable` (empty result —
    /// NOT "no matches") when the query could not be embedded.
    pub retrieval: String,
}

pub async fn recall(deps: &Deps, mut req: RecallRequest) -> Result<RecallResponse, MemoryError> {
    req.query = crate::store::cap_query(&req.query);
    let cfg = deps.config().await;
    let bank_name = crate::scope::request_bank(deps, req.bank.clone()).await?;
    let store = deps.store().await;
    let bank = store.bank(&bank_name).await?;
    let query_vec = match req.mode {
        RecallMode::Lexical => None,
        _ => crate::embed_client::query_vector(deps, &req.query).await,
    };
    let (qv, query_model) = crate::embed_client::QueryVector::split(query_vec.as_ref());
    let memories = rank(&bank, &req, qv, query_model, &cfg).await;
    Ok(RecallResponse {
        bank: bank_name,
        memories,
        retrieval: retrieval_label(req.mode, query_vec.is_some()).into(),
    })
}

fn retrieval_label(mode: RecallMode, embedded: bool) -> &'static str {
    match (mode, embedded) {
        (RecallMode::Hybrid, true) => "bm25-entity-semantic",
        (RecallMode::Hybrid, false) => "bm25-entity",
        (RecallMode::Lexical, _) => "lexical",
        (RecallMode::Semantic, true) => "semantic",
        (RecallMode::Semantic, false) => "semantic-unavailable",
    }
}

/// Score, tag-filter, then cap — in that order: capping first would drop
/// tagged hits ranked past `limit` before the filter ever saw them.
/// `query_model` is the model the provider reported for `query_vec`: only
/// vectors stored under exactly that model are comparable with it.
async fn rank(
    bank: &Bank,
    req: &RecallRequest,
    query_vec: Option<&[f32]>,
    query_model: &str,
    cfg: &WorkerConfig,
) -> Vec<ScoredMemory> {
    let limit = req.limit.unwrap_or(cfg.recall_limit).min(100);
    let fetch = if req.tag.is_some() { usize::MAX } else { limit };
    let include = req.include_superseded.unwrap_or(false);
    let hits = match req.mode {
        RecallMode::Hybrid => {
            bank.recall(
                &req.query,
                query_vec,
                query_model,
                fetch,
                cfg.decay_half_life_days,
                include,
            )
            .await
        }
        RecallMode::Lexical => bank.lexical(&req.query, fetch, include).await,
        RecallMode::Semantic => match query_vec {
            Some(qv) => bank.semantic(qv, query_model, fetch, include).await,
            None => Vec::new(),
        },
    };
    hits.into_iter()
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
            stated_at: None,
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
        let hits = rank(&bank, &req, None, "", &cfg).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.text, "coffee three");

        // Untagged: `limit` still caps.
        let req = request(serde_json::json!({"query": "coffee", "limit": 2}));
        assert_eq!(rank(&bank, &req, None, "", &cfg).await.len(), 2);
    }

    #[tokio::test]
    async fn modes_pick_the_raw_lexical_or_semantic_arm() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let old_twin = memory("kiwi mango smoothie recipe", &[], 400);
        let fresh = memory("kiwi with extra filler words here", &[], 0);
        bank.commit(old_twin.clone()).await.unwrap();
        bank.commit(fresh.clone()).await.unwrap();
        bank.set_vector(&fresh.id, "", &fresh.text, vec![1.0, 0.0])
            .await
            .unwrap();
        let cfg = WorkerConfig::default();

        let lexical =
            request(serde_json::json!({"query": "kiwi mango smoothie recipe", "mode": "lexical"}));
        let hits = rank(&bank, &lexical, None, "", &cfg).await;
        assert_eq!(
            hits[0].memory.id, old_twin.id,
            "raw BM25: age does not sink the twin"
        );
        // Wired to the raw arm, not the fused scorer: hybrid decays the
        // 400-day-old twin's score by the recency factor (x0.8).
        let hybrid =
            request(serde_json::json!({"query": "kiwi mango smoothie recipe", "mode": "hybrid"}));
        let fused = rank(&bank, &hybrid, None, "", &cfg).await;
        let fused_twin = fused.iter().find(|h| h.memory.id == old_twin.id).unwrap();
        assert!(
            hits[0].score > 1.2 * fused_twin.score,
            "lexical mode must not route through the fused scorer"
        );

        let semantic = request(serde_json::json!({"query": "anything", "mode": "semantic"}));
        assert!(
            rank(&bank, &semantic, None, "", &cfg).await.is_empty(),
            "no vector, no semantic arm"
        );
        let hits = rank(&bank, &semantic, Some(&[1.0, 0.0]), "", &cfg).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].memory.id, fresh.id);
        assert!(
            (hits[0].score - 1.0).abs() < 1e-6,
            "semantic scores are raw cosine"
        );

        assert_eq!(retrieval_label(RecallMode::Lexical, false), "lexical");
        assert_eq!(retrieval_label(RecallMode::Semantic, true), "semantic");
        assert_eq!(
            retrieval_label(RecallMode::Semantic, false),
            "semantic-unavailable"
        );
        assert_eq!(
            retrieval_label(RecallMode::Hybrid, true),
            "bm25-entity-semantic"
        );
        assert_eq!(retrieval_label(RecallMode::Hybrid, false), "bm25-entity");
    }
}
