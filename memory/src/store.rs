//! Filesystem store: one folder per bank, memories in an append-only
//! `memories.jsonl`, rules as `rules/<name>.md`.
//!
//! Layout under the configured `data_dir`:
//!
//! ```text
//! <data_dir>/<bank>/bank.yaml         # description
//! <data_dir>/<bank>/rules/*.md        # always-injected markdown rules
//! <data_dir>/<bank>/memories.jsonl    # append-only memory log (full records)
//! <data_dir>/.trash/<bank>-<ms>/      # deleted banks (never destroyed)
//! ```
//!
//! Crash-safety by construction: every mutation appends one fsynced line
//! BEFORE touching RAM, replay is last-wins by memory id, and the search
//! index lives only in RAM (rebuilt from the file at boot) — there is no
//! index persistence to diverge and no shutdown flush to get wrong.
//! [`Store::commit`] is the single choke point through which every memory
//! mutation flows: file, map, and index update together or not at all.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::MemoryError;
use crate::index::Bm25Index;
use crate::types::{now_ms, Memory, Rule};

const MEMORIES_FILE: &str = "memories.jsonl";
const VECTORS_FILE: &str = "vectors.jsonl";
const RULES_DIR: &str = "rules";
const BANK_META: &str = "bank.yaml";
const TRASH_DIR: &str = ".trash";
/// Weight of the cosine similarity in the fused recall score, sized so a
/// strong semantic match competes with a solid BM25 hit.
const SEMANTIC_WEIGHT: f32 = 2.2;
/// Semantic candidates below this cosine are noise, not recall.
const SEMANTIC_FLOOR: f32 = 0.25;

pub struct Store {
    root: PathBuf,
    banks: RwLock<HashMap<String, Arc<Bank>>>,
}

pub struct Bank {
    pub name: String,
    dir: PathBuf,
    pub description: String,
    inner: RwLock<BankInner>,
}

struct BankInner {
    memories: HashMap<String, Memory>,
    index: Bm25Index,
    /// Memory id -> (embedding model, embedding), loaded from the
    /// `vectors.jsonl` sidecar. Like the BM25 index this is a derived
    /// cache: rebuildable, never authoritative, and absent vectors just
    /// mean no semantic signal for that record yet.
    vectors: HashMap<String, (String, Vec<f32>)>,
}

/// One sidecar line: a memory's embedding (last-wins by id on replay).
#[derive(serde::Serialize, serde::Deserialize)]
struct VectorRecord {
    id: String,
    model: String,
    /// Memory revision the text was embedded at; an older revision means
    /// the text changed since, so the vector is stale.
    #[serde(default)]
    revision: u64,
    v: Vec<f32>,
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb: f32 = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na * nb)
}

/// Run blocking filesystem work without stalling the async worker thread:
/// `block_in_place` on the multi-thread runtime, inline on current-thread
/// runtimes (tests), which cannot yield this way.
fn run_blocking<T>(
    f: impl FnOnce() -> Result<T, MemoryError>,
) -> impl std::future::Future<Output = Result<T, MemoryError>> {
    let out = match tokio::runtime::Handle::current().runtime_flavor() {
        tokio::runtime::RuntimeFlavor::MultiThread => tokio::task::block_in_place(f),
        _ => f(),
    };
    std::future::ready(out)
}

/// What a commit did — callers translate this into the emitted event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommitKind {
    Created,
    Updated,
}

/// `[a-z0-9]` then `[a-z0-9_-]`, max 64 — bank and rule names become
/// directory entries, so the charset is locked down.
pub fn validate_name(name: &str) -> Result<(), MemoryError> {
    let ok = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(MemoryError::InvalidInput(format!(
            "name `{name}` must match [a-z0-9][a-z0-9_-]{{0,63}}"
        )))
    }
}

