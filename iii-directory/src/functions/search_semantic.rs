use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};

use sha2::{Digest, Sha256};

#[cfg(minilm)]
use fastembed::{
    InitOptionsUserDefined, OnnxSource, Pooling, RerankInitOptionsUserDefined, RerankResult,
    TextEmbedding, TextRerank, TokenizerFiles, UserDefinedEmbeddingModel,
    UserDefinedRerankingModel,
};

use super::search_index::{canonical_tools, searchable_text, tool_fingerprint, ToolSchema};

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

#[cfg(minilm)]
struct LoadedModel(Mutex<TextEmbedding>);
#[cfg(not(minilm))]
struct LoadedModel;

impl LoadedModel {
    #[cfg(minilm)]
    fn encode(
        &self,
        texts: &[String],
        batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, SemanticUnavailable> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let mut model = self
            .0
            .lock()
            .map_err(|_| unavailable("MiniLM model lock poisoned"))?;
        let mut vectors = catch_encoding(|| model.embed(texts, batch_size))?
            .map_err(|error| unavailable(format!("MiniLM embedding failed: {error}")))?;
        for vector in &mut vectors {
            normalize_minilm_embedding(vector)?;
        }
        Ok(vectors)
    }

    #[cfg(not(minilm))]
    fn encode(
        &self,
        _texts: &[String],
        _batch_size: Option<usize>,
    ) -> Result<Vec<Vec<f32>>, SemanticUnavailable> {
        Err(unavailable("MiniLM is not compiled for this target"))
    }
}

#[derive(Clone, Default)]
pub struct SemanticSearch {
    inner: Arc<Inner>,
}

