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
use std::path::{Path, PathBuf};
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

/// One slot per vocabulary id. Concurrent callers on a cold cache wait on
/// the one load in flight instead of each downloading and parsing the same
/// file; a failed load leaves the slot empty, so the next caller retries.
type Slot = Arc<tokio::sync::OnceCell<Loaded>>;

fn loaded() -> &'static Mutex<HashMap<String, Slot>> {
    static LOADED: OnceLock<Mutex<HashMap<String, Slot>>> = OnceLock::new();
    LOADED.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The vocabulary for `reference`, from memory, then disk, then the network.
///
/// `None` means the vocabulary could not be obtained — an offline machine on
/// a cold cache, or a reference that no longer resolves. The caller keeps its
/// own estimate rather than reporting a wrong count as exact; a poisoned
/// cache lock is treated the same way as a miss.
pub async fn resolve(reference: &VocabularyRef) -> Option<Loaded> {
    resolve_in(reference, &cache_dir()).await
}

/// `resolve` against an explicit cache dir, so tests never mutate the
/// process-wide environment.
async fn resolve_in(reference: &VocabularyRef, dir: &Path) -> Option<Loaded> {
    let slot = loaded()
        .lock()
        .ok()?
        .entry(reference.id.clone())
        .or_default()
        .clone();
    slot.get_or_try_init(|| async { load(reference, dir).await.ok_or(()) })
        .await
        .ok()
        .cloned()
}

async fn load(reference: &VocabularyRef, dir: &Path) -> Option<Loaded> {
    let path = dir.join(format!("{}.json", reference.id));
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
    Tokenizer::from_bytes(&bytes).ok().map(Arc::new)
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Concurrent counts on a cold cache share one fetch: a 33 MB vocabulary
    /// downloaded and parsed once per caller would multiply the cold start by
    /// the number of turns counting at that moment.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_cold_resolves_share_one_fetch() {
        let dir = std::env::temp_dir().join(format!("vocab-flight-{}", uuid::Uuid::new_v4()));
        let body = serde_json::json!({
            "version": "1.0", "truncation": null, "padding": null, "added_tokens": [],
            "normalizer": null, "post_processor": null, "decoder": null,
            "pre_tokenizer": {"type": "Split", "pattern": {"Regex": "[\\s\\S]"},
                              "behavior": "Isolated", "invert": false},
            "model": {"type": "WordLevel", "vocab": {"[UNK]": 0}, "unk_token": "[UNK]"},
        })
        .to_string();

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let served = hits.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                served.fetch_add(1, Ordering::SeqCst);
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 1024];
                    let _ = socket.read(&mut buf).await;
                    // Slow enough that every caller arrives while the first
                    // fetch is still in flight.
                    tokio::time::sleep(Duration::from_millis(300)).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(head.as_bytes()).await;
                    let _ = socket.write_all(body.as_bytes()).await;
                });
            }
        });

        let reference = VocabularyRef::new(
            format!("flight-{}", uuid::Uuid::new_v4()),
            format!("http://{addr}/tokenizer.json"),
        );
        let calls = (0..8).map(|_| {
            let reference = reference.clone();
            let dir = dir.clone();
            tokio::spawn(async move { resolve_in(&reference, &dir).await })
        });
        for call in futures::future::join_all(calls).await {
            assert!(call.unwrap().is_some());
        }
        assert_eq!(hits.load(Ordering::SeqCst), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
