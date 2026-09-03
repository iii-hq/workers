use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use model2vec_rs::model::StaticModel;
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
use fastembed::{
    InitOptionsUserDefined, OnnxSource, Pooling, RerankInitOptionsUserDefined, RerankResult,
    TextEmbedding, TextRerank, TokenizerFiles, UserDefinedEmbeddingModel,
    UserDefinedRerankingModel,
};

use super::search_index::{canonical_tools, searchable_text, tool_fingerprint, ToolSchema};

pub(crate) const MODEL_REPOSITORY: &str = "minishlab/potion-multilingual-128M";
pub(crate) const MODEL_REVISION: &str = "a28f4eebecd4dc585034f605e52d414878a0417c";
pub(crate) const MODEL_DIMENSIONS: usize = 256;
pub(crate) const MODEL_SHA256: &str =
    "14b5eb39cb4ce5666da8ad1f3dc6be4346e9b2d601c073302fa0a31bf7943397";
pub(crate) const TOKENIZER_SHA256: &str =
    "19f1909063da3cfe3bd83a782381f040dccea475f4816de11116444a73e1b6a1";
const MODEL_SIZE: u64 = 512_361_560;
const TOKENIZER_SIZE: u64 = 18_616_131;

pub(crate) const MINILM_REPOSITORY: &str = "sentence-transformers/all-MiniLM-L6-v2";
pub(crate) const MINILM_REVISION: &str = "c9745ed1d9f207416be6d2e6f8de32d1f16199bf";
pub(crate) const MINILM_DIMENSIONS: usize = 384;
pub(crate) const MINILM_MODEL_SHA256: &str =
    "6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452";
pub(crate) const MINILM_TOKENIZER_SHA256: &str =
    "be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037";
const MINILM_MODEL_SIZE: u64 = 90_405_214;
const MINILM_TOKENIZER_SIZE: u64 = 466_247;
const MINILM_INTRA_THREADS: usize = 2;
const MINILM_MAX_LENGTH: usize = 512;

pub(crate) const RERANKER_REPOSITORY: &str = "cross-encoder/ms-marco-MiniLM-L6-v2";
pub(crate) const RERANKER_REVISION: &str = "233902d25c440f23af6f7d6e94d2946bac0bee0a";
pub(crate) const RERANKER_MODEL_SHA256: &str =
    "5d3e70fd0c9ff14b9b5169a51e957b7a9c74897afd0a35ce4bd318150c1d4d4a";
pub(crate) const RERANKER_TOKENIZER_SHA256: &str =
    "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66";
const RERANKER_MAX_LENGTH: usize = 512;
const RERANKER_BATCH_SIZE: usize = 8;

const MINILM_FILES: [(&str, u64, &str); 5] = [
    ("onnx/model.onnx", MINILM_MODEL_SIZE, MINILM_MODEL_SHA256),
    (
        "tokenizer.json",
        MINILM_TOKENIZER_SIZE,
        MINILM_TOKENIZER_SHA256,
    ),
    (
        "config.json",
        612,
        "953f9c0d463486b10a6871cc2fd59f223b2c70184f49815e7efbcab5d8908b41",
    ),
    (
        "special_tokens_map.json",
        112,
        "303df45a03609e4ead04bc3dc1536d0ab19b5358db685b6f3da123d05ec200e3",
    ),
    (
        "tokenizer_config.json",
        350,
        "acb92769e8195aabd29b7b2137a9e6d6e25c476a4f15aa4355c233426c61576b",
    ),
];

const RERANKER_FILES: [(&str, u64, &str); 5] = [
    ("onnx/model.onnx", 91_011_230, RERANKER_MODEL_SHA256),
    ("tokenizer.json", 711_396, RERANKER_TOKENIZER_SHA256),
    (
        "config.json",
        794,
        "380e02c93f431831be65d99a4e7e5f67c133985bf2e77d9d4eba46847190bacc",
    ),
    (
        "special_tokens_map.json",
        132,
        "3c3507f36dff57bce437223db3b3081d1e2b52ec3e56ee55438193ecb2c94dd6",
    ),
    (
        "tokenizer_config.json",
        1_330,
        "a5c2e5a7b1a29a0702cd28c08a399b5ecc110c263009d17f7e3b415f25905fd8",
    ),
];

#[derive(Debug, thiserror::Error)]
#[error("semantic search unavailable: {0}")]
pub(crate) struct SemanticUnavailable(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ModelKind {
    Potion,
    MiniLm,
}

impl ModelKind {
    fn revision(self) -> &'static str {
        match self {
            Self::Potion => MODEL_REVISION,
            Self::MiniLm => MINILM_REVISION,
        }
    }

    fn dimensions(self) -> usize {
        match self {
            Self::Potion => MODEL_DIMENSIONS,
            Self::MiniLm => MINILM_DIMENSIONS,
        }
    }
}

enum LoadedModel {
    Potion(StaticModel),
    #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
    MiniLm(Mutex<TextEmbedding>),
}