impl Store {
    /// Open the root, verify it is writable (a probe file — an unwritable
    /// data dir is boot-fatal; memory must never silently run in RAM), and
    /// load every bank.
    pub fn open(root: PathBuf) -> Result<Self, MemoryError> {
        std::fs::create_dir_all(&root)?;
        let probe = root.join(".write-probe");
        std::fs::write(&probe, b"ok")
            .map_err(|e| MemoryError::Storage(format!("data_dir not writable: {e}")))?;
        std::fs::remove_file(&probe).ok();

        let mut banks = HashMap::new();
        for entry in std::fs::read_dir(&root)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            match Bank::load(&name, entry.path()) {
                Ok(bank) => {
                    banks.insert(name, Arc::new(bank));
                }
                Err(e) => {
                    tracing::error!(bank = %name, error = %e, "skipping unloadable bank");
                }
            }
        }
        tracing::info!(root = %root.display(), banks = banks.len(), "memory store loaded");
        Ok(Self {
            root,
            banks: RwLock::new(banks),
        })
    }

    pub async fn bank_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.banks.read().await.keys().cloned().collect();
        names.sort();
        names
    }

    pub async fn bank(&self, name: &str) -> Result<Arc<Bank>, MemoryError> {
        self.banks
            .read()
            .await
            .get(name)
            .cloned()
            .ok_or_else(|| MemoryError::BankNotFound(name.to_string()))
    }

    /// Get-or-create. Creation is idempotent and cheap, so the default bank
    /// materializes on first use with zero setup.
    pub async fn ensure_bank(
        &self,
        name: &str,
        description: Option<&str>,
    ) -> Result<(Arc<Bank>, bool), MemoryError> {
        validate_name(name)?;
        if let Some(existing) = self.banks.read().await.get(name) {
            return Ok((existing.clone(), false));
        }
        let mut banks = self.banks.write().await;
        if let Some(existing) = banks.get(name) {
            return Ok((existing.clone(), false));
        }
        let dir = self.root.join(name);
        let desc = description.unwrap_or_default().to_string();
        let bank_name = name.to_string();
        let bank = run_blocking(move || {
            std::fs::create_dir_all(dir.join(RULES_DIR))?;
            std::fs::write(
                dir.join(BANK_META),
                format!(
                    "description: {}\n",
                    serde_yaml::to_string(&desc).unwrap_or_default().trim()
                ),
            )?;
            Bank::load(&bank_name, dir)
        })
        .await?;
        let bank = Arc::new(bank);
        banks.insert(name.to_string(), bank.clone());
        Ok((bank, true))
    }

    /// Move the bank folder into `.trash/<name>-<ms>` — recoverable by
    /// hand, never destroyed.
    pub async fn trash_bank(&self, name: &str) -> Result<String, MemoryError> {
        validate_name(name)?;
        let mut banks = self.banks.write().await;
        if !banks.contains_key(name) {
            return Err(MemoryError::BankNotFound(name.to_string()));
        }
        let trash = self.root.join(TRASH_DIR);
        std::fs::create_dir_all(&trash)?;
        let dest = trash.join(format!("{name}-{}", now_ms()));
        // Filesystem first: a failed rename must leave the bank usable,
        // not stranded on disk with no RAM entry until a reload.
        std::fs::rename(self.root.join(name), &dest)?;
        banks.remove(name);
        Ok(dest.display().to_string())
    }

    /// Drop all in-RAM state and reload from disk (the recovery hatch after
    /// hand-editing files). The full-disk replay runs off the async worker
    /// thread on the multi-thread runtime.
    pub async fn reload(&self) -> Result<usize, MemoryError> {
        let root = self.root.clone();
        let fresh = run_blocking(move || Store::open(root)).await?;
        let loaded = fresh.banks.into_inner();
        let count = loaded.values().len();
        *self.banks.write().await = loaded;
        Ok(count)
    }
}