#[derive(Default)]
struct Inner {
    model_dir: Option<PathBuf>,
    model: RwLock<Option<Arc<LoadedModel>>>,
    #[cfg(minilm)]
    reranker: RwLock<Option<Arc<Mutex<TextRerank>>>>,
    active: RwLock<Option<Arc<DenseIndex>>>,
    desired_catalog: Mutex<Option<String>>,
    rebuild: tokio::sync::Mutex<()>,
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
        Self {
            inner: Arc::new(Inner {
                model_dir,
                ..Inner::default()
            }),
        }
    }

    /// A model directory is configured (`null` disables the lane). Whether
    /// the bundle loads is decided lazily; every failure falls back to BM25.
    pub(crate) fn is_production_minilm(&self) -> bool {
        self.inner.model_dir.is_some()
    }

    pub(crate) fn model_revision(&self) -> &'static str {
        MINILM_REVISION
    }

    pub(crate) fn model_repository(&self) -> &'static str {
        MINILM_REPOSITORY
    }

    pub(crate) fn reranker_repository(&self) -> &'static str {
        RERANKER_REPOSITORY
    }

    pub(crate) fn reranker_revision(&self) -> &'static str {
        RERANKER_REVISION
    }

    pub fn rebuild(&self, tools: Arc<Vec<ToolSchema>>) {
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
                    let loaded =
                        tokio::task::spawn_blocking(move || load_verified_minilm(&model_dir)).await;
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
                    if validate_vectors(&vectors, function_ids.len(), MINILM_DIMENSIONS)
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
                model_revision: MINILM_REVISION,
                function_ids,
                searchable_texts,
                vectors,
            };
            #[cfg(minilm)]
            {
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
            || index.model_revision != MINILM_REVISION
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
        validate_vectors(&encoded, query_count, MINILM_DIMENSIONS)?;
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

    /// Rank ad-hoc documents (registry contracts, registry worker
    /// descriptions) against `queries` with the loaded model: encode both on
    /// the fly, cosine, `minimum_cosine` floor. No catalog index is involved,
    /// so this serves anything not (yet) installed.
    pub(crate) async fn rank_documents(
        &self,
        queries: &[String],
        documents: &[ToolSchema],
        minimum_cosine: f32,
    ) -> Result<Vec<Vec<(String, f64)>>, SemanticUnavailable> {
        let model = self
            .inner
            .model
            .read()
            .expect("semantic model")
            .clone()
            .ok_or_else(|| unavailable("model absent"))?;
        if documents.is_empty() {
            return Ok(vec![Vec::new(); queries.len()]);
        }
        let corpus = canonical_tools(documents);
        let ids: Vec<String> = corpus.iter().map(|tool| tool.name.clone()).collect();
        let texts: Vec<String> = corpus.iter().map(searchable_text).collect();
        let queries = queries.to_vec();
        let (query_count, document_count) = (queries.len(), ids.len());
        let dimensions = MINILM_DIMENSIONS;
        let (query_vectors, document_vectors) = tokio::task::spawn_blocking(move || {
            let documents = model.encode(&texts, Some(32))?;
            let queries = model.encode(&queries, None)?;
            Ok::<_, SemanticUnavailable>((queries, documents))
        })
        .await
        .map_err(|error| unavailable(format!("document rank task failed: {error}")))??;
        validate_vectors(&query_vectors, query_count, dimensions)?;
        validate_vectors(&document_vectors, document_count, dimensions)?;
        query_vectors
            .iter()
            .map(|query| {
                rank_vectors(
                    "ad-hoc",
                    "ad-hoc",
                    &ids,
                    &document_vectors,
                    query,
                    minimum_cosine,
                )
            })
            .collect()
    }

    #[cfg(minilm)]
    pub(crate) async fn rerank(
        &self,
        catalog_fingerprint: &str,
        queries: &[String],
        candidate_ids: &[Vec<String>],
    ) -> Result<Vec<Vec<(String, f64)>>, SemanticUnavailable> {
        if queries.len() != candidate_ids.len() {
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

    #[cfg(not(minilm))]
    pub(crate) async fn rerank(
        &self,
        _catalog_fingerprint: &str,
        _queries: &[String],
        _candidate_ids: &[Vec<String>],
    ) -> Result<Vec<Vec<(String, f64)>>, SemanticUnavailable> {
        Err(unavailable("MiniLM is not compiled for this target"))
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
                model_revision: MINILM_REVISION,
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
        || validate_vectors(&index.vectors, index.function_ids.len(), MINILM_DIMENSIONS).is_err()
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

/// Whether the complete pinned MiniLM bundle (embedding files at `root`,
/// reranker files under `root/reranker`) is present with the expected byte
/// lengths. Cheap (metadata only); `load_verified_*` still hashes every file
/// before use.
pub fn bundle_complete(root: &Path) -> bool {
    minilm_artifact_contract_matches(root)
        && RERANKER_FILES.iter().all(|(name, expected_size, _)| {
            root.join("reranker")
                .join(name)
                .symlink_metadata()
                .is_ok_and(|metadata| {
                    metadata.file_type().is_file() && metadata.len() == *expected_size
                })
        })
}

fn bundle_url(repository: &str, revision: &str, file: &str) -> String {
    format!("https://huggingface.co/{repository}/resolve/{revision}/{file}")
}

/// One pinned artifact set: target directory, repository, revision, files.
type BundleSet<'a> = (&'a Path, &'a str, &'a str, &'a [(&'a str, u64, &'a str)]);

/// Download the pinned MiniLM bundle into `root`, one file at a time, each
/// verified by byte length and SHA-256 before it is renamed into place.
/// Files that already verify are left alone, so a partial earlier run
/// resumes. Any mismatch leaves the previous state untouched and fails.
pub async fn download_bundle(root: &Path) -> Result<(), String> {
    download_bundle_inner(root)
        .await
        .map_err(|error| error.to_string())
}

async fn download_bundle_inner(root: &Path) -> Result<(), SemanticUnavailable> {
    let client = reqwest::Client::builder()
        .user_agent(concat!("iii-directory/", env!("CARGO_PKG_VERSION")))
        .timeout(std::time::Duration::from_secs(600))
        .build()
        .map_err(|error| unavailable(format!("build download client: {error}")))?;
    let sets: [BundleSet<'_>; 2] = [
        (root, MINILM_REPOSITORY, MINILM_REVISION, &MINILM_FILES),
        (
            &root.join("reranker"),
            RERANKER_REPOSITORY,
            RERANKER_REVISION,
            &RERANKER_FILES,
        ),
    ];
    for (dir, repository, revision, files) in sets {
        for (name, expected_size, expected_sha256) in files {
            let target = dir.join(name);
            if verify_artifacts(dir, &[(name, *expected_size, expected_sha256)]).is_ok() {
                continue;
            }
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent).map_err(|error| {
                    unavailable(format!("create {}: {error}", parent.display()))
                })?;
            }
            let url = bundle_url(repository, revision, name);
            tracing::info!(%url, bytes = expected_size, "downloading search model artifact");
            let partial = target.with_extension("part");
            let result =
                stream_verified(&client, &url, &partial, *expected_size, expected_sha256).await;
            if let Err(error) = result {
                let _ = std::fs::remove_file(&partial);
                return Err(error);
            }
            std::fs::rename(&partial, &target)
                .map_err(|error| unavailable(format!("publish {}: {error}", target.display())))?;
        }
    }
    Ok(())
}

/// Log a progress line whenever the download crosses another 10% of the
/// file; files under 1 MiB finish before a line would help.
const PROGRESS_LOG_MIN_BYTES: u64 = 1 << 20;

/// Tenths of `total` covered by `received` (0..=10).
fn progress_step(received: u64, total: u64) -> u64 {
    if total == 0 {
        return 10;
    }
    ((u128::from(received) * 10) / u128::from(total)).min(10) as u64
}

/// Stream `url` into `partial`, hashing as it goes, and reject the file
/// unless its length and SHA-256 match the pinned manifest. Never holds
/// the whole artifact in memory.
async fn stream_verified(
    client: &reqwest::Client,
    url: &str,
    partial: &Path,
    expected_size: u64,
    expected_sha256: &str,
) -> Result<(), SemanticUnavailable> {
    use std::io::Write;

    let mut response = client
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|error| unavailable(format!("fetch {url}: {error}")))?;
    let mut file = std::fs::File::create(partial)
        .map_err(|error| unavailable(format!("create {}: {error}", partial.display())))?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    let mut last_step = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|error| unavailable(format!("read {url}: {error}")))?
    {
        received += chunk.len() as u64;
        if received > expected_size {
            return Err(unavailable(format!(
                "{url}: exceeded the pinned length of {expected_size} bytes"
            )));
        }
        file.write_all(&chunk)
            .map_err(|error| unavailable(format!("write {}: {error}", partial.display())))?;
        hasher.update(&chunk);
        let step = progress_step(received, expected_size);
        if expected_size >= PROGRESS_LOG_MIN_BYTES && step > last_step {
            last_step = step;
            tracing::info!(
                %url,
                percent = step * 10,
                received_mb = received / (1 << 20),
                total_mb = expected_size / (1 << 20),
                "search model download progress"
            );
        }
    }
    file.flush()
        .map_err(|error| unavailable(format!("flush {}: {error}", partial.display())))?;
    if received != expected_size {
        return Err(unavailable(format!(
            "{url}: expected {expected_size} bytes, got {received}"
        )));
    }
    if format!("{:x}", hasher.finalize()) != expected_sha256 {
        return Err(unavailable(format!("{url}: SHA-256 mismatch")));
    }
    Ok(())
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

#[cfg(minilm)]
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

#[cfg(minilm)]
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
    let loaded = LoadedModel(Mutex::new(model));
    let probe = loaded.encode(&["model verification".into()], None)?;
    validate_vectors(&probe, 1, MINILM_DIMENSIONS)?;
    Ok(loaded)
}