impl LoadedModel {
    fn encode(
        &self,
        texts: &[String],
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, SemanticUnavailable> {
        #[cfg(not(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu")))]
        let _ = batch_size;
        match self {
            Self::Potion(model) => {
                catch_encoding(|| model.encode_with_args(texts, Some(512), 1024))
            }
            #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
            Self::MiniLm(model) => {
                if texts.is_empty() {
                    return Ok(Vec::new());
                }
                let mut model = model
                    .lock()
                    .map_err(|_| unavailable("MiniLM model lock poisoned"))?;
                let mut vectors = catch_encoding(|| model.embed(texts, batch_size))?
                    .map_err(|error| unavailable(format!("MiniLM embedding failed: {error}")))?;
                for vector in &mut vectors {
                    normalize_minilm_embedding(vector)?;
                }
                Ok(vectors)
            }
        }
    }
}

#[derive(Clone, Default)]
pub struct SemanticSearch {
    inner: Arc<Inner>,
}

struct Inner {
    model_dir: Option<PathBuf>,
    model_kind: ModelKind,
    model: RwLock<Option<Arc<LoadedModel>>>,
    #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
    reranker: RwLock<Option<Arc<Mutex<TextRerank>>>>,
    active: RwLock<Option<Arc<DenseIndex>>>,
    desired_catalog: Mutex<Option<String>>,
    rebuild: tokio::sync::Mutex<()>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            model_dir: None,
            model_kind: ModelKind::Potion,
            model: RwLock::default(),
            #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
            reranker: RwLock::default(),
            active: RwLock::default(),
            desired_catalog: Mutex::default(),
            rebuild: tokio::sync::Mutex::default(),
        }
    }
}

struct DenseIndex {
    catalog_fingerprint: String,
    model_revision: &'static str,
    function_ids: Vec<String>,
    searchable_texts: Vec<String>,
    vectors: Vec<Vec<f32>>,
}

impl SemanticSearch {
    pub fn new(model_dir: Option<PathBuf>) -> Self {
        let model_kind = model_dir
            .as_deref()
            .filter(|path| minilm_artifact_contract_matches(path))
            .map_or(ModelKind::Potion, |_| ModelKind::MiniLm);
        Self {
            inner: Arc::new(Inner {
                model_dir,
                model_kind,
                ..Inner::default()
            }),
        }
    }

    #[cfg(test)]
    pub(crate) fn production_minilm_unavailable_for_test() -> Self {
        Self {
            inner: Arc::new(Inner {
                model_kind: ModelKind::MiniLm,
                ..Inner::default()
            }),
        }
    }

    pub(crate) fn is_production_minilm(&self) -> bool {
        self.inner.model_kind == ModelKind::MiniLm
    }

