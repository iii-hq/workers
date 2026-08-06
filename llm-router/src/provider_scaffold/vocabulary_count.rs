//! Prompt token counting with a model's own vocabulary, for providers that
//! publish no metering endpoint.
//!
//! DeepSeek, GLM and the other open-weight families ship a HuggingFace
//! `tokenizer.json` — the same vocabulary the model was trained with, so a
//! local count is the real count rather than an approximation borrowed from
//! someone else's tokenizer. Counting one with tiktoken would be wrong in a
//! way nobody could see: the number looks authoritative and is off by
//! whatever the two vocabularies disagree about.
//!
//! A vocabulary is fetched once and cached on disk (`~/.iii/tokenizers/`),
//! then parsed once per process. That is what keeps a newly announced model
//! working without a release: the vocabulary is data resolved at runtime, not
//! a table compiled into the binary.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use tokenizers::Tokenizer;

use crate::provider_scaffold::chat_framing::count_framed_chat;
use crate::types::messages::AgentMessage;
use crate::types::model::AgentFunction;

/// The count came from the model's own vocabulary, computed locally.
pub const ESTIMATOR_TOKENIZER: &str = "tokenizer";

/// A vocabulary is a few megabytes over the wire, fetched at most once per
/// machine. Beyond this the caller keeps its estimate rather than holding a
/// turn open.
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Where a vocabulary comes from. A provider names its models' vocabulary
/// once; the file behind it never changes for a given model family, which is
/// why caching it forever on disk is safe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct VocabularyRef {
    /// Stable cache identity, also the on-disk filename stem.
    pub id: String,
    /// Where to fetch `tokenizer.json` when the cache misses.
    pub url: String,
}

impl VocabularyRef {
    pub fn new(id: impl Into<String>, url: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            url: url.into(),
        }
    }

    /// A HuggingFace repo's `tokenizer.json` on the default branch.
    pub fn huggingface(repo: &str) -> Self {
        Self::new(
            repo.replace('/', "--"),
            format!("https://huggingface.co/{repo}/resolve/main/tokenizer.json"),
        )
    }
}

fn cache_dir() -> PathBuf {
    std::env::var_os("III_TOKENIZER_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            home.join(".iii").join("tokenizers")
        })
}

type Loaded = Arc<Tokenizer>;

fn loaded() -> &'static Mutex<HashMap<String, Loaded>> {
    static LOADED: OnceLock<Mutex<HashMap<String, Loaded>>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The vocabulary for `reference`, from memory, then disk, then the network.
///
/// `None` means the vocabulary could not be obtained — an offline machine on
/// a cold cache, or a reference that no longer resolves. The caller keeps its
/// own estimate rather than reporting a wrong count as exact; a poisoned
/// cache lock is treated the same way as a miss.
pub async fn resolve(reference: &VocabularyRef) -> Option<Loaded> {
    if let Some(hit) = loaded().lock().ok()?.get(&reference.id).cloned() {
        return Some(hit);
    }

    let path = cache_dir().join(format!("{}.json", reference.id));
    let bytes = match tokio::fs::read(&path).await {
        Ok(bytes) => bytes,
        Err(_) => {
            let bytes = fetch(&reference.url).await?;
            // A partial write would poison the cache for every later run, so
            // the file is only named once it is whole.
            if let Some(parent) = path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            let staging = path.with_extension("json.partial");
            if tokio::fs::write(&staging, &bytes).await.is_ok() {
                let _ = tokio::fs::rename(&staging, &path).await;
            }
            bytes
        }
    };

    let tokenizer = Tokenizer::from_bytes(&bytes).ok()?;
    let tokenizer = Arc::new(tokenizer);
    if let Ok(mut cache) = loaded().lock() {
        cache.insert(reference.id.clone(), tokenizer.clone());
    }
    Some(tokenizer)
}

async fn fetch(url: &str) -> Option<Vec<u8>> {
    let response = reqwest::Client::new()
        .get(url)
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        tracing::warn!(%url, status = %response.status(), "vocabulary fetch failed");
        return None;
    }
    response.bytes().await.ok().map(|b| b.to_vec())
}

/// Count an assembled chat request with `tokenizer`, framed the same way
/// every other local counter frames it.
pub fn count_chat_tokens(
    tokenizer: &Tokenizer,
    system_prompt: Option<&str>,
    tools: &[AgentFunction],
    messages: &[AgentMessage],
) -> u64 {
    count_framed_chat(system_prompt, tools, messages, |text| {
        tokenizer
            .encode(text, false)
            .map(|encoded| encoded.len() as u64)
            // An input the vocabulary cannot encode is not worth failing a
            // turn over; it is rare, and the row it belongs to still counts
            // its framing.
            .unwrap_or(0)
    })
}