#[cfg(not(minilm))]
fn load_verified_minilm(_path: &Path) -> Result<LoadedModel, SemanticUnavailable> {
    Err(unavailable("MiniLM is not compiled for this target"))
}

#[cfg(minilm)]
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

#[cfg(minilm)]
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

#[cfg(minilm)]
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

    #[test]
    fn bundle_url_targets_the_pinned_revision() {
        assert_eq!(
            bundle_url(MINILM_REPOSITORY, MINILM_REVISION, "onnx/model.onnx"),
            format!(
                "https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2/resolve/{MINILM_REVISION}/onnx/model.onnx"
            )
        );
    }

    #[test]
    fn progress_step_counts_tenths_and_saturates() {
        assert_eq!(progress_step(0, 100), 0);
        assert_eq!(progress_step(9, 100), 0);
        assert_eq!(progress_step(10, 100), 1);
        assert_eq!(progress_step(55, 100), 5);
        assert_eq!(progress_step(100, 100), 10);
        assert_eq!(progress_step(150, 100), 10, "never past 100%");
        assert_eq!(progress_step(0, 0), 10, "empty file is complete");
        assert_eq!(progress_step(u64::MAX, u64::MAX), 10, "no overflow");
    }

    #[test]
    fn bundle_complete_needs_every_pinned_file_at_its_size() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!bundle_complete(dir.path()));
        // Sparse files of the pinned lengths satisfy the metadata check
        // (the loader hashes them later; this predicate only gates download).
        for (sub, files) in [("", &MINILM_FILES[..]), ("reranker", &RERANKER_FILES[..])] {
            for (name, size, _) in files {
                let path = dir.path().join(sub).join(name);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::File::create(&path)
                    .unwrap()
                    .set_len(*size)
                    .unwrap();
            }
        }
        assert!(bundle_complete(dir.path()));
        std::fs::File::create(dir.path().join("reranker/tokenizer.json"))
            .unwrap()
            .set_len(1)
            .unwrap();
        assert!(
            !bundle_complete(dir.path()),
            "a short file breaks completeness"
        );
    }

    /// Network: fetches ~180 MB from Hugging Face. Proves the pinned URLs,
    /// lengths and hashes still agree.
    #[tokio::test(flavor = "multi_thread")]
    #[ignore = "downloads the pinned MiniLM bundle from Hugging Face"]
    async fn downloads_and_verifies_the_pinned_bundle() {
        // Show the progress lines under --nocapture.
        let _ = tracing_subscriber::fmt().with_env_filter("info").try_init();
        let dir = tempfile::tempdir().unwrap();
        download_bundle(dir.path())
            .await
            .expect("download + verify");
        assert!(bundle_complete(dir.path()));
        verify_artifacts(dir.path(), &MINILM_FILES).expect("embedding files verify");
        verify_artifacts(&dir.path().join("reranker"), &RERANKER_FILES)
            .expect("reranker files verify");
        // A second run is a no-op: everything already verifies.
        download_bundle(dir.path()).await.expect("idempotent");
    }

    #[test]
    fn vectors_require_exact_finite_dimensions() {
        assert!(validate_vectors(&[vec![0.0; MINILM_DIMENSIONS]], 1, MINILM_DIMENSIONS).is_ok());
        assert!(
            validate_vectors(&[vec![0.0; MINILM_DIMENSIONS - 1]], 1, MINILM_DIMENSIONS).is_err()
        );
        let mut non_finite = vec![0.0; MINILM_DIMENSIONS];
        non_finite[0] = f32::NAN;
        assert!(validate_vectors(&[non_finite], 1, MINILM_DIMENSIONS).is_err());
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
            vec![vec![0.0; MINILM_DIMENSIONS]]
        ));
        assert!(semantic.publish_for_test(
            "new",
            vec!["new::fn".into()],
            vec![vec![0.0; MINILM_DIMENSIONS]]
        ));
        assert_eq!(
            semantic.active_fingerprint_for_test().as_deref(),
            Some("new")
        );
        assert!(!semantic.publish_for_test("new", vec!["broken::fn".into()], Vec::new()));
    }

    #[cfg(minilm)]
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
}