    pub(crate) fn model_revision(&self) -> &'static str {
        self.inner.model_kind.revision()
    }

    pub(crate) fn model_repository(&self) -> &'static str {
        match self.inner.model_kind {
            ModelKind::Potion => MODEL_REPOSITORY,
            ModelKind::MiniLm => MINILM_REPOSITORY,
        }
    }

    pub(crate) fn reranker_repository(&self) -> &'static str {
        RERANKER_REPOSITORY
    }

    pub(crate) fn reranker_revision(&self) -> &'static str {
        RERANKER_REVISION
    }

    pub(crate) fn rebuild(&self, tools: Arc<Vec<ToolSchema>>) {
        let fingerprint = tool_fingerprint(&tools);
        *self.inner.desired_catalog.lock().expect("desired catalog") = Some(fingerprint.clone());
        let inner = self.inner.clone();
        tokio::spawn(async move {
            let _rebuild = inner.rebuild.lock().await;
            if !is_desired(&inner, &fingerprint) {
                return;
            }
            let Some(model_dir) = inner.model_dir.clone() else {
                return;
            };
            let loaded_model = { inner.model.read().expect("semantic model").clone() };
            let model = match loaded_model {
                Some(model) => model,
                None => {
                    let model_kind = inner.model_kind;
                    let loaded = tokio::task::spawn_blocking(move || {
                        load_verified_model(&model_dir, model_kind)
                    })
                    .await;
                    let model = match loaded {
                        Ok(Ok(model)) => Arc::new(model),
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "semantic model unavailable");
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "semantic model load task failed");
                            return;
                        }
                    };
                    *inner.model.write().expect("semantic model") = Some(model.clone());
                    model
                }
            };
            if !is_desired(&inner, &fingerprint) {
                return;
            }
            let corpus = canonical_tools(&tools);
            let function_ids: Vec<String> = corpus.iter().map(|tool| tool.name.clone()).collect();
            let documents: Vec<String> = corpus.iter().map(searchable_text).collect();
            let searchable_texts = documents.clone();
            let encode_model = model.clone();
            let encoded =
                tokio::task::spawn_blocking(move || encode_model.encode(&documents, Some(32)))
                    .await;
            let vectors = match encoded {
                Ok(Ok(vectors))
                    if validate_vectors(
                        &vectors,
                        function_ids.len(),
                        inner.model_kind.dimensions(),
                    )
                    .is_ok() =>
                {
                    vectors
                }
                Ok(Ok(_)) => {
                    tracing::warn!("semantic catalog vectors failed validation");
                    return;
                }
                Ok(Err(error)) => {
                    tracing::warn!(%error, "semantic catalog encoding panicked");
                    return;
                }
                Err(error) => {
                    tracing::warn!(%error, "semantic catalog encoding task failed");
                    return;
                }
            };
            let index = DenseIndex {
                catalog_fingerprint: fingerprint.clone(),
                model_revision: inner.model_kind.revision(),
                function_ids,
                searchable_texts,
                vectors,
            };
            #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
            if inner.model_kind == ModelKind::MiniLm {
                let reranker = match inner.reranker.read() {
                    Ok(current) => current.clone(),
                    Err(_) => None,
                };
                if reranker.is_none() {
                    let reranker_root = match inner.model_dir.clone() {
                        Some(path) => path.join("reranker"),
                        None => return,
                    };
                    match tokio::task::spawn_blocking(move || {
                        load_verified_reranker(&reranker_root)
                    })
                    .await
                    {
                        Ok(Ok(loaded)) => {
                            if let Ok(mut slot) = inner.reranker.write() {
                                *slot = Some(Arc::new(Mutex::new(loaded)));
                            } else {
                                tracing::warn!("semantic reranker lock poisoned");
                                return;
                            }
                        }
                        Ok(Err(error)) => {
                            tracing::warn!(%error, "semantic reranker unavailable");
                            return;
                        }
                        Err(error) => {
                            tracing::warn!(%error, "semantic reranker load task failed");
                            return;
                        }
                    }
                }
            }
            publish_if_desired(&inner, &fingerprint, index);
        });
    }

    pub(crate) async fn rank(
        &self,
        catalog_fingerprint: &str,
        queries: &[String],
        minimum_cosine: f32,
    ) -> Result<Vec<Vec<(String, f64)>>, SemanticUnavailable> {
        let index = self
            .inner
            .active
            .read()
            .expect("semantic index")
            .clone()
            .ok_or_else(|| unavailable("index absent"))?;
        if index.catalog_fingerprint != catalog_fingerprint
            || index.model_revision != self.inner.model_kind.revision()
        {
            return Err(unavailable("index stale"));
        }
        let model = self
            .inner
            .model
            .read()
            .expect("semantic model")
            .clone()
            .ok_or_else(|| unavailable("model absent"))?;
        let queries = queries.to_vec();
        let query_count = queries.len();
        let encoded = tokio::task::spawn_blocking(move || model.encode(&queries, None))
            .await
            .map_err(|error| unavailable(format!("query task failed: {error}")))??;
        validate_vectors(&encoded, query_count, self.inner.model_kind.dimensions())?;
        encoded
            .iter()
            .map(|query| {
                rank_vectors(
                    &index.catalog_fingerprint,
                    catalog_fingerprint,
                    &index.function_ids,
                    &index.vectors,
                    query,
                    minimum_cosine,
                )
            })
            .collect()
    }

    #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
    pub(crate) async fn rerank(
        &self,
        catalog_fingerprint: &str,
        queries: &[String],
        candidate_ids: &[Vec<String>],
    ) -> Result<Vec<Vec<(String, f64)>>, SemanticUnavailable> {
        if self.inner.model_kind != ModelKind::MiniLm || queries.len() != candidate_ids.len() {
            return Err(unavailable("production MiniLM reranker unavailable"));
        }
        let index = self
            .inner
            .active
            .read()
            .map_err(|_| unavailable("semantic index lock poisoned"))?
            .clone()
            .ok_or_else(|| unavailable("index absent"))?;
        if index.catalog_fingerprint != catalog_fingerprint
            || index.model_revision != MINILM_REVISION
        {
            return Err(unavailable("index stale"));
        }
        let positions: HashMap<&str, usize> = index
            .function_ids
            .iter()
            .enumerate()
            .map(|(position, id)| (id.as_str(), position))
            .collect();
        let mut lanes = Vec::with_capacity(candidate_ids.len());
        for (query, candidates) in queries.iter().zip(candidate_ids) {
            let mut seen = std::collections::HashSet::new();
            let documents = candidates
                .iter()
                .map(|id| {
                    if !seen.insert(id.as_str()) {
                        return Err(unavailable("duplicate rerank candidate"));
                    }
                    positions
                        .get(id.as_str())
                        .map(|position| index.searchable_texts[*position].clone())
                        .ok_or_else(|| unavailable("unknown rerank candidate"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            lanes.push((query.clone(), candidates.clone(), documents));
        }
        let reranker = self
            .inner
            .reranker
            .read()
            .map_err(|_| unavailable("semantic reranker lock poisoned"))?
            .clone()
            .ok_or_else(|| unavailable("reranker absent"))?;
        tokio::task::spawn_blocking(move || {
            let mut reranker = reranker
                .lock()
                .map_err(|_| unavailable("reranker model lock poisoned"))?;
            lanes
                .into_iter()
                .map(|(query, ids, documents)| {
                    if documents.is_empty() {
                        return Ok(Vec::new());
                    }
                    let results = catch_encoding(|| {
                        reranker.rerank(query, documents, false, Some(RERANKER_BATCH_SIZE))
                    })?
                    .map_err(|error| unavailable(format!("reranking failed: {error}")))?;
                    map_rerank_results(&ids, &results)
                })
                .collect()
        })
        .await
        .map_err(|error| unavailable(format!("rerank task failed: {error}")))?
    }

    #[cfg(not(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu")))]
    pub(crate) async fn rerank(
        &self,
        _catalog_fingerprint: &str,
        _queries: &[String],
        _candidate_ids: &[Vec<String>],
    ) -> Result<Vec<Vec<(String, f64)>>, SemanticUnavailable> {
        Err(unavailable("production MiniLM support is not compiled"))
    }

    #[cfg(test)]
    fn set_desired_for_test(&self, fingerprint: &str) {
        *self.inner.desired_catalog.lock().expect("desired catalog") = Some(fingerprint.into());
    }

    #[cfg(test)]
    fn publish_for_test(
        &self,
        fingerprint: &str,
        function_ids: Vec<String>,
        vectors: Vec<Vec<f32>>,
    ) -> bool {
        publish_if_desired(
            &self.inner,
            fingerprint,
            DenseIndex {
                catalog_fingerprint: fingerprint.into(),
                model_revision: self.inner.model_kind.revision(),
                searchable_texts: function_ids.clone(),
                function_ids,
                vectors,
            },
        )
    }

    #[cfg(test)]
    fn active_fingerprint_for_test(&self) -> Option<String> {
        self.inner
            .active
            .read()
            .expect("semantic index")
            .as_ref()
            .map(|index| index.catalog_fingerprint.clone())
    }
}

fn unavailable(message: impl Into<String>) -> SemanticUnavailable {
    SemanticUnavailable(message.into())
}

fn is_desired(inner: &Inner, fingerprint: &str) -> bool {
    inner
        .desired_catalog
        .lock()
        .expect("desired catalog")
        .as_deref()
        == Some(fingerprint)
}

fn publish_if_desired(inner: &Inner, fingerprint: &str, index: DenseIndex) -> bool {
    let desired = inner.desired_catalog.lock().expect("desired catalog");
    if desired.as_deref() != Some(fingerprint)
        || index.searchable_texts.len() != index.function_ids.len()
        || validate_vectors(
            &index.vectors,
            index.function_ids.len(),
            inner.model_kind.dimensions(),
        )
        .is_err()
    {
        return false;
    }
    *inner.active.write().expect("semantic index") = Some(Arc::new(index));
    true
}

fn catch_encoding<T>(encode: impl FnOnce() -> T) -> Result<T, SemanticUnavailable> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(encode))
        .map_err(|_| unavailable("encoding panicked"))
}

fn validate_vectors(
    vectors: &[Vec<f32>],
    expected_count: usize,
    expected_dimensions: usize,
) -> Result<(), SemanticUnavailable> {
    if vectors.len() != expected_count
        || vectors.iter().any(|vector| {
            vector.len() != expected_dimensions || vector.iter().any(|value| !value.is_finite())
        })
    {
        return Err(unavailable("invalid vector dimensions or values"));
    }
    Ok(())
}

fn cosine(left: &[f32], right: &[f32]) -> Result<f64, SemanticUnavailable> {
    if left.len() != right.len()
        || left.is_empty()
        || left.iter().chain(right).any(|value| !value.is_finite())
    {
        return Err(unavailable("invalid cosine vectors"));
    }
    let dot: f64 = left
        .iter()
        .zip(right)
        .map(|(a, b)| f64::from(*a) * f64::from(*b))
        .sum();
    let left_norm: f64 = left
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    let right_norm: f64 = right
        .iter()
        .map(|value| f64::from(*value).powi(2))
        .sum::<f64>()
        .sqrt();
    if left_norm == 0.0 || right_norm == 0.0 {
        return Err(unavailable("zero-length cosine vector"));
    }
    Ok(dot / (left_norm * right_norm))
}

fn rank_vectors(
    index_fingerprint: &str,
    requested_fingerprint: &str,
    function_ids: &[String],
    vectors: &[Vec<f32>],
    query: &[f32],
    minimum_cosine: f32,
) -> Result<Vec<(String, f64)>, SemanticUnavailable> {
    if index_fingerprint != requested_fingerprint || function_ids.len() != vectors.len() {
        return Err(unavailable("index stale or incomplete"));
    }
    let mut ranked = function_ids
        .iter()
        .zip(vectors)
        .map(|(id, vector)| Ok((id.clone(), cosine(query, vector)?)))
        .collect::<Result<Vec<_>, SemanticUnavailable>>()?;
    ranked.retain(|(_, score)| *score >= f64::from(minimum_cosine));
    ranked.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| left.0.cmp(&right.0))
    });
    Ok(ranked)
}