impl Bank {
    fn load(name: &str, dir: PathBuf) -> Result<Self, MemoryError> {
        let description = std::fs::read_to_string(dir.join(BANK_META))
            .ok()
            .and_then(|raw| serde_yaml::from_str::<serde_yaml::Value>(&raw).ok())
            .and_then(|v| {
                v.get("description")
                    .and_then(|d| d.as_str().map(String::from))
            })
            .unwrap_or_default();

        let mut memories: HashMap<String, Memory> = HashMap::new();
        let memories_path = dir.join(MEMORIES_FILE);
        if memories_path.exists() {
            let file = std::fs::File::open(&memories_path)?;
            for (lineno, line) in std::io::BufReader::new(file).lines().enumerate() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                // A corrupt line (partial write from a crash) is skipped with
                // a warning, never a boot failure — the rest of the log is
                // intact by construction.
                match serde_json::from_str::<Memory>(&line) {
                    Ok(memory) => {
                        let keep = memories
                            .get(&memory.id)
                            .is_none_or(|prev| memory.revision >= prev.revision);
                        if keep {
                            memories.insert(memory.id.clone(), memory);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(bank = name, line = lineno + 1, error = %e, "skipping corrupt memory line");
                    }
                }
            }
        }

        let mut index = Bm25Index::default();
        for memory in memories.values().filter(|f| f.is_live()) {
            index.add(memory);
        }

        // Vector sidecar: last-wins by id, entries for unknown memories
        // dropped (they belong to trimmed history), corrupt lines skipped.
        let mut vectors: HashMap<String, (String, Vec<f32>)> = HashMap::new();
        let vectors_path = dir.join(VECTORS_FILE);
        if vectors_path.exists() {
            let file = std::fs::File::open(&vectors_path)?;
            for line in std::io::BufReader::new(file).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(rec) = serde_json::from_str::<VectorRecord>(&line) {
                    // Keep only vectors embedded at the record's CURRENT
                    // revision — an edit since the embed makes it stale.
                    let current = memories
                        .get(&rec.id)
                        .is_some_and(|m| m.revision == rec.revision);
                    if current {
                        vectors.insert(rec.id, (rec.model, rec.v));
                    }
                }
            }
        }