pub(crate) fn weighted_rrf(
    lexical: &[(String, f64)],
    semantic: &[(String, f64)],
    semantic_weight: f64,
) -> Vec<(String, f64)> {
    #[derive(Default)]
    struct Rank {
        lexical: Option<usize>,
        semantic: Option<usize>,
    }
    let mut ranks: HashMap<String, Rank> = HashMap::new();
    let mut lexical_rank = 0;
    for (id, _) in lexical {
        let entry = ranks.entry(id.clone()).or_default();
        if entry.lexical.is_none() {
            lexical_rank += 1;
            entry.lexical = Some(lexical_rank);
        }
    }
    let mut semantic_rank = 0;
    for (id, _) in semantic {
        let entry = ranks.entry(id.clone()).or_default();
        if entry.semantic.is_none() {
            semantic_rank += 1;
            entry.semantic = Some(semantic_rank);
        }
    }
    let mut fused: Vec<(String, f64, Option<usize>, Option<usize>)> = ranks
        .into_iter()
        .map(|(id, rank)| {
            let score = rank
                .lexical
                .map_or(0.0, |value| 1.0 / (60.0 + value as f64))
                + rank
                    .semantic
                    .map_or(0.0, |value| semantic_weight / (60.0 + value as f64));
            (id, score, rank.lexical, rank.semantic)
        })
        .collect();
    fused.sort_by(|left, right| {
        right
            .1
            .total_cmp(&left.1)
            .then_with(|| {
                left.2
                    .unwrap_or(usize::MAX)
                    .cmp(&right.2.unwrap_or(usize::MAX))
            })
            .then_with(|| {
                left.3
                    .unwrap_or(usize::MAX)
                    .cmp(&right.3.unwrap_or(usize::MAX))
            })
            .then_with(|| left.0.cmp(&right.0))
    });
    fused
        .into_iter()
        .map(|(id, score, _, _)| (id, score))
        .collect()
}

#[derive(Clone)]
struct ObservedModelContract {
    required_files: [bool; 4],
    repo: String,
    revision: String,
    manifest_model_sha256: String,
    manifest_tokenizer_sha256: String,
    manifest_dimensions: usize,
    model_type: String,
    hidden_dim: usize,
    normalize: bool,
    model_size: u64,
    tokenizer_size: u64,
    model_sha256: String,
    tokenizer_sha256: String,
}

impl ObservedModelContract {
    #[cfg(test)]
    fn expected() -> Self {
        Self {
            required_files: [true; 4],
            repo: MODEL_REPOSITORY.into(),
            revision: MODEL_REVISION.into(),
            manifest_model_sha256: MODEL_SHA256.into(),
            manifest_tokenizer_sha256: TOKENIZER_SHA256.into(),
            manifest_dimensions: MODEL_DIMENSIONS,
            model_type: "model2vec".into(),
            hidden_dim: MODEL_DIMENSIONS,
            normalize: true,
            model_size: MODEL_SIZE,
            tokenizer_size: TOKENIZER_SIZE,
            model_sha256: MODEL_SHA256.into(),
            tokenizer_sha256: TOKENIZER_SHA256.into(),
        }
    }
}

fn validate_contract(observed: &ObservedModelContract) -> Result<(), SemanticUnavailable> {
    validate_contract_metadata(observed)?;
    if observed.model_sha256 != MODEL_SHA256 || observed.tokenizer_sha256 != TOKENIZER_SHA256 {
        return Err(unavailable("model checksum mismatch"));
    }
    Ok(())
}