        Ok(Self {
            name: name.to_string(),
            dir,
            description,
            inner: RwLock::new(BankInner {
                memories,
                index,
                vectors,
            }),
        })
    }

    /// Persist one memory's embedding (sidecar append + RAM). Best-effort
    /// derived data: failures cost a re-embed, never memory integrity.
    pub async fn set_vector(&self, id: &str, model: &str, v: Vec<f32>) -> Result<(), MemoryError> {
        let mut inner = self.inner.write().await;
        let revision = match inner.memories.get(id) {
            Some(m) => m.revision,
            // The record vanished between embed and store; nothing to attach.
            None => return Ok(()),
        };
        let line = serde_json::to_string(&VectorRecord {
            id: id.to_string(),
            model: model.to_string(),
            revision,
            v: v.clone(),
        })
        .map_err(|e| MemoryError::Storage(format!("serialize vector: {e}")))?;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.dir.join(VECTORS_FILE))?;
        file.write_all(line.as_bytes())?;
        file.write_all(b"\n")?;
        inner.vectors.insert(id.to_string(), (model.to_string(), v));
        Ok(())
    }

    /// Live memories that have no embedding yet (backfill work list).
    pub async fn unembedded(&self, limit: usize, active_model: &str) -> Vec<Memory> {
        let inner = self.inner.read().await;
        inner
            .memories
            .values()
            .filter(|f| {
                f.is_live()
                    && match inner.vectors.get(&f.id) {
                        None => true,
                        // A vector from a different model than the active
                        // one is due a re-embed; empty active model means
                        // "provider default" and accepts whatever exists.
                        Some((model, _)) => !active_model.is_empty() && model != active_model,
                    }
            })
            .take(limit)
            .cloned()
            .collect()
    }

    pub async fn vector_count(&self) -> usize {
        self.inner.read().await.vectors.len()
    }

    /// THE single mutation choke point: append the full record (fsync),
    /// then update the map and the index together. Every save / update /
    /// pin / supersede / tombstone flows through here.
    pub async fn commit(&self, memory: Memory) -> Result<CommitKind, MemoryError> {
        let mut inner = self.inner.write().await;
        let line = serde_json::to_string(&memory)
            .map_err(|e| MemoryError::Storage(format!("serialize memory: {e}")))?;
        // The write lock must span the fsync (append order = replay
        // order), but a slow disk must not stall this Tokio worker thread:
        // on the multi-thread runtime, block_in_place keeps the ordering
        // while letting other tasks run (current-thread runtimes — tests —
        // cannot yield this way and just run inline).
        let path = self.dir.join(MEMORIES_FILE);
        run_blocking(move || -> Result<(), MemoryError> {
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)?;
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
            file.sync_data()?;
            Ok(())
        })
        .await?;

        let kind = if inner.memories.contains_key(&memory.id) {
            CommitKind::Updated
        } else {
            CommitKind::Created
        };
        if memory.is_live() {
            inner.index.add(&memory);
        } else {
            inner.index.remove(&memory.id);
        }
        // The text changed under this id: any stored vector embeds the old
        // text. Drop it so unembedded() re-lists the record for backfill.
        let stale_vector = inner
            .memories
            .get(&memory.id)
            .is_some_and(|prev| prev.text != memory.text);
        if stale_vector {
            inner.vectors.remove(&memory.id);
        }
        inner.memories.insert(memory.id.clone(), memory);
        Ok(kind)
    }

    pub async fn get(&self, id: &str) -> Option<Memory> {
        self.inner.read().await.memories.get(id).cloned()
    }

    /// Newest-first page. `include_superseded` also returns tombstoned /
    /// superseded records (the history view).
    pub async fn list(
        &self,
        limit: usize,
        offset: usize,
        include_superseded: bool,
    ) -> (Vec<Memory>, usize) {
        let inner = self.inner.read().await;
        let mut all: Vec<&Memory> = inner
            .memories
            .values()
            .filter(|f| include_superseded || f.is_live())
            .collect();
        all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        let total = all.len();
        let page = all.into_iter().skip(offset).take(limit).cloned().collect();
        (page, total)
    }

    pub async fn counts(&self) -> (usize, usize) {
        let inner = self.inner.read().await;
        let live = inner.memories.values().filter(|f| f.is_live()).count();
        let pinned = inner
            .memories
            .values()
            .filter(|f| f.is_live() && f.pinned)
            .count();
        (live, pinned)
    }

    /// Hybrid recall over live memories: BM25 + entity/corroboration/pin
    /// fusion, plus an optional semantic signal (cosine against the
    /// vector sidecar) that surfaces memories sharing MEANING but zero
    /// vocabulary with the query. Read-only; zero LLM at query time (the
    /// caller embeds the query, if at all). First-person pronouns expand
    /// to the `user` entity handle so identity questions reach the memories
    /// phrased third-person about the user.
    pub async fn recall(
        &self,
        query: &str,
        query_vec: Option<&[f32]>,
        limit: usize,
        half_life_days: u64,
        include_superseded: bool,
    ) -> Vec<(Memory, f32)> {
        let mut query_tokens = crate::index::tokenize(query);
        if query_tokens
            .iter()
            .any(|t| matches!(t.as_str(), "i" | "me" | "my" | "mine" | "myself"))
            && !query_tokens.iter().any(|t| t == "user")
        {
            query_tokens.push("user".to_string());
        }
        if query_tokens.is_empty() && query_vec.is_none() {
            return Vec::new();
        }
        let inner = self.inner.read().await;
        let mut bm25 = inner.index.score(&query_tokens);
        // The BM25 index only holds LIVE records, so a history query must
        // seed non-live candidates itself (score 0.0: entity, pin,
        // corroboration, and recency fusion still rank them).
        if include_superseded {
            for (id, m) in &inner.memories {
                if !m.is_live() {
                    bm25.entry(id.clone()).or_insert(0.0);
                }
            }
        }
        // Semantic candidates join the pool even without a lexical hit.
        let mut sem: HashMap<String, f32> = HashMap::new();
        if let Some(qv) = query_vec {
            for (id, (_, v)) in &inner.vectors {
                let c = cosine(qv, v);
                if c > SEMANTIC_FLOOR {
                    sem.insert(id.clone(), c);
                    bm25.entry(id.clone()).or_insert(0.0);
                }
            }
        }
        let now = now_ms();
        let mut scored: Vec<(Memory, f32)> = bm25
            .into_iter()
            .filter_map(|(id, s)| inner.memories.get(&id).map(|f| (f.clone(), s, id)))
            .filter(|(f, _, _)| include_superseded || f.is_live())
            .map(|(f, s, id)| {
                let mut fused =
                    crate::index::fused_score(s, &f, &query_tokens, now, half_life_days);
                if let Some(c) = sem.get(&id) {
                    fused += SEMANTIC_WEIGHT * c;
                }
                (f, fused)
            })
            .collect();
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        scored.truncate(limit);
        scored
    }

    /// The bank's strongest memories with no query: pinned first, then
    /// corroboration, then recency. The injection floor for turns whose
    /// query recalls little (identity questions, session openers).
    pub async fn top_memories(&self, limit: usize) -> Vec<Memory> {
        let inner = self.inner.read().await;
        let mut all: Vec<&Memory> = inner.memories.values().filter(|f| f.is_live()).collect();
        all.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then(b.corroboration.cmp(&a.corroboration))
                .then(b.updated_at.cmp(&a.updated_at))
        });
        all.into_iter().take(limit).cloned().collect()
    }

    // ---- rules ------------------------------------------------------------

    fn rules_dir(&self) -> PathBuf {
        self.dir.join(RULES_DIR)
    }

    pub fn list_rules(&self) -> Result<Vec<Rule>, MemoryError> {
        let dir = self.rules_dir();
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut rules = Vec::new();
        for entry in std::fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("md") {
                continue;
            }
            let name = path
                .file_stem()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    tracing::warn!(path = %path.display(), error = %e, "skipping unreadable rule file");
                    continue;
                }
            };
            let updated_at = entry
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0);
            rules.push(Rule {
                name,
                content,
                updated_at,
            });
        }
        rules.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(rules)
    }

    /// Atomic write (tmp + rename); empty content removes the rule.
    pub fn set_rule(&self, name: &str, content: &str) -> Result<(), MemoryError> {
        validate_name(name)?;
        let dir = self.rules_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{name}.md"));
        if content.trim().is_empty() {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            return Ok(());
        }
        let tmp = dir.join(format!(".{name}.md.tmp"));
        std::fs::write(&tmp, content)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{fingerprint, Confidence};

    fn memory(text: &str) -> Memory {
        Memory {
            id: fingerprint(text),
            text: text.into(),
            entities: vec![],
            confidence: Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
        }
    }

    #[tokio::test]
    async fn save_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, created) = store.ensure_bank("main", None).await.unwrap();
        assert!(created);
        bank.commit(memory("mike prefers formal writing"))
            .await
            .unwrap();

        // Simulate a restart: reopen from disk only.
        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        let hits = bank2.recall("formal writing", None, 5, 30, false).await;
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0.text, "mike prefers formal writing");
    }

    #[tokio::test]
    async fn last_wins_replay_and_tombstone_unindexes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();

        let mut f = memory("the api port is 3000");
        bank.commit(f.clone()).await.unwrap();
        f.text = "the api port is 4000".into();
        f.revision = 1;
        f.updated_at = now_ms();
        bank.commit(f.clone()).await.unwrap();

        let got = bank.get(&f.id).await.unwrap();
        assert_eq!(got.text, "the api port is 4000");
        assert_eq!(got.revision, 1);

        // Tombstone: still on disk + gettable, but out of recall.
        f.invalid_at = Some(now_ms());
        f.revision = 2;
        bank.commit(f.clone()).await.unwrap();
        assert!(bank.recall("api port", None, 5, 30, false).await.is_empty());
        assert!(bank.get(&f.id).await.unwrap().invalid_at.is_some());

        // And the same is true after a reopen.
        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        assert!(bank2
            .recall("api port", None, 5, 30, false)
            .await
            .is_empty());
        let (memories, total) = bank2.list(10, 0, true).await;
        assert_eq!(total, 1);
        assert!(memories[0].invalid_at.is_some());
    }

    #[tokio::test]
    async fn corrupt_line_is_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        bank.commit(memory("good memory one")).await.unwrap();
        // Simulate a torn write.
        {
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(bank.dir().join(MEMORIES_FILE))
                .unwrap();
            file.write_all(b"{\"id\": \"fp-torn").unwrap();
            file.write_all(b"\n").unwrap();
        }
        bank.commit(memory("good memory two")).await.unwrap();

        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        let (_, total) = bank2.list(10, 0, false).await;
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn rules_roundtrip_and_empty_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("blog", Some("blog voice")).await.unwrap();
        bank.set_rule("style", "# Style\nNo em-dashes.").unwrap();
        let rules = bank.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "style");
        bank.set_rule("style", "").unwrap();
        assert!(bank.list_rules().unwrap().is_empty());
    }

    #[tokio::test]
    async fn trash_bank_moves_not_deletes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("scratch", None).await.unwrap();
        bank.commit(memory("keep me recoverable")).await.unwrap();
        let dest = store.trash_bank("scratch").await.unwrap();
        assert!(std::path::Path::new(&dest).join(MEMORIES_FILE).exists());
        assert!(store.bank("scratch").await.is_err());
    }

    #[test]
    fn name_validation() {
        assert!(validate_name("main").is_ok());
        assert!(validate_name("mikes-blog_2").is_ok());
        assert!(validate_name("").is_err());
        assert!(validate_name("Bad").is_err());
        assert!(validate_name("../evil").is_err());
        assert!(validate_name("-lead").is_err());
    }
}