fn validate_contract_metadata(observed: &ObservedModelContract) -> Result<(), SemanticUnavailable> {
    if observed.required_files != [true; 4]
        || observed.repo != MODEL_REPOSITORY
        || observed.revision != MODEL_REVISION
        || observed.manifest_model_sha256 != MODEL_SHA256
        || observed.manifest_tokenizer_sha256 != TOKENIZER_SHA256
        || observed.manifest_dimensions != MODEL_DIMENSIONS
        || observed.model_type != "model2vec"
        || observed.hidden_dim != MODEL_DIMENSIONS
        || !observed.normalize
        || observed.model_size != MODEL_SIZE
        || observed.tokenizer_size != TOKENIZER_SIZE
    {
        return Err(unavailable("model contract mismatch"));
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    repo: String,
    revision: String,
    model_sha256: String,
    tokenizer_sha256: String,
    dimensions: usize,
}

#[derive(Deserialize)]
struct ModelConfig {
    model_type: String,
    hidden_dim: usize,
    normalize: bool,
}

fn sha256(path: &Path) -> Result<String, SemanticUnavailable> {
    let mut file = File::open(path).map_err(|error| unavailable(error.to_string()))?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| unavailable(error.to_string()))?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn observe_model_contract(path: &Path) -> Result<ObservedModelContract, SemanticUnavailable> {
    let names = [
        "config.json",
        "tokenizer.json",
        "model.safetensors",
        "iii-model.json",
    ];
    let required_files = names.map(|name| required_file_is_regular(&path.join(name)));
    if required_files != [true; 4] {
        return Err(unavailable("required model file absent"));
    }
    let manifest: Manifest = serde_json::from_reader(
        File::open(path.join("iii-model.json")).map_err(|error| unavailable(error.to_string()))?,
    )
    .map_err(|error| unavailable(error.to_string()))?;
    let config: ModelConfig = serde_json::from_reader(
        File::open(path.join("config.json")).map_err(|error| unavailable(error.to_string()))?,
    )
    .map_err(|error| unavailable(error.to_string()))?;
    let model = path.join("model.safetensors");
    let tokenizer = path.join("tokenizer.json");
    let mut observed = ObservedModelContract {
        required_files,
        repo: manifest.repo,
        revision: manifest.revision,
        manifest_model_sha256: manifest.model_sha256,
        manifest_tokenizer_sha256: manifest.tokenizer_sha256,
        manifest_dimensions: manifest.dimensions,
        model_type: config.model_type,
        hidden_dim: config.hidden_dim,
        normalize: config.normalize,
        model_size: model
            .metadata()
            .map_err(|error| unavailable(error.to_string()))?
            .len(),
        tokenizer_size: tokenizer
            .metadata()
            .map_err(|error| unavailable(error.to_string()))?
            .len(),
        model_sha256: String::new(),
        tokenizer_sha256: String::new(),
    };
    validate_contract_metadata(&observed)?;
    observed.model_sha256 = sha256(&model)?;
    observed.tokenizer_sha256 = sha256(&tokenizer)?;
    Ok(observed)
}

fn required_file_is_regular(path: &Path) -> bool {
    path.symlink_metadata()
        .is_ok_and(|metadata| metadata.file_type().is_file())
}

fn minilm_artifact_contract_matches(path: &Path) -> bool {
    MINILM_FILES.iter().all(|(name, expected_size, _)| {
        path.join(name).symlink_metadata().is_ok_and(|metadata| {
            metadata.file_type().is_file() && metadata.len() == *expected_size
        })
    })
}

fn verify_artifacts(root: &Path, files: &[(&str, u64, &str)]) -> Result<(), SemanticUnavailable> {
    for (name, expected_size, expected_sha256) in files {
        let artifact = root.join(name);
        let metadata = artifact
            .symlink_metadata()
            .map_err(|error| unavailable(format!("inspect {}: {error}", artifact.display())))?;
        if !metadata.file_type().is_file()
            || metadata.len() != *expected_size
            || sha256(&artifact)? != *expected_sha256
        {
            return Err(unavailable(format!(
                "artifact failed verification: {}",
                artifact.display()
            )));
        }
    }
    Ok(())
}

fn load_verified_model(
    path: &Path,
    model_kind: ModelKind,
) -> Result<LoadedModel, SemanticUnavailable> {
    match model_kind {
        ModelKind::Potion => {
            validate_contract(&observe_model_contract(path)?)?;
            let model = StaticModel::from_pretrained(path, None, None, None)
                .map_err(|error| unavailable(error.to_string()))?;
            let probe = catch_encoding(|| {
                model.encode_with_args(&["model verification".into()], Some(512), 1024)
            })?;
            validate_vectors(&probe, 1, MODEL_DIMENSIONS)?;
            Ok(LoadedModel::Potion(model))
        }
        ModelKind::MiniLm => load_verified_minilm(path),
    }
}

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
fn tokenizer_files(root: &Path) -> Result<TokenizerFiles, SemanticUnavailable> {
    let read = |name: &str| {
        let path = root.join(name);
        std::fs::read(&path)
            .map_err(|error| unavailable(format!("read {}: {error}", path.display())))
    };
    Ok(TokenizerFiles {
        tokenizer_file: read("tokenizer.json")?,
        config_file: read("config.json")?,
        special_tokens_map_file: read("special_tokens_map.json")?,
        tokenizer_config_file: read("tokenizer_config.json")?,
    })
}

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
fn load_verified_minilm(path: &Path) -> Result<LoadedModel, SemanticUnavailable> {
    verify_artifacts(path, &MINILM_FILES)?;
    let onnx = std::fs::read(path.join("onnx/model.onnx"))
        .map_err(|error| unavailable(format!("read MiniLM model: {error}")))?;
    let user_model =
        UserDefinedEmbeddingModel::new(onnx, tokenizer_files(path)?).with_pooling(Pooling::Mean);
    let model = TextEmbedding::try_new_from_user_defined(
        user_model,
        InitOptionsUserDefined::new()
            .with_max_length(MINILM_MAX_LENGTH)
            .with_intra_threads(MINILM_INTRA_THREADS),
    )
    .map_err(|error| unavailable(format!("initialize MiniLM: {error}")))?;
    let loaded = LoadedModel::MiniLm(Mutex::new(model));
    let probe = loaded.encode(&["model verification".into()], None)?;
    validate_vectors(&probe, 1, MINILM_DIMENSIONS)?;
    Ok(loaded)
}

#[cfg(not(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu")))]
fn load_verified_minilm(_path: &Path) -> Result<LoadedModel, SemanticUnavailable> {
    Err(unavailable("production MiniLM support is not compiled"))
}

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
fn load_verified_reranker(path: &Path) -> Result<TextRerank, SemanticUnavailable> {
    verify_artifacts(path, &RERANKER_FILES)?;
    let onnx = std::fs::read(path.join("onnx/model.onnx"))
        .map_err(|error| unavailable(format!("read reranker model: {error}")))?;
    let user_model =
        UserDefinedRerankingModel::new(OnnxSource::Memory(onnx), tokenizer_files(path)?);
    TextRerank::try_new_from_user_defined(
        user_model,
        RerankInitOptionsUserDefined::new()
            .with_max_length(RERANKER_MAX_LENGTH)
            .with_intra_threads(MINILM_INTRA_THREADS),
    )
    .map_err(|error| unavailable(format!("initialize reranker: {error}")))
}

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
fn map_rerank_results(
    candidate_ids: &[String],
    results: &[RerankResult],
) -> Result<Vec<(String, f64)>, SemanticUnavailable> {
    if results.len() != candidate_ids.len() {
        return Err(unavailable("reranker returned the wrong result count"));
    }
    let mut seen = vec![false; candidate_ids.len()];
    results
        .iter()
        .map(|result| {
            if !result.score.is_finite()
                || result.index >= candidate_ids.len()
                || seen[result.index]
            {
                return Err(unavailable("reranker returned invalid output"));
            }
            seen[result.index] = true;
            Ok((candidate_ids[result.index].clone(), f64::from(result.score)))
        })
        .collect()
}

#[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
fn normalize_minilm_embedding(embedding: &mut [f32]) -> Result<(), SemanticUnavailable> {
    if embedding.len() != MINILM_DIMENSIONS || embedding.iter().any(|value| !value.is_finite()) {
        return Err(unavailable("invalid MiniLM embedding dimensions or values"));
    }
    let norm_squared: f32 = embedding.iter().map(|value| value * value).sum();
    if !norm_squared.is_finite() || norm_squared <= f32::EPSILON {
        return Err(unavailable("invalid zero-length MiniLM embedding"));
    }
    let inverse_norm = norm_squared.sqrt().recip();
    for value in embedding {
        *value *= inverse_norm;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_contract() -> ObservedModelContract {
        ObservedModelContract::expected()
    }

    #[test]
    fn observed_contract_rejects_missing_and_mismatched_model_files() {
        let mut observed = valid_contract();
        observed.required_files[0] = false;
        assert!(validate_contract(&observed).is_err());

        let mut observed = valid_contract();
        observed.revision = "wrong".into();
        assert!(validate_contract(&observed).is_err());

        let mut observed = valid_contract();
        observed.model_size -= 1;
        assert!(validate_contract(&observed).is_err());

        let mut observed = valid_contract();
        observed.tokenizer_sha256 = "bad".into();
        assert!(validate_contract(&observed).is_err());

        let mut observed = valid_contract();
        observed.model_type = "bert".into();
        assert!(validate_contract(&observed).is_err());

        let mut observed = valid_contract();
        observed.normalize = false;
        assert!(validate_contract(&observed).is_err());
    }

    #[test]
    fn vectors_require_exact_finite_dimensions() {
        assert!(validate_vectors(&[vec![0.0; MODEL_DIMENSIONS]], 1, MODEL_DIMENSIONS).is_ok());
        assert!(validate_vectors(&[vec![0.0; MODEL_DIMENSIONS - 1]], 1, MODEL_DIMENSIONS).is_err());
        let mut non_finite = vec![0.0; MODEL_DIMENSIONS];
        non_finite[0] = f32::NAN;
        assert!(validate_vectors(&[non_finite], 1, MODEL_DIMENSIONS).is_err());
    }

    #[test]
    fn cosine_rank_is_ordered_thresholded_stable_and_rejects_stale_data() {
        let ids = vec!["b::same".into(), "a::best".into(), "c::low".into()];
        let vectors = vec![vec![1.0, 0.0], vec![1.0, 0.0], vec![0.0, 1.0]];
        assert!(rank_vectors("old", "new", &ids, &vectors, &[1.0, 0.0], 0.5).is_err());
        assert_eq!(
            rank_vectors("new", "new", &ids, &vectors, &[1.0, 0.0], 0.5).unwrap(),
            vec![("a::best".into(), 1.0), ("b::same".into(), 1.0)]
        );
        assert!(rank_vectors("new", "new", &ids, &vectors, &[0.0, 0.0], 0.0).is_err());
    }

    #[test]
    fn weighted_rrf_uses_first_rank_and_deterministic_ties() {
        let fused = weighted_rrf(
            &[("a".into(), 9.0), ("a".into(), 8.0), ("b".into(), 7.0)],
            &[("b".into(), 1.0), ("c".into(), 0.5)],
            0.5,
        );
        let scores: std::collections::HashMap<_, _> = fused.iter().cloned().collect();
        assert!((scores["a"] - 1.0 / 61.0).abs() < 1e-12);
        assert!((scores["b"] - (1.0 / 62.0 + 0.5 / 61.0)).abs() < 1e-12);
        assert_eq!(fused.iter().filter(|(id, _)| id == "a").count(), 1);

        let tied = weighted_rrf(&[("b".into(), 1.0)], &[("a".into(), 1.0)], 1.0);
        assert_eq!(tied[0].0, "b");
    }

    #[test]
    fn encoding_panics_are_contained() {
        assert!(catch_encoding(|| -> Vec<Vec<f32>> { panic!("tokenizer") }).is_err());
    }

    #[test]
    fn only_complete_latest_catalog_can_publish() {
        let semantic = SemanticSearch::default();
        semantic.set_desired_for_test("new");
        assert!(!semantic.publish_for_test(
            "old",
            vec!["old::fn".into()],
            vec![vec![0.0; MODEL_DIMENSIONS]]
        ));
        assert!(semantic.publish_for_test(
            "new",
            vec!["new::fn".into()],
            vec![vec![0.0; MODEL_DIMENSIONS]]
        ));
        assert_eq!(
            semantic.active_fingerprint_for_test().as_deref(),
            Some("new")
        );
        assert!(!semantic.publish_for_test("new", vec!["broken::fn".into()], Vec::new()));
    }

    #[test]
    fn absent_model_files_fail_before_loading() {
        let directory = tempfile::tempdir().unwrap();
        assert!(observe_model_contract(directory.path()).is_err());
    }

    #[test]
    fn complete_minilm_layout_selects_the_production_backend() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("onnx")).unwrap();
        for (name, size, _) in MINILM_FILES {
            let path = directory.path().join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            File::create(path).unwrap().set_len(size).unwrap();
        }
        let semantic = SemanticSearch::new(Some(directory.path().into()));

        assert!(semantic.is_production_minilm());
        assert_eq!(semantic.model_repository(), MINILM_REPOSITORY);
        assert_eq!(semantic.model_revision(), MINILM_REVISION);
    }

    #[cfg(all(target_arch = "x86_64", target_os = "linux", target_env = "gnu"))]
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "requires III_DIRECTORY_MINILM_MODEL_PATH and the pinned ONNX runtime"]
    async fn real_production_minilm_retrieval_and_reranker_round_trip() {
        let Ok(path) = std::env::var("III_DIRECTORY_MINILM_MODEL_PATH") else {
            return;
        };
        let tools = Arc::new(vec![
            ToolSchema {
                name: "web::fetch".into(),
                description: "Fetch a web page by URL and return its contents.".into(),
                parameters: serde_json::json!({"type": "object", "properties": {"url": {"type": "string"}}}),
            },
            ToolSchema {
                name: "email::send".into(),
                description: "Send an email message to a recipient.".into(),
                parameters: serde_json::json!({"type": "object", "properties": {"to": {"type": "string"}}}),
            },
            ToolSchema {
                name: "calendar::list".into(),
                description: "List calendar events in a time range.".into(),
                parameters: serde_json::json!({"type": "object"}),
            },
        ]);
        let fingerprint = tool_fingerprint(&tools);
        let semantic = SemanticSearch::new(Some(path.into()));
        assert!(semantic.is_production_minilm());
        semantic.rebuild(tools);

        let queries = vec!["download the contents of a web page".to_owned()];
        let ranked = tokio::time::timeout(std::time::Duration::from_secs(120), async {
            loop {
                if let Ok(ranked) = semantic.rank(&fingerprint, &queries, -1.0).await {
                    break ranked;
                }
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
        })
        .await
        .expect("MiniLM catalog preparation timed out");
        assert_eq!(ranked.len(), 1);
        assert_eq!(ranked[0].len(), 3);
        let candidates = vec![ranked[0].iter().map(|(id, _)| id.clone()).collect()];
        let reranked = semantic
            .rerank(&fingerprint, &queries, &candidates)
            .await
            .unwrap();
        assert_eq!(reranked.len(), 1);
        assert_eq!(reranked[0].len(), 3);
        assert_eq!(reranked[0][0].0, "web::fetch");
    }

    #[cfg(unix)]
    #[test]
    fn model_contract_rejects_symlinked_files() {
        use std::os::unix::fs::symlink;
        let directory = tempfile::tempdir().unwrap();
        let target = directory.path().join("target");
        std::fs::write(&target, b"{}").unwrap();
        for name in [
            "config.json",
            "tokenizer.json",
            "model.safetensors",
            "iii-model.json",
        ] {
            symlink(&target, directory.path().join(name)).unwrap();
        }
        assert!(!required_file_is_regular(
            &directory.path().join("config.json")
        ));
    }

    #[test]
    #[ignore = "requires III_DIRECTORY_POTION_MODEL_PATH"]
    fn real_potion_model() {
        let Ok(path) = std::env::var("III_DIRECTORY_POTION_MODEL_PATH") else {
            return;
        };
        let model = load_verified_model(Path::new(&path), ModelKind::Potion).unwrap();
        let vector = model.encode(&["verify local model".into()], None).unwrap();
        validate_vectors(&vector, 1, MODEL_DIMENSIONS).unwrap();
    }
}
