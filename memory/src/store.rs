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
//! Ordinary writes use [`Bank::commit`]; proof-bound consolidation uses
//! [`Bank::supersede`] to publish both revisions by an atomic log replacement.
//! Both keep file, map and index publication behind the bank write lock.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::error::MemoryError;
use crate::index::Bm25Index;
use crate::types::{now_ms, Memory, Rule};

#[path = "query_projection.rs"]
mod query_projection;

const MEMORIES_FILE: &str = "memories.jsonl";
// Bank Arcs from before reload may outlive their Store generation. Serialize
// their disk mutations too; an in-RAM bank lock alone cannot guard rename
// against an append from that older generation.
static MEMORY_LOG: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn memory_log_lock() -> std::sync::MutexGuard<'static, ()> {
    MEMORY_LOG.lock().unwrap_or_else(|e| e.into_inner())
}

fn replay_record(log: &[u8], id: &str) -> Option<Memory> {
    let mut latest: Option<Memory> = None;
    for line in log.split(|&byte| byte == b'\n') {
        if let Ok(memory) = serde_json::from_slice::<Memory>(line) {
            if memory.id == id
                && latest
                    .as_ref()
                    .is_none_or(|prev| memory.revision >= prev.revision)
            {
                latest = Some(memory);
            }
        }
    }
    latest
}
const VECTORS_FILE: &str = "vectors.jsonl";
const RULES_DIR: &str = "rules";
const BANK_META: &str = "bank.yaml";
const TRASH_DIR: &str = ".trash";
/// Weight of the semantic arm (cosine on an absolute scale from
/// `SEMANTIC_ANCHOR`, never divided by the pool's best) against the
/// max-normalized BM25 in the fused recall score.
const SEMANTIC_WEIGHT: f32 = 16.0;
/// Semantic candidates below this cosine are noise, not recall.
const SEMANTIC_FLOOR: f32 = 0.25;
/// Cosine the semantic arm counts from. Just below the floor on purpose:
/// at the floor itself the small cosine gap between a stale fact and its
/// update becomes a ratio past what the recency multiplier corrects (recall
/// eval: 3-4 traps at every weight from 2 to 24, against 2).
const SEMANTIC_ANCHOR: f32 = 0.2;
/// iii-directory's local embedder, reported as `<name>@<revision>`.
const MINILM_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// The semantic arm's `(floor, anchor, weight)` for the model the query was
/// embedded with; the constants above were measured on text-embedding-3-small.
/// MiniLM spreads a stale fact and its update further apart, so from 0.2 the
/// gap outgrew recency (recall eval: 4 traps against 2): raw cosine at a
/// quarter of the slope keeps BM25 and recency in the vote (2 traps), and its
/// relevant matches start lower, so the floor does too.
fn semantic_scale(model: &str) -> (f32, f32, f32) {
    if model.split('@').next() == Some(MINILM_MODEL) {
        (0.2, 0.0, 5.0)
    } else {
        (SEMANTIC_FLOOR, SEMANTIC_ANCHOR, SEMANTIC_WEIGHT)
    }
}

/// Vectors a bank needs before a cosine's z-score means anything: z >= 2
/// is impossible below 5 and noise below about 20.
const MIN_Z_POOL: usize = 20;

/// Why a recalled memory matched, for the injection hook to tell a hit
/// from a coincidence. Ranking never reads it.
#[derive(Debug, Clone, Default)]
pub struct Evidence {
    /// The question's content terms the memory holds: never a stopword,
    /// never the `user` a pronoun added, except `user` alone for a
    /// explicit identity question ("who am I"), or of `MEMORY_VERBS` asked
    /// of "me" ("what do you remember about me").
    pub matched: Vec<String>,
    /// Matched terms with an occurrence outside a recognized answer slot.
    /// Unknown projections retain all legacy matches, not a new admission rule.
    pub relation_matched: Vec<String>,
    /// Number of distinct eligible query terms with an occurrence outside the
    /// certified slot, including terms absent from this candidate. Repetition
    /// does not increase this count; a term inside AND outside the slot counts
    /// once as relational. Only a known projection uses this denominator.
    pub relation_terms: usize,
    /// Matched terms occurring inside a recognized answer slot. A repeated term
    /// may also be relational when it occurs outside that slot.
    pub requested_slot_matched: Vec<String>,
    /// Half-open token spans in the original query; None explicitly means
    /// unknown. This evidence is transient and is never persisted or ranked.
    pub requested_slot_spans: Option<Vec<std::ops::Range<usize>>>,
    /// How many content terms the question has.
    pub terms: usize,
    /// The question asks what memory holds of the user ("what do you
    /// remember about me?"), not a question of pronouns alone ("what should
    /// I do now?").
    pub of_me: bool,
    /// The query explicitly names a non-user entity handle (at least three
    /// characters); one incidental normalized BM25 word is not enough.
    pub named_subject: bool,
    /// A named subject is directly questioned (copula + name), or named in
    /// an explicit first-person question. Not an incidental object of "of".
    pub subject_question: bool,
    /// At least one content word is shared outside entity handles. A project
    /// name alone cannot answer every question about that project's facts.
    pub detail_match: bool,
    /// The same detail check using only occurrences outside an answer slot.
    pub relation_detail_match: bool,
    /// Reserved legacy evidence field; no rarity-only escape is emitted.
    pub specific_recall: bool,
    /// The question directly requests a stored attribute tag ("which units",
    /// "qual CI"). Entity names or incidental words are not attributes.
    pub attribute_match: bool,
    /// Another live fact establishes an explicitly requested attribute tag,
    /// while this candidate is on a different attribute of the same subject.
    pub attribute_conflict: bool,
    /// A delimited personal nominal scope or predicate plus its argument
    /// is attested together. A possessive window or bare verb is insufficient.
    pub personal_match: bool,
    /// Query-token ranges and argument/owner terms used by the transient audit.
    pub personal_scope_spans: Vec<[usize; 2]>,
    pub personal_scope_terms: Vec<Vec<String>>,
    pub personal_scope_predicates: Vec<String>,
    pub personal_scope_owners: Vec<String>,
    pub personal_scope_matched: bool,
    pub personal_scope_required: bool,
    /// A certified query-target shape is evaluated independently of scores.
    pub target_scope_required: bool,
    pub target_scope_matched: bool,
    pub target_scope_span: Option<[usize; 2]>,
    pub target_scope_kind: Option<String>,
    pub target_scope_terms: Vec<String>,
    pub target_scope_predicate: Option<String>,
    pub target_scope_subject: Vec<String>,
    pub target_scope_object: Vec<String>,
    pub target_scope_relative: Vec<String>,
    /// Deterministic obligation witnesses, computed from raw text before
    /// selection. None means a literal/unknown quantity, not negative support.
    pub target_obligation: Option<serde_json::Value>,
    /// Transient user -> assistant -> output witnesses; neither ranking nor
    /// stored metadata supplies a missing grammatical role.
    pub directive_scope_required: bool,
    pub directive_scope_matched: bool,
    pub target_directive: Option<serde_json::Value>,
    /// Query names a known non-user subject, but this candidate is on none
    /// of those subjects. Prevent borrowing another project's attribute.
    pub subject_conflict: bool,
    /// Explicit predicate conflict supplied by evidence consumers. Recall does
    /// not infer this from another memory containing an isolated query verb.
    pub predicate_conflict: bool,
    /// BM25 over the shared max: the lexical part of `relevance`.
    pub lexical: f32,
    /// `lexical + weight * semantic`, before entity, corroboration, pin
    /// and recency boosts: those rank, they do not say what is relevant.
    pub relevance: f32,
    /// The cosine, when above the model's floor (`semantic_scale`).
    pub cosine: Option<f32>,
    /// Standard deviations above the question's mean cosine over this
    /// bank, for a cosine above the floor in a bank of MIN_Z_POOL vectors.
    pub z: Option<f32>,
    /// Whether the memory has a vector of the query's model at all.
    pub vector: bool,
}

impl Evidence {
    /// Legacy/test evidence without a certified projection keeps its behavior.
    pub(crate) fn relation_words(&self) -> &[String] {
        if self.requested_slot_spans.is_some() {
            &self.relation_matched
        } else {
            &self.matched
        }
    }
    /// Unknown projections retain the legacy question denominator. Never
    /// derive sufficiency from the candidate's matched subset.
    pub(crate) fn relation_term_count(&self) -> usize {
        if self.requested_slot_spans.is_some() {
            self.relation_terms
        } else {
            self.terms
        }
    }
}

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
#[derive(serde::Deserialize)]
struct VectorRecord {
    id: String,
    model: String,
    /// The exact text embedded: the vector is current while the memory
    /// still has this text (pin/tag/reinforce bumps leave it valid).
    text: String,
    v: Vec<f32>,
}

/// The write side of [`VectorRecord`], borrowing instead of cloning.
#[derive(serde::Serialize)]
struct VectorLine<'a> {
    id: &'a str,
    model: &'a str,
    text: &'a str,
    v: &'a [f32],
}

/// Serializes every vector sidecar replay+compaction with every append.
/// During `memory::reload` (or a `data_dir` swap) the OLD store's banks can
/// still append while the NEW store's bank compacts the same file: an
/// append landing on the inode the rename replaces would be lost.
static SIDECAR: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn sidecar_lock() -> std::sync::MutexGuard<'static, ()> {
    SIDECAR.lock().unwrap_or_else(|e| e.into_inner())
}

/// Rewrite the sidecar to exactly the vectors `open` kept: temp file,
/// fsync, rename over. A crash leaves the old file or the new one, never a
/// torn one; a failed rewrite removes its temp (a partial copy would only
/// eat the disk the authoritative memories.jsonl appends need).
fn compact_vectors(
    path: &Path,
    vectors: &HashMap<String, (String, Vec<f32>)>,
    memories: &HashMap<String, Memory>,
) -> std::io::Result<()> {
    // The caller holds `sidecar_lock()`, so no concurrent open writes the
    // same temp file and no append slips in between replay and rename.
    let tmp = path.with_extension("jsonl.tmp");
    let rewrite = || -> std::io::Result<()> {
        let mut out = std::io::BufWriter::new(std::fs::File::create(&tmp)?);
        for (id, (model, v)) in vectors {
            // Kept means it embeds the record's current text.
            let text = memories.get(id).map_or("", |m| m.text.as_str());
            serde_json::to_writer(&mut out, &VectorLine { id, model, text, v })?;
            out.write_all(b"\n")?;
        }
        out.into_inner().map_err(|e| e.into_error())?.sync_all()?;
        std::fs::rename(&tmp, path)
    };
    rewrite().inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// Replace a small file whole: hidden temp file beside it, fsync, rename
/// over. Readers observe one complete version after rename. No directory
/// fsync is issued: this is not a universal power-loss durability guarantee.
fn write_atomic(path: &Path, content: &[u8]) -> std::io::Result<()> {
    let mut name = std::ffi::OsString::from(".");
    name.push(path.file_name().unwrap_or_default());
    name.push(".tmp");
    let tmp = path.with_file_name(name);
    let mut file = std::fs::File::create(&tmp)?;
    file.write_all(content)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)
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

/// English/Portuguese function words dropped from QUERIES before lexical scoring (the
/// index keeps every token), as Postgres's `english` tsvector drops them.
/// Without it a natural-language question matches every
/// memory through "the/do/my", and that BM25 noise outscores a paraphrase's
/// semantic match. "us" is kept (US), as Snowball's English list does.
// `pub` only so the recall_quality eval checks its data against this list.
#[doc(hidden)]
#[rustfmt::skip]
pub const STOPWORDS: &[&str] = &[
    "a", "about", "again", "all", "also", "am", "an", "and", "any", "are", "as", "at", "be",
    "been", "being", "but", "by", "can", "could", "d", "did", "do", "does", "doing", "for",
    "from", "had", "has", "have", "having", "he", "her", "here", "hers", "him", "his", "how",
    "i", "if", "in", "into", "is", "it", "its", "just", "ll", "m", "me", "mine", "more", "most",
    "my", "myself", "no", "not", "now", "of", "off", "on", "only", "or", "other", "our", "out",
    "over", "own", "re", "s", "same", "she", "should", "so", "some", "still", "such", "t",
    "than", "that", "the", "their", "them", "then", "there", "these", "they", "this", "those",
    "to", "too", "under", "up", "ve", "very", "was", "we", "were", "what", "when", "where",
    "which", "who", "whom", "whose", "why", "will", "with", "would", "you", "your", "yours",
    "ao", "aos", "as", "até", "com", "como", "da", "das", "de", "do", "dos", "e", "ela",
    "ele", "em", "entre", "era", "essa", "esse", "esta", "este", "eu", "foi", "isso", "isto",
    "já", "mas", "meu", "meus", "mim", "minha", "minhas", "na", "nas", "nem", "no", "nos",
    "nós", "o", "ou", "para", "pela", "pelo", "por", "qual", "quais", "que", "quem",
    "se", "sem", "ser", "sua", "suas", "um", "uma", "uns", "umas", "você", "vocês", "é",
];

/// Verbs that ask what memory holds in English/Portuguese.
/// Not "tell": its "me" is who is told ("tell me more").
const MEMORY_VERBS: &[&str] = &[
    "remember", "know", "recall", "lembre", "lembra", "lembrar", "recorde", "recordar", "sabe",
];

/// Bytes of a recall query kept: half from its head, half from its tail.
const QUERY_MAX_BYTES: usize = 800;

/// A recall query within [`QUERY_MAX_BYTES`]: a longer one (a pasted log, a
/// sub-agent's brief) keeps its head and tail, where a request sits around
/// a paste. Its middle would dilute BM25 over every memory sharing a word
/// and can overrun or time out the query embed.
// ponytail: identifiers mid-paste that match memories are dropped; measure
// on a long-query eval before tuning the size.
pub(crate) fn cap_query(query: &str) -> String {
    if query.len() <= QUERY_MAX_BYTES {
        return query.to_string();
    }
    let half = QUERY_MAX_BYTES / 2;
    let head = crate::extract::clamp_char_boundary(query, half);
    let mut tail = query.len() - half;
    while !query.is_char_boundary(tail) {
        tail += 1;
    }
    format!("{}\n{}", &query[..head], &query[tail..])
}

/// Query tokens without stopwords (all kept when nothing else is left) or
/// repeats (BM25 would count a pasted term once per occurrence), with
/// first-person pronouns expanded to the `user` entity handle so identity
/// questions reach memories phrased about the user. The flag is true when
/// the pronoun added `user`, which a typed "user" does not: that `user`
/// ranks, but is [`Evidence`] only for a question of pronouns alone (or
/// with [`MEMORY_VERBS`]).
fn query_tokens(query: &str) -> (Vec<String>, bool) {
    let mut tokens = crate::index::tokenize(query);
    let mut seen = std::collections::HashSet::new();
    tokens.retain(|t| seen.insert(t.clone()));
    let first_person = tokens.iter().any(|t| {
        matches!(
            t.as_str(),
            "i" | "me"
                | "my"
                | "mine"
                | "myself"
                | "eu"
                | "mim"
                | "meu"
                | "meus"
                | "minha"
                | "minhas"
        )
    });
    if tokens.iter().any(|t| !STOPWORDS.contains(&t.as_str())) {
        tokens.retain(|t| !STOPWORDS.contains(&t.as_str()));
    }
    let expanded = first_person && !tokens.iter().any(|t| t == "user");
    if expanded {
        tokens.push("user".to_string());
    }
    (tokens, expanded)
}

/// Raw recall keeps its historical English query normalization. Portuguese
/// content/inflection handling belongs to Evidence for injection selection,
/// not a silent reranking of public recall or its accepted metric snapshots.
fn ranking_tokens(query: &str) -> Vec<String> {
    let mut tokens = crate::index::tokenize(query);
    let mut seen = HashSet::new();
    tokens.retain(|t| seen.insert(t.clone()));
    let personal = tokens
        .iter()
        .any(|t| matches!(t.as_str(), "i" | "me" | "my" | "mine" | "myself"));
    let english = &STOPWORDS[..STOPWORDS
        .iter()
        .position(|w| *w == "ao")
        .expect("English prefix")];
    let english_stop = |t: &str| english.contains(&t);
    if tokens.iter().any(|t| !english_stop(t)) {
        tokens.retain(|t| !english_stop(t));
    }
    if personal && !tokens.iter().any(|t| t == "user") {
        tokens.push("user".into());
    }
    tokens
}

/// Score descending, ties by id (HashMap iteration order would otherwise
/// rank equal scores differently per process), then cap.
fn rank_desc(scored: &mut Vec<(Memory, f32)>, limit: usize) {
    scored.sort_by(|a, b| {
        b.1.partial_cmp(&a.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.0.id.cmp(&b.0.id))
    });
    scored.truncate(limit);
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
            write_atomic(
                &dir.join(BANK_META),
                format!(
                    "description: {}\n",
                    serde_yaml::to_string(&desc).unwrap_or_default().trim()
                )
                .as_bytes(),
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

    /// Remove the bank folder outright, skipping `.trash`. Only for a
    /// scratch bank its caller created and nothing else wrote to.
    pub async fn destroy_bank(&self, name: &str) -> Result<(), MemoryError> {
        validate_name(name)?;
        let mut banks = self.banks.write().await;
        if !banks.contains_key(name) {
            return Err(MemoryError::BankNotFound(name.to_string()));
        }
        std::fs::remove_dir_all(self.root.join(name))?;
        banks.remove(name);
        Ok(())
    }

    /// Deleted: no live folder, but a `.trash/<name>-<ms>` one. A path that
    /// picks the bank implicitly (a session's bank, extraction) must not
    /// bring it back; an explicit `bank` may.
    pub fn is_deleted(&self, name: &str) -> bool {
        if self.root.join(name).is_dir() {
            return false;
        }
        let prefix = format!("{name}-");
        std::fs::read_dir(self.root.join(TRASH_DIR))
            .into_iter()
            .flatten()
            .flatten()
            .any(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .and_then(|n| n.strip_prefix(&prefix))
                    .is_some_and(|ms| !ms.is_empty() && ms.bytes().all(|b| b.is_ascii_digit()))
            })
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

        // Vector sidecar: last-wins by id; entries of retired or unknown
        // memories, of text since edited, and corrupt or pre-text-key lines
        // are skipped (those memories are backfill work again).
        let mut vectors: HashMap<String, (String, Vec<f32>)> = HashMap::new();
        let vectors_path = dir.join(VECTORS_FILE);
        let mut lines = 0usize;
        let sidecar = sidecar_lock();
        if vectors_path.exists() {
            let file = std::fs::File::open(&vectors_path)?;
            for line in std::io::BufReader::new(file).lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                lines += 1;
                if let Ok(rec) = serde_json::from_str::<VectorRecord>(&line) {
                    // Keep only vectors of the record's CURRENT text — a
                    // text edit since the embed makes it stale.
                    let current = memories
                        .get(&rec.id)
                        .is_some_and(|m| m.is_live() && m.text == rec.text);
                    if current {
                        vectors.insert(rec.id, (rec.model, rec.v));
                    }
                }
            }
        }
        // Dead lines (edited text, retired records, re-embeds, pre-text-key
        // lines, torn writes) are re-parsed every open and never reclaimed
        // otherwise; ~19 KB each for a 1536-dim vector.
        // ponytail: compacts at open only; a long uptime grows until the
        // next boot or memory::reload.
        if lines > 2 * vectors.len() {
            if let Err(e) = compact_vectors(&vectors_path, &vectors, &memories) {
                tracing::warn!(bank = name, error = %e, "vector sidecar compaction failed");
            }
        }
        drop(sidecar);

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

    /// Persist one memory's embedding of `embedded_text` (sidecar append +
    /// RAM); false when it was discarded. Best-effort derived data: failures
    /// cost a re-embed, never memory integrity.
    pub async fn set_vector(
        &self,
        id: &str,
        model: &str,
        embedded_text: &str,
        v: Vec<f32>,
    ) -> Result<bool, MemoryError> {
        let mut inner = self.inner.write().await;
        // The record vanished, or its text was edited while the embed was
        // in flight: this vector is of text the record no longer has, and
        // the record stays listed by `unembedded` for the new text.
        if !inner
            .memories
            .get(id)
            .is_some_and(|m| m.is_live() && m.text == embedded_text)
        {
            return Ok(false);
        }
        let line = serde_json::to_string(&VectorLine {
            id,
            model,
            text: embedded_text,
            v: &v,
        })
        .map_err(|e| MemoryError::Storage(format!("serialize vector: {e}")))?;
        {
            let _sidecar = sidecar_lock();
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(self.dir.join(VECTORS_FILE))?;
            file.write_all(line.as_bytes())?;
            file.write_all(b"\n")?;
        }
        inner.vectors.insert(id.to_string(), (model.to_string(), v));
        Ok(true)
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
                        // A vector from another model than the one the
                        // provider currently reports is in another space:
                        // due a re-embed. Empty = that model is not known
                        // yet, so whatever exists is accepted.
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
        self.commit_inner(memory, None).await
    }

    /// Guard read/modify/write callers against a consolidation or edit that
    /// committed after their read. None means the id must still be absent.
    pub async fn commit_expected(
        &self,
        memory: Memory,
        expected: Option<&Memory>,
    ) -> Result<CommitKind, MemoryError> {
        self.commit_inner(memory, Some(expected)).await
    }

    async fn commit_inner(
        &self,
        mut memory: Memory,
        expected: Option<Option<&Memory>>,
    ) -> Result<CommitKind, MemoryError> {
        // The backstop for every write path: no known secret shape reaches
        // memories.jsonl, whoever built the record (the entry points redact
        // first, so dedup fingerprints the redacted text).
        let mut redacted = crate::redact::redact(&memory.text);
        redacted
            .labels
            .extend(crate::redact::drop_secrets(&mut memory.entities));
        redacted
            .labels
            .extend(crate::redact::drop_secrets(&mut memory.tags));
        if !redacted.labels.is_empty() {
            tracing::warn!(bank = %self.name, id = %memory.id, labels = ?redacted.labels,
                "redacted secrets from a memory at commit");
            memory.text = redacted.text;
        }
        let mut inner = self.inner.write().await;
        if let Some(expected) = expected {
            if inner.memories.get(&memory.id) != expected {
                return Err(MemoryError::InvalidInput(format!(
                    "memory `{}` changed before commit",
                    memory.id
                )));
            }
        }
        let line = serde_json::to_string(&memory)
            .map_err(|e| MemoryError::Storage(format!("serialize memory: {e}")))?;
        // The write lock must span the fsync (append order = replay
        // order), but a slow disk must not stall this Tokio worker thread:
        // on the multi-thread runtime, block_in_place keeps the ordering
        // while letting other tasks run (current-thread runtimes — tests —
        // cannot yield this way and just run inline).
        let path = self.dir.join(MEMORIES_FILE);
        let expected_disk = expected.map(|value| value.cloned());
        let memory_id = memory.id.clone();
        run_blocking(move || -> Result<(), MemoryError> {
            let _log = memory_log_lock();
            if let Some(expected) = expected_disk {
                let log = match std::fs::read(&path) {
                    Ok(log) => log,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                    Err(e) => return Err(e.into()),
                };
                if replay_record(&log, &memory_id) != expected {
                    return Err(MemoryError::InvalidInput(format!(
                        "memory `{memory_id}` changed on disk before commit"
                    )));
                }
            }
            let mut file = std::fs::OpenOptions::new()
                .create(true)
                .read(true)
                .append(true)
                .open(path)?;
            // A last line without its newline (a hand edit, a torn write)
            // would fuse with this record: both unparseable on replay, and
            // a lost tombstone brings its memory back. One buffer, one
            // write: no kill between record and newline.
            let mut buf = Vec::with_capacity(line.len() + 2);
            if file.metadata()?.len() > 0 {
                let mut last = [0u8];
                file.seek(std::io::SeekFrom::End(-1))?;
                file.read_exact(&mut last)?;
                if last[0] != b'\n' {
                    buf.push(b'\n');
                }
            }
            buf.extend_from_slice(line.as_bytes());
            buf.push(b'\n');
            file.write_all(&buf)?;
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
        let stale_vector = !memory.is_live()
            || inner
                .memories
                .get(&memory.id)
                .is_some_and(|prev| prev.text != memory.text);
        if stale_vector {
            inner.vectors.remove(&memory.id);
        }
        inner.memories.insert(memory.id.clone(), memory);
        Ok(kind)
    }

    /// Publish an extraction correction as one transition, before either side
    /// becomes visible. Both snapshots are from the extraction input, not a
    /// post-inference reread. A stale target, failed write or retry cannot leave
    /// a newly published replacement beside an unretired original.
    pub async fn correct_expected(
        &self,
        old: &Memory,
        replacement: Memory,
        expected_replacement: Option<&Memory>,
    ) -> Result<Memory, MemoryError> {
        if old.id == replacement.id
            || old.pinned
            || !old.is_live()
            || !replacement.is_live()
            || expected_replacement.is_some_and(|m| !m.is_live() || (m.pinned && m != &replacement))
        {
            return Err(MemoryError::InvalidInput(
                "invalid correction boundary".into(),
            ));
        }
        for record in [old, &replacement] {
            let mut entities = record.entities.clone();
            let mut tags = record.tags.clone();
            if !crate::redact::redact(&record.text).labels.is_empty()
                || !crate::redact::drop_secrets(&mut entities).is_empty()
                || !crate::redact::drop_secrets(&mut tags).is_empty()
            {
                return Err(MemoryError::InvalidInput(
                    "correction requires redacted records".into(),
                ));
            }
        }
        crate::redact::reject_injection("correction", &replacement.text)?;
        let mut inner = self.inner.write().await;
        if inner.memories.get(&old.id) != Some(old)
            || inner.memories.get(&replacement.id) != expected_replacement
        {
            return Err(MemoryError::InvalidInput(
                "correction records changed before commit".into(),
            ));
        }
        let (retired, _) = crate::functions::items::supersede_transition(
            old.clone(),
            replacement.clone(),
            false,
            now_ms(),
        );
        let mut suffix = Vec::new();
        for record in [&replacement, &retired] {
            serde_json::to_writer(&mut suffix, record)
                .map_err(|e| MemoryError::Storage(e.to_string()))?;
            suffix.push(b'\n');
        }
        let path = self.dir.join(MEMORIES_FILE);
        let disk_old = old.clone();
        let disk_expected = expected_replacement.cloned();
        let replacement_id = replacement.id.clone();
        run_blocking(move || -> Result<(), MemoryError> {
            let _log = memory_log_lock();
            let mut log = match std::fs::read(&path) {
                Ok(log) => log,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e.into()),
            };
            if replay_record(&log, &disk_old.id).as_ref() != Some(&disk_old)
                || replay_record(&log, &replacement_id) != disk_expected
            {
                return Err(MemoryError::InvalidInput(
                    "correction records changed on disk".into(),
                ));
            }
            if !log.is_empty() && !log.ends_with(b"\n") {
                log.push(b'\n');
            }
            log.extend_from_slice(&suffix);
            write_atomic(&path, &log)?;
            Ok(())
        })
        .await?;
        // Readers acquire the same lock, so neither RAM nor the index exposes
        // half of the transition. Replaced text invalidates its old embedding.
        if expected_replacement.is_some_and(|m| m.text != replacement.text) {
            inner.vectors.remove(&replacement.id);
        }
        inner.index.add(&replacement);
        inner.index.remove(&retired.id);
        inner.vectors.remove(&retired.id);
        inner.memories.insert(replacement.id.clone(), replacement);
        inner.memories.insert(retired.id.clone(), retired.clone());
        Ok(retired)
    }

    /// Joint compare-and-write for consolidation. The bank write lock covers
    /// BOTH preconditions, transition, durable replacement and index publication.
    /// Retaining ordinary JSONL records preserves legacy replay and full history.
    pub async fn supersede(
        &self,
        id: &str,
        winner_id: &str,
        reinforce: bool,
        expected: (Option<&serde_json::Value>, Option<&serde_json::Value>),
    ) -> Result<(Memory, Option<Memory>), MemoryError> {
        if id == winner_id {
            return Err(MemoryError::InvalidInput(
                "a memory cannot supersede itself".into(),
            ));
        }
        let mut inner = self.inner.write().await;
        let loser = inner
            .memories
            .get(id)
            .cloned()
            .ok_or_else(|| MemoryError::MemoryNotFound(id.into()))?;
        let winner = inner
            .memories
            .get(winner_id)
            .cloned()
            .ok_or_else(|| MemoryError::MemoryNotFound(winner_id.into()))?;
        for (record, snapshot) in [(&loser, expected.0), (&winner, expected.1)] {
            if let Some(snapshot) = snapshot {
                // A snapshot may be a full record or a revision-bound projection.
                // Unknown fields are rejected rather than silently ignored.
                let fields = snapshot
                    .as_object()
                    .filter(|fields| !fields.is_empty())
                    .ok_or_else(|| {
                        MemoryError::InvalidInput(
                            "expected snapshot must be a nonempty object".into(),
                        )
                    })?;
                if fields
                    .get("revision")
                    .and_then(serde_json::Value::as_u64)
                    .is_none()
                {
                    return Err(MemoryError::InvalidInput(
                        "expected snapshot requires an unsigned revision".into(),
                    ));
                }
                let current = serde_json::to_value(record)
                    .map_err(|e| MemoryError::Storage(e.to_string()))?;
                if !fields.iter().all(|(key, value)| {
                    current.get(key).unwrap_or(&serde_json::Value::Null) == value
                        && (current.get(key).is_some()
                            || matches!(
                                key.as_str(),
                                "source" | "stated_at" | "invalid_at" | "superseded_by"
                            ))
                }) {
                    return Err(MemoryError::InvalidInput(format!(
                        "memory `{}` changed since expected snapshot",
                        record.id
                    )));
                }
            }
        }
        if loser.pinned || !loser.is_live() || !winner.is_live() {
            return Err(MemoryError::InvalidInput(
                "supersede requires a live unpinned loser and a live winner".into(),
            ));
        }
        // Loaded legacy records can predate commit-time redaction. A merge
        // cannot silently change the text certified by its caller, nor append
        // a secret again: reject and leave both originals untouched instead.
        for record in [&loser, &winner] {
            let mut entities = record.entities.clone();
            let mut tags = record.tags.clone();
            if !crate::redact::redact(&record.text).labels.is_empty()
                || !crate::redact::drop_secrets(&mut entities).is_empty()
                || !crate::redact::drop_secrets(&mut tags).is_empty()
            {
                return Err(MemoryError::InvalidInput(
                    "supersede requires redacted records".into(),
                ));
            }
        }
        let disk_loser = loser.clone();
        let disk_winner = winner.clone();
        let (retired, survivor) =
            crate::functions::items::supersede_transition(loser, winner, reinforce, now_ms());
        let path = self.dir.join(MEMORIES_FILE);
        let records = std::iter::once(&retired).chain(survivor.iter());
        let mut suffix = Vec::new();
        for memory in records {
            serde_json::to_writer(&mut suffix, memory)
                .map_err(|e| MemoryError::Storage(e.to_string()))?;
            suffix.push(b'\n');
        }
        // Replacing the complete log is intentionally conservative: two plain
        // appended records could tear between loser and winner. Keep ALL old
        // bytes (including corrupt lines) for audit; rename publishes both or
        // neither, and a failed temp write/fsync/rename never changes RAM/log.
        run_blocking(move || -> Result<(), MemoryError> {
            let _log = memory_log_lock();
            let mut log = match std::fs::read(&path) {
                Ok(log) => log,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
                Err(e) => return Err(e.into()),
            };
            // Validate against the actual authoritative bytes as well: a
            // reload may leave another live Bank Arc on this same path.
            if replay_record(&log, &disk_loser.id).as_ref() != Some(&disk_loser)
                || replay_record(&log, &disk_winner.id).as_ref() != Some(&disk_winner)
            {
                return Err(MemoryError::InvalidInput(
                    "supersede records changed on disk".into(),
                ));
            }
            if !log.is_empty() && !log.ends_with(b"\n") {
                log.push(b'\n');
            }
            log.extend_from_slice(&suffix);
            write_atomic(&path, &log)?;
            Ok(())
        })
        .await?;
        for memory in std::iter::once(&retired).chain(survivor.iter()) {
            if memory.is_live() {
                inner.index.add(memory);
            } else {
                inner.index.remove(&memory.id);
                inner.vectors.remove(&memory.id);
            }
            inner.memories.insert(memory.id.clone(), memory.clone());
        }
        Ok((retired, survivor))
    }

    pub async fn get(&self, id: &str) -> Option<Memory> {
        self.inner.read().await.memories.get(id).cloned()
    }

    /// Newest-first page. `include_superseded` also returns tombstoned /
    /// superseded records (the history view); `tag` narrows to memories
    /// carrying that label.
    pub async fn list(
        &self,
        limit: usize,
        offset: usize,
        include_superseded: bool,
        tag: Option<&str>,
    ) -> (Vec<Memory>, usize) {
        let inner = self.inner.read().await;
        let mut all: Vec<&Memory> = inner
            .memories
            .values()
            .filter(|f| include_superseded || f.is_live())
            .filter(|f| tag.is_none_or(|t| f.tags.iter().any(|x| x == t)))
            .collect();
        all.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
        let total = all.len();
        let page = all.into_iter().skip(offset).take(limit).cloned().collect();
        (page, total)
    }

    /// Distinct tags across live memories with usage counts, most used
    /// first (the console's filter source).
    pub async fn tags(&self) -> Vec<(String, usize)> {
        let inner = self.inner.read().await;
        let mut counts: HashMap<String, usize> = HashMap::new();
        for m in inner.memories.values().filter(|f| f.is_live()) {
            for tag in &m.tags {
                *counts.entry(tag.clone()).or_default() += 1;
            }
        }
        let mut out: Vec<(String, usize)> = counts.into_iter().collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        out
    }

    /// Live memories tagged with any of `entities`, exactly.
    pub async fn tagged(&self, entities: &HashSet<String>) -> Vec<Memory> {
        if entities.is_empty() {
            return Vec::new();
        }
        let inner = self.inner.read().await;
        inner
            .memories
            .values()
            .filter(|m| m.is_live() && m.entities.iter().any(|e| entities.contains(e)))
            .cloned()
            .collect()
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
    /// phrased third-person about the user; the injection hook counts that
    /// `user` as a match only in a question of pronouns alone ("who am I"),
    /// or with `MEMORY_VERBS` ("what do you know about me").
    ///
    /// `embedding_model` is the model the provider REPORTED for `query_vec`;
    /// vectors are tagged the same way, so ones from any other model (an
    /// earlier provider default, a previously pinned model) live in an
    /// incomparable space and are skipped — and backfill re-embeds them.
    /// Empty (no query vector, the ops probe) accepts any.
    pub async fn recall(
        &self,
        query: &str,
        query_vec: Option<&[f32]>,
        embedding_model: &str,
        limit: usize,
        half_life_days: u64,
        include_superseded: bool,
    ) -> Vec<(Memory, f32)> {
        self.recall_scaled(
            query,
            query_vec,
            embedding_model,
            limit,
            half_life_days,
            include_superseded,
            0.0,
        )
        .await
    }

    /// The best raw BM25 any live memory `keep` accepts scores for `query`
    /// (only what the caller can rank sets the scale): what a caller
    /// merging several banks into one ranking passes to
    /// [`Bank::recall_scaled`] (the maximum over all of them), so every bank's
    /// lexical arm divides by the same number.
    pub async fn bm25_max(&self, query: &str, keep: impl Fn(&Memory) -> bool) -> f32 {
        let inner = self.inner.read().await;
        inner
            .index
            .score(&ranking_tokens(query))
            .into_iter()
            .filter(|(id, _)| {
                inner
                    .memories
                    .get(id)
                    .is_some_and(|m| m.is_live() && keep(m))
            })
            .map(|(_, s)| s)
            .fold(0.0, f32::max)
    }

    /// [`Bank::recall`] with the lexical arm divided by `bm25_max` (this
    /// bank's own best when it is 0), so hits from banks recalled with the
    /// same `bm25_max` compare by score.
    #[allow(clippy::too_many_arguments)]
    pub async fn recall_scaled(
        &self,
        query: &str,
        query_vec: Option<&[f32]>,
        embedding_model: &str,
        limit: usize,
        half_life_days: u64,
        include_superseded: bool,
        bm25_max: f32,
    ) -> Vec<(Memory, f32)> {
        self.recall_evidence(
            query,
            query_vec,
            embedding_model,
            limit,
            half_life_days,
            include_superseded,
            bm25_max,
        )
        .await
        .0
    }

    /// [`Bank::recall_scaled`] plus, for every candidate it ranked (also
    /// those `limit` cut), the [`Evidence`] of why it matched. The ranking
    /// is the same arithmetic; the evidence only reads it.
    #[allow(clippy::too_many_arguments)]
    pub async fn recall_evidence(
        &self,
        query: &str,
        query_vec: Option<&[f32]>,
        embedding_model: &str,
        limit: usize,
        half_life_days: u64,
        include_superseded: bool,
        bm25_max: f32,
    ) -> (Vec<(Memory, f32)>, HashMap<String, Evidence>) {
        let (query_tokens, expanded) = query_tokens(query);
        if query_tokens.is_empty() && query_vec.is_none() {
            return (Vec::new(), HashMap::new());
        }
        // What a hit must share with the question to be evidence.
        let mut content: Vec<&str> = query_tokens
            .iter()
            .map(String::as_str)
            .filter(|t| !STOPWORDS.contains(t) && !(expanded && *t == "user"))
            .collect();
        // What memory holds of "me", not who is told ("tell me more") or a bare "I know".
        let words = crate::index::tokenize(query);
        let requested_slot_spans = query_projection::requested_slots(query, &words);
        let directive_scope = query_projection::directive_scope(query);
        let target_scope = if requested_slot_spans.is_some() {
            None
        } else {
            query_projection::target_scope(query)
        };
        let in_slot = |i: usize| {
            requested_slot_spans
                .as_ref()
                .is_some_and(|spans| spans.iter().any(|span| span.contains(&i)))
        };
        let has = |w: &str| words.iter().any(|t| t == w);
        // In a compound noun phrase the head is the last word before an
        // auxiliary/function word: "which code editor am I using" asks for
        // the editor, not the incidental modifier "code".
        let attributes: HashSet<&str> = words
            .iter()
            .enumerate()
            .filter(|(i, word)| {
                !in_slot(*i) && matches!(word.as_str(), "which" | "what" | "qual" | "quais" | "que")
            })
            .filter_map(|(i, _)| {
                words
                    .iter()
                    .skip(i + 1)
                    .take(3)
                    .take_while(|word| !STOPWORDS.contains(&word.as_str()))
                    .last()
            })
            .map(String::as_str)
            .collect();
        // Preserve the existing bounded WH noun-phrase head, but retain its
        // preceding constituents. A tag for the head alone cannot answer a
        // differently qualified attribute, even in a bank with no entity tags.
        let compound_attributes: Vec<(&str, Vec<&str>)> = words
            .iter()
            .enumerate()
            .filter(|(i, word)| {
                !in_slot(*i) && matches!(word.as_str(), "which" | "what" | "qual" | "quais" | "que")
            })
            .filter_map(|(i, _)| {
                let mut phrase = words
                    .iter()
                    .skip(i + 1)
                    .take(3)
                    .take_while(|word| !STOPWORDS.contains(&word.as_str()))
                    .map(String::as_str)
                    .collect::<Vec<_>>();
                let mut head = *phrase.last()?;
                let mut seam = i + 1 + phrase.len();
                // A single head may take one explicit genitive complement.
                // Certify only head + linker + nominal token + the existing
                // clause seam below; fragments/longer phrases stay unknown.
                let english = matches!(words[i].as_str(), "which" | "what");
                let linker = words.get(seam)?.as_str();
                let genitive = if english {
                    linker == "of"
                } else {
                    matches!(linker, "de" | "do" | "da" | "dos" | "das")
                };
                if phrase.len() == 1 && genitive {
                    let mut complement_at = seam + 1;
                    let article = words.get(complement_at)?.as_str();
                    if (english && matches!(article, "the" | "a" | "an"))
                        || (!english
                            && linker == "de"
                            && matches!(article, "o" | "a" | "os" | "as"))
                    {
                        complement_at += 1;
                    }
                    let complement = words.get(complement_at)?.as_str();
                    if STOPWORDS.contains(&complement) {
                        return None;
                    }
                    head = phrase[0];
                    phrase.push(complement);
                    seam = complement_at + 1;
                }
                // A preposition or the lookahead limit cannot certify a noun
                // phrase: WH may instead be the subject of a verbal clause.
                // Only an explicit inverted auxiliary + subject (EN), or an
                // overt subject opening (PT), certifies this bounded shape.
                let boundary = words.get(seam)?.as_str();
                let next = words.get(seam + 1)?.as_str();
                let certified = match words[i].as_str() {
                    "which" | "what" => {
                        matches!(
                            boundary,
                            "do" | "does" | "did" | "am" | "is" | "are" | "was" | "were"
                        ) && matches!(
                            next,
                            "i" | "we"
                                | "you"
                                | "he"
                                | "she"
                                | "it"
                                | "they"
                                | "my"
                                | "our"
                                | "your"
                                | "his"
                                | "her"
                                | "their"
                                | "the"
                                | "a"
                                | "an"
                        )
                    }
                    "qual" | "quais" | "que" => {
                        matches!(
                            boundary,
                            "eu" | "nós"
                                | "ele"
                                | "ela"
                                | "você"
                                | "vocês"
                                | "meu"
                                | "minha"
                                | "meus"
                                | "minhas"
                                | "o"
                                | "a"
                                | "os"
                                | "as"
                        ) && !STOPWORDS.contains(&next)
                    }
                    _ => false,
                };
                (phrase.len() > 1 && certified).then_some((head, phrase))
            })
            .collect();
        let personal_scopes = if requested_slot_spans.is_some() {
            Vec::new()
        } else {
            query_projection::personal_scopes(query)
        };
        // Possessives expose only a nominal head before a grammatical seam.
        // Never harvest a fixed window across a subordinate clause or verb.
        let possessed: HashSet<&str> = words
            .iter()
            .enumerate()
            .filter(|(i, word)| !in_slot(*i) && query_projection::possessive(word))
            .filter_map(|(i, _)| {
                let run = words
                    .iter()
                    .skip(i + 1)
                    .take_while(|word| {
                        !STOPWORDS.contains(&word.as_str()) && !query_projection::subordinate(word)
                    })
                    .collect::<Vec<_>>();
                if run.is_empty()
                    || words
                        .get(i + 1 + run.len())
                        .is_some_and(|w| query_projection::subordinate(w))
                {
                    return None;
                }
                run.last().copied().map(String::as_str)
            })
            .collect();
        let verb = content.iter().any(|t| MEMORY_VERBS.contains(t));
        // "tell" asks it only beside a memory verb or "myself" ("tell me about myself").
        let tell = verb || has("myself");
        let of_me = content
            .iter()
            .all(|t| MEMORY_VERBS.contains(t) || *t == "anything" || (*t == "tell" && tell))
            // Or asked of the user addressed as "I": "do you know who I am?".
            && (has("me") || has("myself") || has("mim") || (has("you") && verb && has("i")) || (has("você") && verb && has("eu")));
        let identity =
            (has("who") && has("am") && has("i")) || (has("quem") && has("sou") && has("eu"));
        if expanded && ((content.is_empty() && identity) || of_me) {
            // A question of pronouns alone, or of what memory holds of them, is about the user.
            content = vec!["user"];
        }
        if !of_me && !identity {
            content.retain(|t| !MEMORY_VERBS.contains(t));
        }
        // An occurrence outside the slot remains relational even when the
        // same word also appears in the requested constituent.
        let relation_content: Vec<&str> = content
            .iter()
            .copied()
            .filter(|term| {
                requested_slot_spans.is_none()
                    || words
                        .iter()
                        .enumerate()
                        .any(|(i, word)| !in_slot(i) && word.as_str() == *term)
            })
            .collect();
        let slot_content: Vec<&str> = content
            .iter()
            .copied()
            .filter(|term| {
                words
                    .iter()
                    .enumerate()
                    .any(|(i, word)| in_slot(i) && word.as_str() == *term)
            })
            .collect();
        let inner = self.inner.read().await;
        let rank_tokens = ranking_tokens(query);
        let mut bm25 = inner.index.score(&rank_tokens);
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
        let (floor, anchor, weight) = semantic_scale(embedding_model);
        let mut sem: HashMap<String, f32> = HashMap::new();
        // The query's cosine over every same-model vector: what a single
        // cosine stands out from.
        let (mut pool, mut sum, mut sum_sq) = (0usize, 0f64, 0f64);
        if let Some(qv) = query_vec {
            for (id, (model, v)) in &inner.vectors {
                if !embedding_model.is_empty() && model != embedding_model {
                    continue;
                }
                let c = cosine(qv, v);
                pool += 1;
                sum += f64::from(c);
                sum_sq += f64::from(c) * f64::from(c);
                if c > floor {
                    sem.insert(id.clone(), c);
                    bm25.entry(id.clone()).or_insert(0.0);
                }
            }
        }
        let candidates: Vec<(Memory, f32, Option<f32>)> = bm25
            .into_iter()
            .filter_map(|(id, s)| {
                let f = inner.memories.get(&id)?;
                (include_superseded
                    || (f.is_live() && !inner.memories.values().any(|new| f.revalidated_by(new))))
                .then(|| (f.clone(), s, sem.get(&id).copied()))
            })
            .collect();
        // BM25 over the caller's shared maximum (computed over what it can
        // rank), else this bank's best candidate; cosine on an absolute
        // scale, so a weak best match stays weak.
        let max_bm25 = if bm25_max > 0.0 {
            bm25_max
        } else {
            candidates.iter().map(|c| c.1).fold(f32::EPSILON, f32::max)
        };
        // Population mean and sd of the query's cosine over the pool; a
        // z-score needs a pool big enough to mean anything and some spread.
        let mean = sum / pool.max(1) as f64;
        let sd = (sum_sq / pool.max(1) as f64 - mean * mean).max(0.0).sqrt();
        let z_of =
            |c: f32| (pool >= MIN_Z_POOL && sd > 0.0).then(|| ((f64::from(c) - mean) / sd) as f32);
        let now = now_ms();
        let named_entities: HashSet<String> = inner
            .memories
            .values()
            .flat_map(|m| &m.entities)
            .filter(|e| e.as_str() != crate::scope::LAYER_ENTITY)
            .filter(|e| {
                let terms = crate::index::tokenize(e);
                !terms.is_empty() && terms.iter().all(|t| content.contains(&t.as_str()))
            })
            .cloned()
            .collect();
        let nominal_possessions: Vec<(std::ops::Range<usize>, Vec<String>)> = words
            .iter()
            .enumerate()
            .filter(|(i, w)| !in_slot(*i) && query_projection::possessive(w))
            .filter_map(|(i, _)| {
                let run = words
                    .iter()
                    .skip(i + 1)
                    .take_while(|word| {
                        !STOPWORDS.contains(&word.as_str()) && !query_projection::subordinate(word)
                    })
                    .collect::<Vec<_>>();
                let head = run.last()?.as_str();
                let end = i + 1 + run.len();
                if words
                    .get(end)
                    .is_some_and(|w| query_projection::subordinate(w))
                {
                    return None;
                }
                let attested = inner
                    .memories
                    .values()
                    .filter(|m| m.is_live())
                    .any(|m| m.tags.iter().any(|tag| tag == head));
                attested.then(|| (i..end, run.iter().map(|word| word.to_string()).collect()))
            })
            .collect();
        let requested_tags: HashSet<&str> = inner
            .memories
            .values()
            .filter(|m| m.is_live())
            .flat_map(|m| &m.tags)
            .map(String::as_str)
            .filter(|tag| attributes.contains(tag) || possessed.contains(tag))
            .collect();
        let mut evidence = HashMap::with_capacity(candidates.len());
        let mut scored: Vec<(Memory, f32)> = Vec::with_capacity(candidates.len());
        for (f, s, cos) in candidates {
            // Semantic joins lexical BEFORE fusion so recency decays
            // both alike.
            let sem = ((cos.unwrap_or(0.0) - anchor) / (1.0 - anchor)).max(0.0);
            let relevance = s / max_bm25 + weight * sem;
            let fused = crate::index::fused_score(relevance, &f, &rank_tokens, now, half_life_days);
            let compound_answer = compound_attributes
                .iter()
                .filter(|(head, _)| {
                    requested_tags.contains(head) || inner.index.has_inflection(head, &f.id)
                })
                .all(|(_, phrase)| {
                    phrase.iter().all(|term| {
                        f.tags.iter().any(|tag| tag.as_str() == *term)
                            || inner.index.has_inflection(term, &f.id)
                    })
                });
            let nominal_match = !nominal_possessions.is_empty()
                && nominal_possessions.iter().all(|(_, terms)| {
                    terms.last().is_some_and(|head| {
                        let scope = query_projection::PersonalScope {
                            span: 0..0,
                            terms: vec![head.clone()],
                            predicate: None,
                            owner: None,
                            self_owned: true,
                            open_argument: false,
                            portuguese_first_person: false,
                        };
                        query_projection::scope_matches(&scope, &f.text)
                    })
                });
            let scoped_personal = personal_scopes
                .iter()
                .all(|scope| query_projection::scope_matches(scope, &f.text));
            let directive = directive_scope
                .as_ref()
                .map(|scope| query_projection::directive_evidence(scope, &f.text));
            let target_obligation = target_scope
                .as_ref()
                .and_then(|scope| query_projection::obligation_evidence(scope, &f.text));
            let target_match = target_scope.as_ref().is_none_or(|scope| {
                target_obligation.as_ref().map_or_else(
                    || query_projection::target_matches(scope, &f.text),
                    |obligation| obligation.matched,
                )
            });
            let attribute_answer = scoped_personal
                && compound_answer
                && (f
                    .tags
                    .iter()
                    .any(|tag| requested_tags.contains(tag.as_str()))
                    || (f.entities.iter().any(|e| named_entities.contains(e))
                        && requested_tags
                            .iter()
                            .any(|tag| inner.index.has_inflection(tag, &f.id))));
            evidence.insert(
                f.id.clone(),
                Evidence {
                    matched: content
                        .iter()
                        .filter(|t| inner.index.has_inflection(t, &f.id))
                        .map(|t| t.to_string())
                        .collect(),
                    relation_matched: relation_content
                        .iter()
                        .filter(|t| inner.index.has_inflection(t, &f.id))
                        .map(|t| t.to_string())
                        .collect(),
                    requested_slot_matched: slot_content
                        .iter()
                        .filter(|t| inner.index.has_inflection(t, &f.id))
                        .map(|t| t.to_string())
                        .collect(),
                    relation_terms: relation_content.len(),
                    requested_slot_spans: requested_slot_spans.clone(),
                    terms: content.len(),
                    of_me: expanded && of_me,
                    named_subject: f
                        .entities
                        .iter()
                        .filter(|e| e.as_str() != crate::scope::LAYER_ENTITY)
                        .any(|e| {
                            let terms = crate::index::tokenize(e);
                            e.chars().count() >= 3
                                && !terms.is_empty()
                                && terms.iter().all(|t| content.contains(&t.as_str()))
                        }),
                    subject_question: f
                        .entities
                        .iter()
                        .filter(|e| e.as_str() != crate::scope::LAYER_ENTITY)
                        .any(|e| {
                            let terms = crate::index::tokenize(e);
                            !terms.is_empty()
                                && terms.iter().all(|t| content.contains(&t.as_str()))
                                && (words.windows(2).any(|w| {
                                    matches!(w[0].as_str(), "is" | "é") && w[1] == terms[0]
                                }) || (query.contains('?')
                                    && words.iter().any(|w| {
                                        matches!(w.as_str(), "i" | "eu" | "my" | "meu" | "minha")
                                    })))
                        }),
                    subject_conflict: !named_entities.is_empty()
                        && !f.entities.is_empty()
                        && !f.entities.iter().any(|e| named_entities.contains(e)),
                    // Argument/owner absence is lack of a certificate, not
                    // semantic proof of a conflicting predicate.
                    predicate_conflict: false,
                    specific_recall: false,
                    personal_match: scoped_personal
                        && (nominal_match
                            || personal_scopes.iter().any(|scope| {
                                scope.predicate.is_some()
                                    || scope.owner.is_some()
                                    || scope.self_owned
                            })),
                    personal_scope_spans: personal_scopes
                        .iter()
                        .map(|scope| [scope.span.start, scope.span.end])
                        .chain(
                            nominal_possessions
                                .iter()
                                .map(|(span, _)| [span.start, span.end]),
                        )
                        .collect(),
                    personal_scope_terms: personal_scopes
                        .iter()
                        .map(|scope| scope.terms.clone())
                        .chain(nominal_possessions.iter().map(|(_, terms)| terms.clone()))
                        .collect(),
                    personal_scope_predicates: personal_scopes
                        .iter()
                        .filter_map(|scope| scope.predicate.clone())
                        .collect(),
                    personal_scope_owners: personal_scopes
                        .iter()
                        .filter_map(|scope| scope.owner.clone())
                        .collect(),
                    personal_scope_matched: !personal_scopes.is_empty() && scoped_personal,
                    personal_scope_required: !personal_scopes.is_empty(),
                    target_scope_required: target_scope.is_some(),
                    target_scope_matched: target_scope.is_some() && target_match,
                    target_scope_span: target_scope
                        .as_ref()
                        .map(|scope| [scope.span.start, scope.span.end]),
                    target_scope_kind: target_scope
                        .as_ref()
                        .map(|scope| format!("{:?}", scope.kind)),
                    target_scope_terms: target_scope
                        .as_ref()
                        .map(|scope| scope.terms.clone())
                        .unwrap_or_default(),
                    target_scope_predicate: target_scope
                        .as_ref()
                        .and_then(|scope| scope.predicate.clone()),
                    target_scope_subject: target_scope
                        .as_ref()
                        .map(|scope| scope.subject.clone())
                        .unwrap_or_default(),
                    target_scope_object: target_scope
                        .as_ref()
                        .map(|scope| scope.object.clone())
                        .unwrap_or_default(),
                    target_scope_relative: target_scope
                        .as_ref()
                        .map(|scope| scope.relative.clone())
                        .unwrap_or_default(),
                    target_obligation: target_obligation
                        .map(|obligation| serde_json::json!(obligation)),
                    directive_scope_required: directive_scope.is_some(),
                    directive_scope_matched: directive.as_ref().is_some_and(|audit| audit.matched),
                    target_directive: directive.map(|audit| serde_json::json!(audit)),
                    attribute_match: attribute_answer,
                    attribute_conflict: !compound_answer
                        || (!requested_tags.is_empty() && !attribute_answer),
                    detail_match: {
                        let entities: HashSet<String> = f
                            .entities
                            .iter()
                            .flat_map(|e| crate::index::tokenize(e))
                            .collect();
                        content
                            .iter()
                            .any(|t| !entities.contains(*t) && inner.index.has_inflection(t, &f.id))
                    },
                    relation_detail_match: {
                        let entities: HashSet<String> = f
                            .entities
                            .iter()
                            .flat_map(|e| crate::index::tokenize(e))
                            .collect();
                        relation_content
                            .iter()
                            .any(|t| !entities.contains(*t) && inner.index.has_inflection(t, &f.id))
                    },
                    lexical: s / max_bm25,
                    relevance,
                    cosine: cos,
                    z: cos.and_then(z_of),
                    vector: query_vec.is_some()
                        && inner.vectors.get(&f.id).is_some_and(|(model, _)| {
                            embedding_model.is_empty() || model == embedding_model
                        }),
                },
            );
            scored.push((f, fused));
        }
        rank_desc(&mut scored, limit);
        (scored, evidence)
    }

    /// The raw lexical arm: plain BM25 order (entity handles are part of
    /// the indexed text) with no recency, pin, or corroboration fusion — an
    /// old exact twin must not sink below fresh weak matches. Consolidation
    /// interleaves this with [`Bank::semantic`], because a fused rank buries
    /// the twin.
    pub async fn lexical(
        &self,
        query: &str,
        limit: usize,
        include_superseded: bool,
    ) -> Vec<(Memory, f32)> {
        let tokens = ranking_tokens(query);
        let inner = self.inner.read().await;
        let mut bm25 = inner.index.score(&tokens);
        // The index only holds live records; history seeds non-live ones.
        if include_superseded {
            for (id, m) in &inner.memories {
                if !m.is_live() {
                    bm25.entry(id.clone()).or_insert(0.0);
                }
            }
        }
        let mut scored: Vec<(Memory, f32)> = bm25
            .into_iter()
            .filter_map(|(id, s)| inner.memories.get(&id).map(|m| (m.clone(), s)))
            .filter(|(m, _)| include_superseded || m.is_live())
            .collect();
        rank_desc(&mut scored, limit);
        scored
    }

    /// The raw semantic arm: cosine against the vector sidecar, above
    /// `SEMANTIC_FLOOR`, same model-acceptance rule as [`Bank::recall`].
    /// Scores are the cosine itself, so callers can reason about them.
    pub async fn semantic(
        &self,
        query_vec: &[f32],
        embedding_model: &str,
        limit: usize,
        include_superseded: bool,
    ) -> Vec<(Memory, f32)> {
        let inner = self.inner.read().await;
        let mut scored: Vec<(Memory, f32)> = inner
            .vectors
            .iter()
            .filter(|(_, (model, _))| embedding_model.is_empty() || model == embedding_model)
            .filter_map(|(id, (_, v))| {
                let c = cosine(query_vec, v);
                let m = inner.memories.get(id)?;
                (c > SEMANTIC_FLOOR && (include_superseded || m.is_live())).then(|| (m.clone(), c))
            })
            .collect();
        rank_desc(&mut scored, limit);
        scored
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

    /// Atomic write ([`write_atomic`]); empty content removes the rule.
    pub fn set_rule(&self, name: &str, content: &str) -> Result<(), MemoryError> {
        validate_name(name)?;
        // Rules are injected into every turn: no known secret shape is kept.
        let redacted = crate::redact::redact(content);
        if !redacted.labels.is_empty() {
            tracing::warn!(bank = %self.name, rule = name, labels = ?redacted.labels,
                "redacted secrets from a rule");
        }
        let content = redacted.text.as_str();
        let dir = self.rules_dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{name}.md"));
        if content.trim().is_empty() {
            if path.exists() {
                std::fs::remove_file(&path)?;
            }
            return Ok(());
        }
        write_atomic(&path, content.as_bytes())?;
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
            tags: vec![],
            confidence: Confidence::Stated,
            corroboration: 0,
            pinned: false,
            source: None,
            created_at: now_ms(),
            updated_at: now_ms(),
            invalid_at: None,
            superseded_by: None,
            revision: 0,
            stated_at: None,
        }
    }

    #[tokio::test]
    async fn open_compacts_the_sidecar_once_dead_lines_outnumber_live() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let (mut a, mut b, c) = (memory("alpha"), memory("bravo"), memory("charlie"));
        for (m, v) in [(&a, 0.1), (&b, 0.2), (&c, 0.3)] {
            bank.commit(m.clone()).await.unwrap();
            bank.set_vector(&m.id, "m", &m.text, vec![v, 1.0])
                .await
                .unwrap();
        }
        let path = bank.dir().join(VECTORS_FILE);
        let c_line = std::fs::read_to_string(&path)
            .unwrap()
            .lines()
            .find(|l| l.contains(&c.id))
            .unwrap()
            .to_string();
        // a's text edited, b retired: both lines dead; c's stays live.
        a.text = "alpha edited".into();
        a.revision += 1;
        bank.commit(a.clone()).await.unwrap();
        b.invalid_at = Some(now_ms());
        b.revision += 1;
        bank.commit(b.clone()).await.unwrap();
        // A crash left a torn temp file behind.
        std::fs::write(path.with_extension("jsonl.tmp"), b"{\"torn").unwrap();

        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        assert_eq!(bank2.vector_count().await, 1);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{c_line}\n")
        );
        assert!(!path.with_extension("jsonl.tmp").exists());

        // The re-embed appends; at 2 lines for 2 kept, nothing is rewritten.
        bank2
            .set_vector(&a.id, "m", &a.text, vec![0.1, 1.0])
            .await
            .unwrap();
        let store3 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank3 = store3.bank("main").await.unwrap();
        assert_eq!(bank3.vector_count().await, 2);
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 2);
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn a_failed_compaction_removes_its_temp_and_still_opens() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let m = memory("alpha");
        bank.commit(m.clone()).await.unwrap();
        // Three re-embeds of one text: 1 live vector, 2 dead lines.
        for _ in 0..3 {
            bank.set_vector(&m.id, "m", &m.text, vec![0.1; 64])
                .await
                .unwrap();
        }
        let path = bank.dir().join(VECTORS_FILE);
        let before = std::fs::read_to_string(&path).unwrap();
        let tmp = path.with_extension("jsonl.tmp");
        // The compaction's writes land on /dev/full: ENOSPC mid-rewrite.
        std::os::unix::fs::symlink("/dev/full", &tmp).unwrap();

        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = reopened.bank("main").await.unwrap();
        assert_eq!(bank2.vector_count().await, 1);
        assert!(std::fs::symlink_metadata(&tmp).is_err(), "temp removed");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), before, "untouched");
    }

    #[tokio::test]
    async fn no_known_secret_reaches_memories_jsonl_or_a_rule_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        // Assembled at runtime: no literal key shape in the source.
        let key = format!("ghp_{}", &"a1B2c3D4e5".repeat(4)[..36]);
        let key = key.as_str();
        // Built by a path that skipped the entry-point redaction.
        let mut m = memory(&format!("User's deploy token is {key}"));
        m.entities = vec!["AKIAIOSFODNN7EXAMPLE".into()];
        m.tags = vec![format!("{}://ops:S3cr3tPass@db/app", "postgres")];
        bank.commit(m.clone()).await.unwrap();
        bank.set_rule("deploy", &format!("Use the key {key} for CI."))
            .unwrap();

        let on_disk = std::fs::read_to_string(bank.dir().join(MEMORIES_FILE)).unwrap();
        for secret in [key, "AKIAIOSFODNN7EXAMPLE", "S3cr3tPass"] {
            assert!(!on_disk.contains(secret), "{secret} reached memories.jsonl");
        }
        let stored = bank.get(&m.id).await.unwrap();
        assert_eq!(
            stored.text,
            "User's deploy token is [REDACTED:github_token]"
        );
        assert!(stored.entities.is_empty(), "a secret entity is dropped");
        assert!(stored.tags.is_empty(), "a secret tag is dropped");
        let rule = std::fs::read_to_string(bank.dir().join("rules/deploy.md")).unwrap();
        assert_eq!(rule, "Use the key [REDACTED:github_token] for CI.");
    }

    #[tokio::test]
    async fn retired_records_hold_no_vector_in_ram_or_on_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let m = memory("beta to retire");
        bank.commit(m.clone()).await.unwrap();
        bank.set_vector(&m.id, "m", &m.text, vec![0.5])
            .await
            .unwrap();
        let mut gone = m.clone();
        gone.invalid_at = Some(now_ms());
        gone.revision += 1;
        bank.commit(gone.clone()).await.unwrap();
        assert_eq!(bank.vector_count().await, 0, "dropped in RAM");
        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = reopened.bank("main").await.unwrap();
        assert_eq!(bank2.vector_count().await, 0, "and on reopen");

        // A vector landing after the retirement is refused.
        assert!(!bank
            .set_vector(&m.id, "m", &m.text, vec![0.5])
            .await
            .unwrap());
        // Revived, the record is backfill work again.
        let mut back = gone;
        back.invalid_at = None;
        back.revision += 1;
        bank.commit(back).await.unwrap();
        assert_eq!(bank.unembedded(10, "m").await.len(), 1);
    }

    #[test]
    fn ranking_remains_legacy_while_evidence_normalizes_portuguese() {
        assert_eq!(
            ranking_tokens("qual é o nome da minha gata?"),
            ["qual", "é", "o", "nome", "da", "minha", "gata"]
        );
        assert_eq!(
            query_tokens("qual é o nome da minha gata?").0,
            ["nome", "gata", "user"]
        );
        assert_eq!(ranking_tokens("which OS do I use"), ["os", "use", "user"]);
    }

    #[test]
    fn query_tokens_drop_stopwords_but_never_empty_the_query() {
        assert_eq!(
            query_tokens("what is my dog's name").0,
            ["dog", "name", "user"]
        );
        // `.1` says the pronoun added `user`; a typed one is no expansion.
        let (tokens, expanded) = query_tokens("which OS do I use");
        assert_eq!(tokens, ["os", "use", "user"]);
        // Preserve ambiguous technical initialisms (OS/US) even though
        // Portuguese uses `os` as an article. Never lose explicit OS recall.
        assert!(expanded);
        let (tokens, expanded) = query_tokens("does the user live in the US");
        assert_eq!(tokens, ["user", "live", "us"]);
        assert!(!expanded);
        // An all-stopword query keeps its tokens rather than matching nothing.
        assert_eq!(query_tokens("what is it").0, ["what", "is", "it"]);
        // A pasted term counts once.
        assert_eq!(
            query_tokens("error: timeout\nerror: timeout\nwhy my error").0,
            ["error", "timeout", "user"]
        );
    }

    #[test]
    fn a_long_query_keeps_its_head_and_tail() {
        assert_eq!(
            cap_query("which db does ledgerly use"),
            "which db does ledgerly use"
        );
        let paste = format!("why does this fail? {} end of the log é", "é".repeat(2_000));
        let capped = cap_query(&paste);
        assert!(capped.len() <= QUERY_MAX_BYTES + 1, "{}", capped.len());
        assert!(capped.starts_with("why does this fail? "));
        assert!(capped.ends_with(" end of the log é"));
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
        let hits = bank2.recall("formal writing", None, "", 5, 30, false).await;
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
        assert!(bank
            .recall("api port", None, "", 5, 30, false)
            .await
            .is_empty());
        assert!(bank.get(&f.id).await.unwrap().invalid_at.is_some());

        // And the same is true after a reopen.
        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        assert!(bank2
            .recall("api port", None, "", 5, 30, false)
            .await
            .is_empty());
        let (memories, total) = bank2.list(10, 0, true, None).await;
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
        let (_, total) = bank2.list(10, 0, false, None).await;
        assert_eq!(total, 2);
    }

    #[tokio::test]
    async fn a_last_line_without_its_newline_is_not_fused_with_the_next() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mut gone = memory("deleted by the user");
        bank.commit(gone.clone()).await.unwrap();
        bank.commit(memory("kept by hand")).await.unwrap();
        // An editor saved the file without its final newline.
        let path = bank.dir().join(MEMORIES_FILE);
        let raw = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, raw.trim_end()).unwrap();
        gone.invalid_at = Some(now_ms());
        gone.revision += 1;
        bank.commit(gone).await.unwrap();

        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        let (live, _) = bank2.list(10, 0, false, None).await;
        let texts: Vec<&str> = live.iter().map(|m| m.text.as_str()).collect();
        assert_eq!(texts, ["kept by hand"], "the tombstone held, the tail kept");
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
    async fn a_trashed_bank_reads_as_deleted_until_recreated() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        store.ensure_bank("blog", None).await.unwrap();
        store.ensure_bank("blog-old", None).await.unwrap();
        store.trash_bank("blog-old").await.unwrap();
        assert!(store.is_deleted("blog-old"));
        assert!(
            !store.is_deleted("blog"),
            "another bank's trash is not this one's"
        );
        assert!(
            !store.is_deleted("never"),
            "a bank that never existed is not deleted"
        );
        store.trash_bank("blog").await.unwrap();
        assert!(store.is_deleted("blog"));
        store.ensure_bank("blog", None).await.unwrap();
        assert!(
            !store.is_deleted("blog"),
            "an explicit re-create revives it"
        );
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

    #[tokio::test]
    async fn editing_text_invalidates_the_stored_vector() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mut m = memory("the port is 3000");
        bank.commit(m.clone()).await.unwrap();
        bank.set_vector(&m.id, "test-model", &m.text, vec![0.1, 0.2])
            .await
            .unwrap();
        assert_eq!(bank.vector_count().await, 1);
        assert!(bank.unembedded(10, "").await.is_empty());

        // Text edit: the vector embeds the OLD text — must re-enter backfill.
        m.text = "the port is 4000".into();
        m.revision += 1;
        bank.commit(m.clone()).await.unwrap();
        assert_eq!(bank.vector_count().await, 0);
        assert_eq!(bank.unembedded(10, "").await.len(), 1);
    }

    #[tokio::test]
    async fn reopen_keeps_vectors_until_the_text_changes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mut m = memory("stable memory");
        bank.commit(m.clone()).await.unwrap();
        bank.set_vector(&m.id, "test-model", &m.text, vec![0.5])
            .await
            .unwrap();
        // Reinforce / pin / tag: revision moves, text does not.
        m.revision += 1;
        m.pinned = true;
        bank.commit(m.clone()).await.unwrap();
        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        assert_eq!(reopened.bank("main").await.unwrap().vector_count().await, 1);

        // A text edit (embed not yet landed) makes the stored vector stale.
        m.text = "edited memory".into();
        m.revision += 1;
        bank.commit(m.clone()).await.unwrap();
        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = reopened.bank("main").await.unwrap();
        assert_eq!(bank2.vector_count().await, 0);
        assert_eq!(bank2.unembedded(10, "").await.len(), 1);
    }

    #[tokio::test]
    async fn legacy_revision_stamped_vectors_are_dropped_for_re_embed() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let m = memory("legacy memory");
        bank.commit(m.clone()).await.unwrap();
        std::fs::write(
            bank.dir().join(VECTORS_FILE),
            format!(
                "{{\"id\":\"{}\",\"model\":\"\",\"revision\":0,\"v\":[0.5]}}\n",
                m.id
            ),
        )
        .unwrap();
        let reopened = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = reopened.bank("main").await.unwrap();
        assert_eq!(bank2.vector_count().await, 0);
        assert_eq!(bank2.unembedded(10, "").await.len(), 1);
    }

    #[tokio::test]
    async fn unembedded_treats_a_model_mismatch_as_missing() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let m = memory("model tracked");
        bank.commit(m.clone()).await.unwrap();
        bank.set_vector(&m.id, "old-model", &m.text, vec![0.5])
            .await
            .unwrap();

        assert!(bank.unembedded(10, "old-model").await.is_empty());
        assert_eq!(bank.unembedded(10, "new-model").await.len(), 1);
        // Empty active model = provider default: whatever exists counts.
        assert!(bank.unembedded(10, "").await.is_empty());
    }

    #[tokio::test]
    async fn semantic_relevance_decays_with_recency_like_lexical() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let fresh = memory("alpha beta");
        let mut old = memory("gamma delta");
        old.updated_at = now_ms() - 365 * 86_400_000;
        bank.commit(fresh.clone()).await.unwrap();
        bank.commit(old.clone()).await.unwrap();
        for m in [&fresh, &old] {
            bank.set_vector(&m.id, "m", &m.text, vec![1.0, 0.0])
                .await
                .unwrap();
        }
        // No lexical overlap: the identical semantic match is the only
        // relevance, so recency alone must separate the two.
        let hits = bank
            .recall("zzz", Some(&[1.0, 0.0]), "m", 5, 30, false)
            .await;
        let score = |id: &str| {
            hits.iter()
                .find(|(m, _)| m.id == id)
                .map(|(_, s)| *s)
                .expect("both memories are semantic hits")
        };
        assert!(score(&fresh.id) > score(&old.id), "{hits:?}");
    }

    #[tokio::test]
    async fn lexical_is_raw_bm25_unaffected_by_age() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        // The old exact twin must lead the raw lexical arm even though the
        // fused recall would decay it by the recency factor.
        let mut old = memory("kiwi mango smoothie recipe");
        old.updated_at = now_ms() - 400 * 86_400_000;
        bank.commit(old.clone()).await.unwrap();
        for w in ["one", "two", "three", "four"] {
            bank.commit(memory(&format!("kiwi {w} extra filler words")))
                .await
                .unwrap();
        }
        let hits = bank.lexical("kiwi mango smoothie recipe", 3, false).await;
        assert_eq!(hits[0].0.id, old.id, "{hits:?}");
        assert!(hits.len() <= 3);
        // The SCORE is raw BM25: fusion would scale the 400-day-old twin by
        // the recency factor (x0.8), so a fused lexical arm fails this.
        let fused = bank
            .recall("kiwi mango smoothie recipe", None, "", 3, 30, false)
            .await;
        let fused_old = fused.iter().find(|(m, _)| m.id == old.id).unwrap().1;
        assert!(
            hits[0].1 > 1.2 * fused_old,
            "raw BM25, not recency-fused: {hits:?} vs {fused:?}"
        );
        // Tombstoned records stay out unless history is asked for.
        let mut gone = old.clone();
        gone.invalid_at = Some(now_ms());
        gone.revision += 1;
        bank.commit(gone).await.unwrap();
        let live = bank.lexical("kiwi mango smoothie recipe", 10, false).await;
        assert!(live.iter().all(|(m, _)| m.id != old.id));
        let history = bank.lexical("kiwi mango smoothie recipe", 10, true).await;
        assert!(history.iter().any(|(m, _)| m.id == old.id));
    }

    fn user_memory(text: &str) -> Memory {
        let mut m = memory(text);
        m.entities = vec!["user".into()];
        m
    }

    async fn evidence_for(
        bank: &Bank,
        query: &str,
    ) -> (Vec<(Memory, f32)>, HashMap<String, Evidence>) {
        bank.recall_evidence(query, None, "", 10, 30, false, 0.0)
            .await
    }

    #[tokio::test]
    async fn slot_projection_preserves_ranking_and_repeated_relational_occurrences() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("projection", None).await.unwrap();
        let fact = memory("The cycle draft is archived.");
        bank.commit(fact.clone()).await.unwrap();
        let query = "During what cycle do I inspect the cycle draft?";
        let (hits, evidence) = evidence_for(&bank, query).await;
        let e = &evidence[&fact.id];
        assert_eq!(e.relation_matched, ["cycle", "draft"]);
        // Includes the unmatched `inspect`, counts repeated `cycle` once.
        assert_eq!(e.relation_terms, 3);
        assert_eq!(e.relation_term_count(), 3);
        assert_eq!(e.requested_slot_matched, ["cycle"]);
        assert_eq!(e.requested_slot_spans.as_ref().unwrap()[0], 0..3);
        let plain = bank
            .recall_scaled(query, None, "", 10, 30, false, 0.0)
            .await;
        assert_eq!(
            hits.iter().map(|(m, s)| (&m.id, *s)).collect::<Vec<_>>(),
            plain.iter().map(|(m, s)| (&m.id, *s)).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn the_pronoun_user_is_not_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        for t in [
            "The user lives in Recife",
            "The user prefers dark themes",
            "The user drinks green tea",
        ] {
            bank.commit(user_memory(t)).await.unwrap();
        }
        bank.commit(memory("Ledgerly stores invoices in Postgres"))
            .await
            .unwrap();

        let query = "can you help me refactor my parser";
        let (hits, evidence) = evidence_for(&bank, query).await;
        assert_eq!(hits.len(), 3, "the three user memories: {hits:?}");
        for (m, _) in &hits {
            let e = &evidence[&m.id];
            assert!(e.matched.is_empty(), "{}: {e:?}", m.text);
            assert_eq!(e.terms, 3, "help, refactor, parser");
        }
        // Ranking keeps them: only the evidence says they are coincidence.
        let plain = bank
            .recall_scaled(query, None, "", 10, 30, false, 0.0)
            .await;
        assert_eq!(plain.len(), 3);
        let ids = |h: &[(Memory, f32)]| {
            h.iter()
                .map(|(m, s)| (m.id.clone(), *s))
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(&plain), ids(&hits));
    }

    #[tokio::test]
    async fn a_pronoun_only_question_matches_the_user_handle() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mine = user_memory("The user lives in Recife");
        let also = user_memory("The user drinks green tea");
        let stray = memory("Ask who reviews the release, I think Marta");
        for m in [&mine, &also, &stray] {
            bank.commit(m.clone()).await.unwrap();
        }

        let (_, evidence) = evidence_for(&bank, "who am I").await;
        for m in [&mine, &also] {
            assert_eq!(evidence[&m.id].matched, ["user"], "{}", m.text);
            assert_eq!(evidence[&m.id].terms, 1);
        }
        // Reached through "who" and "i" alone, which are not evidence.
        assert!(
            evidence[&stray.id].matched.is_empty(),
            "{:?}",
            evidence[&stray.id]
        );
    }

    #[tokio::test]
    async fn a_question_about_what_memory_knows_of_me_is_about_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mine = user_memory("The user lives in Recife");
        let also = user_memory("The user drinks green tea");
        let stray = memory("Ask who reviews the release, I think Marta");
        for m in [&mine, &also, &stray] {
            bank.commit(m.clone()).await.unwrap();
        }

        for query in [
            "what do you remember about me",
            "what do you know about me?",
            "can you recall anything about me?",
            "tell me what you know about me",
            "tell me what you remember about me",
            "tell me about myself",
            "do you know who I am?",
        ] {
            let (_, evidence) = evidence_for(&bank, query).await;
            for m in [&mine, &also] {
                assert_eq!(evidence[&m.id].matched, ["user"], "{query}: {}", m.text);
            }
            let e = evidence.get(&stray.id).cloned().unwrap_or_default();
            assert!(e.matched.is_empty(), "{query}: {e:?}");
        }
    }

    #[tokio::test]
    async fn tell_and_bare_verbs_are_not_about_the_user() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mine = user_memory("The user lives in Recife");
        bank.commit(mine.clone()).await.unwrap();

        // "me" as who is told, or no object at all: a follow-up, not a question about the user.
        for query in [
            "tell me more",
            "tell me again",
            "can you tell me more about it?",
            "I know.",
            "you know.",
            "I remember",
        ] {
            let (_, evidence) = evidence_for(&bank, query).await;
            let e = evidence.get(&mine.id).cloned().unwrap_or_default();
            assert!(e.matched.is_empty(), "{query}: {e:?}");
        }
    }

    #[tokio::test]
    async fn selector_final_diagnoses_lexical_only_paraphrase_without_fabricating_evidence() {
        let corpus: serde_json::Value =
            serde_json::from_str(include_str!("../tests/recall_quality/golden.json")).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("lexical-diagnostic", None).await.unwrap();
        for f in corpus["memories"].as_array().unwrap() {
            let mut m = memory(f["text"].as_str().unwrap());
            m.id = f["key"].as_str().unwrap().into();
            m.entities = serde_json::from_value(f["entities"].clone()).unwrap_or_default();
            m.tags = serde_json::from_value(f["tags"].clone()).unwrap_or_default();
            bank.commit(m).await.unwrap();
        }
        for (id, required) in [
            ("q-pt-life-par-mana", "pt-life-julia-videochamada"),
            ("q-evt-next-week", "evt-palestra-meetup"),
        ] {
            let text = corpus["queries"]
                .as_array()
                .unwrap()
                .iter()
                .find(|q| q["id"] == id)
                .unwrap()["text"]
                .as_str()
                .unwrap();
            let (ranked, evidence) = bank
                .recall_evidence(text, None, "", usize::MAX, 30, false, 0.0)
                .await;
            let e = evidence.get(required);
            println!(
                "{}",
                serde_json::json!({"diagnostic":"lexical-paraphrase-capability","query_id":id,"required":required,"ranked":ranked.iter().any(|(m,_)|m.id==required),"matched":e.map(|e|&e.matched),"terms":e.map(|e|e.terms),"vector":false,"semantic_quality_measured":false})
            );
            assert!(
                e.is_none_or(|e| e.matched.is_empty()),
                "do not relabel Portuguese function words as topical evidence: {text}: {e:?}"
            );
        }
    }

    #[tokio::test]
    async fn a_personal_chatter_pronoun_does_not_create_identity_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        bank.commit(user_memory("The user lives in Recife"))
            .await
            .unwrap();
        for question in ["so what should I do now?", "what is my next step?"] {
            let (_, evidence) = evidence_for(&bank, question).await;
            assert!(
                evidence
                    .values()
                    .all(|e| !e.of_me && !e.matched.iter().any(|w| w == "user")),
                "{question}: {evidence:?}"
            );
        }
    }

    #[tokio::test]
    async fn stopwords_alone_are_never_evidence() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        bank.commit(memory("Notes on how things are going, you know"))
            .await
            .unwrap();
        bank.commit(user_memory("The user drinks green tea"))
            .await
            .unwrap();

        let (hits, evidence) = evidence_for(&bank, "how are you").await;
        assert_eq!(hits.len(), 1, "{hits:?}");
        let e = &evidence[&hits[0].0.id];
        assert!(e.matched.is_empty(), "{e:?}");
        assert_eq!(e.terms, 0);
    }

    #[tokio::test]
    async fn a_typed_user_is_a_content_word() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mine = user_memory("The user lives in Recife");
        bank.commit(mine.clone()).await.unwrap();

        let (_, evidence) = evidence_for(&bank, "who is the user").await;
        assert_eq!(evidence[&mine.id].matched, ["user"]);
        assert_eq!(evidence[&mine.id].terms, 1);
    }

    #[tokio::test]
    async fn semantic_evidence_needs_the_floor_and_z_a_pool() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let near = memory("nearest by meaning");
        let below = memory("kiwi sits under the floor");
        let other = memory("kiwi has another model's vector");
        for m in [&near, &below, &other] {
            bank.commit(m.clone()).await.unwrap();
        }
        bank.set_vector(&near.id, "m", &near.text, vec![1.0, 0.0])
            .await
            .unwrap();
        bank.set_vector(&below.id, "m", &below.text, vec![0.1, 0.995])
            .await
            .unwrap();
        bank.set_vector(&other.id, "x", &other.text, vec![1.0, 0.0])
            .await
            .unwrap();
        // 17 orthogonal fillers: 19 vectors of model "m" in all.
        for i in 0..17 {
            let f = memory(&format!("filler {i}"));
            bank.commit(f.clone()).await.unwrap();
            bank.set_vector(&f.id, "m", &f.text, vec![0.0, 1.0])
                .await
                .unwrap();
        }
        let q = [1.0, 0.0];

        let (_, ev) = bank
            .recall_evidence("kiwi", Some(&q), "m", 50, 30, false, 0.0)
            .await;
        assert!((ev[&near.id].cosine.unwrap() - 1.0).abs() < 1e-6);
        assert!(ev[&near.id].vector);
        assert!(ev[&near.id].matched.is_empty());
        // Under the floor: a vector, but no semantic evidence.
        assert_eq!(ev[&below.id].cosine, None);
        assert!(ev[&below.id].vector);
        assert_eq!(ev[&below.id].matched, ["kiwi"]);
        assert_eq!(ev[&below.id].terms, 1);
        // Another model's vector is not a vector of the query's model.
        assert!(!ev[&other.id].vector);
        assert_eq!(ev[&other.id].cosine, None);
        // 19 vectors of the query's model (the other model's never count).
        assert_eq!(ev[&near.id].z, None);

        let last = memory("filler last");
        bank.commit(last.clone()).await.unwrap();
        bank.set_vector(&last.id, "m", &last.text, vec![0.0, 1.0])
            .await
            .unwrap();
        let (_, ev) = bank
            .recall_evidence("kiwi", Some(&q), "m", 50, 30, false, 0.0)
            .await;
        // mean 0.055, population sd 0.2179 over [1.0, 0.1, 0 x 18].
        let z = ev[&near.id].z.expect("20 vectors make a pool");
        assert!((z - 4.337).abs() < 0.01, "{z}");
        assert_eq!(ev[&below.id].z, None, "no cosine above the floor, no z");
    }

    #[tokio::test]
    async fn hybrid_policy_small_bank_similarity_requires_independent_words() {
        // Existing algebraic fixture, not an embedding-quality measurement.
        // Retain its exact vectors and candidate/ranking checks; only the
        // explicitly changed admission policy is different.
        let texts = [
            memory("The API gateway is written in Go."),
            memory("Nightly backups run at 2am UTC."),
            memory("The billing service runs on Postgres 16."),
            user_memory("The user works as a senior backend engineer."),
            user_memory("The user prefers a dark editor theme."),
        ];
        let questions = [
            "What language is the gateway in?",
            "When do backups happen?",
            "Which postgres version again?",
            "what's my job title?",
            "which theme do I like",
        ];
        let supported_questions = [
            "API gateway Go?",
            "Nightly backups UTC?",
            "billing service Postgres 16?",
            "senior backend engineer?",
            "dark editor theme?",
        ];
        for model in ["m", "sentence-transformers/all-MiniLM-L6-v2@rev"] {
            let dir = tempfile::tempdir().unwrap();
            let store = Store::open(dir.path().to_path_buf()).unwrap();
            let (bank, _) = store.ensure_bank("main", None).await.unwrap();
            let one_hot = |i: usize| {
                let mut v = vec![0.0f32; texts.len() + 1];
                v[i] = 1.0;
                v
            };
            for (i, m) in texts.iter().enumerate() {
                bank.commit(m.clone()).await.unwrap();
                bank.set_vector(&m.id, model, &m.text, one_hot(i))
                    .await
                    .unwrap();
            }
            for (target, question) in questions.iter().enumerate() {
                // Cosine 0.8 to its memory, 0 to the others.
                let mut q = one_hot(target);
                q[target] = 0.8;
                q[texts.len()] = 0.6;
                let (hits, ev) = bank
                    .recall_evidence(question, Some(&q), model, usize::MAX, 30, false, 0.0)
                    .await;
                assert_eq!(ev[&texts[target].id].z, None, "{question}");
                let picked =
                    crate::hooks::select(hits, &ev, &std::collections::HashSet::new(), 6, question);
                let picked: Vec<&str> = picked.iter().map(|(m, _)| m.text.as_str()).collect();
                assert!(
                    picked.is_empty(),
                    "similarity without sufficient words: {model} {question}: {picked:?}"
                );
                let (supported_hits, supported_ev) = bank
                    .recall_evidence(
                        supported_questions[target],
                        Some(&q),
                        model,
                        usize::MAX,
                        30,
                        false,
                        0.0,
                    )
                    .await;
                let supported = crate::hooks::select(
                    supported_hits,
                    &supported_ev,
                    &HashSet::new(),
                    6,
                    supported_questions[target],
                );
                assert_eq!(
                    supported
                        .iter()
                        .map(|(m, _)| m.id.as_str())
                        .collect::<Vec<_>>(),
                    [texts[target].id.as_str()],
                    "explicit support must survive: {model}/{target}"
                );
            }
        }
    }

    #[tokio::test]
    async fn semantic_ranks_raw_cosine_with_floor_model_and_liveness() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let near = memory("near");
        let mid = memory("mid");
        let far = memory("far");
        let other = memory("other model");
        for m in [&near, &mid, &far, &other] {
            bank.commit(m.clone()).await.unwrap();
        }
        bank.set_vector(&near.id, "m", &near.text, vec![1.0, 0.0])
            .await
            .unwrap();
        bank.set_vector(&mid.id, "m", &mid.text, vec![0.6, 0.8])
            .await
            .unwrap();
        bank.set_vector(&far.id, "m", &far.text, vec![0.0, 1.0])
            .await
            .unwrap();
        bank.set_vector(&other.id, "x", &other.text, vec![1.0, 0.0])
            .await
            .unwrap();

        let hits = bank.semantic(&[1.0, 0.0], "m", 10, false).await;
        let ids: Vec<&str> = hits.iter().map(|(m, _)| m.id.as_str()).collect();
        assert_eq!(
            ids,
            vec![near.id.as_str(), mid.id.as_str()],
            "far is under the floor, other is another model"
        );
        assert!(
            (hits[0].1 - 1.0).abs() < 1e-6 && (hits[1].1 - 0.6).abs() < 1e-6,
            "raw cosine scores"
        );

        let mut retired = near.clone();
        retired.invalid_at = Some(now_ms());
        retired.revision += 1;
        bank.commit(retired).await.unwrap();
        let live = bank.semantic(&[1.0, 0.0], "m", 10, false).await;
        assert!(live.iter().all(|(m, _)| m.id != near.id));
    }

    #[tokio::test]
    async fn minilm_vectors_use_their_own_semantic_floor() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let minilm = format!("{MINILM_MODEL}@c9745ed1d9f2");
        let a = memory("weak match a");
        let b = memory("weak match b");
        // Cosine 0.22 with the query: under the default floor, over MiniLM's.
        let v = vec![0.22, (1.0f32 - 0.22 * 0.22).sqrt()];
        for (m, model) in [(&a, minilm.as_str()), (&b, "text-embedding-3-small")] {
            bank.commit(m.clone()).await.unwrap();
            bank.set_vector(&m.id, model, &m.text, v.clone())
                .await
                .unwrap();
        }
        for model in [minilm.as_str(), "text-embedding-3-small"] {
            let semantic = bank.semantic(&[1.0, 0.0], model, 10, false).await;
            let recalled = bank
                .recall("", Some(&[1.0, 0.0]), model, 10, 60, false)
                .await;
            // The raw semantic arm keeps the default floor for every model.
            let want = usize::from(model == minilm);
            assert_eq!((semantic.len(), recalled.len()), (0, want), "{model}");
        }
    }

    #[tokio::test]
    async fn tied_scores_rank_deterministically_by_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        // Same length, same age, one shared query token each: every score
        // ties, so only the tie-break decides the order.
        for w in ["one", "two", "three", "four", "five", "six"] {
            let mut m = memory(&format!("kiwi {w}"));
            m.updated_at = 1_000;
            bank.commit(m).await.unwrap();
        }
        let ids: Vec<String> = bank
            .recall("kiwi", None, "", 10, 30, false)
            .await
            .into_iter()
            .map(|(m, _)| m.id)
            .collect();
        let mut sorted = ids.clone();
        sorted.sort();
        assert_eq!(ids.len(), 6);
        assert_eq!(ids, sorted, "ties break by id, not HashMap order");
    }

    #[tokio::test]
    async fn lexical_only_recall_keeps_bm25_order() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        // Same age, no pins or entities: normalizing the BM25 arm by its
        // maximum must not reorder anything.
        for text in [
            "kiwi mango smoothie recipe",
            "kiwi mango",
            "kiwi with a lot of other filler words around it",
            "mango sorbet",
        ] {
            let mut m = memory(text);
            m.updated_at = 1_000;
            bank.commit(m).await.unwrap();
        }
        let q = "kiwi mango smoothie";
        let ids =
            |hits: Vec<(Memory, f32)>| hits.into_iter().map(|(m, _)| m.id).collect::<Vec<_>>();
        let lexical = ids(bank.lexical(q, 10, false).await);
        assert_eq!(lexical.len(), 4);
        assert_eq!(ids(bank.recall(q, None, "", 10, 30, false).await), lexical);
    }

    #[tokio::test]
    async fn recall_ignores_vectors_from_another_pinned_model() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let m = memory("alpha beta");
        bank.commit(m.clone()).await.unwrap();
        bank.set_vector(&m.id, "model-a", &m.text, vec![1.0, 0.0])
            .await
            .unwrap();
        let q = [1.0, 0.0];

        let same = bank.recall("zzz", Some(&q), "model-a", 5, 30, false).await;
        assert_eq!(same.len(), 1, "matching model contributes the hit");
        let other = bank.recall("zzz", Some(&q), "model-b", 5, 30, false).await;
        assert!(other.is_empty(), "another model's space is not comparable");
        // Empty = provider default: whatever exists counts (mirrors
        // `unembedded`, so nothing skipped here is left un-re-embedded).
        let default = bank.recall("zzz", Some(&q), "", 5, 30, false).await;
        assert_eq!(default.len(), 1);
    }

    #[tokio::test]
    async fn history_recall_surfaces_tombstones_lexical_only() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mut m = memory("the retired convention");
        bank.commit(m.clone()).await.unwrap();
        m.invalid_at = Some(now_ms());
        m.revision += 1;
        bank.commit(m.clone()).await.unwrap();

        assert!(bank
            .recall("retired convention", None, "", 5, 30, false)
            .await
            .is_empty());
        let hits = bank
            .recall("retired convention", None, "", 5, 30, true)
            .await;
        assert_eq!(
            hits.len(),
            1,
            "history mode must reach non-live records with no query vector"
        );
        assert_eq!(hits[0].0.id, m.id);
    }

    #[tokio::test]
    async fn ensure_bank_is_idempotent_and_keeps_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, created) = store.ensure_bank("main", Some("first")).await.unwrap();
        assert!(created);
        bank.commit(memory("keep across ensure")).await.unwrap();
        let (bank2, created2) = store.ensure_bank("main", Some("second")).await.unwrap();
        assert!(!created2);
        let (memories, total) = bank2.list(10, 0, false, None).await;
        assert_eq!(total, 1);
        assert_eq!(memories[0].text, "keep across ensure");
    }

    #[tokio::test]
    async fn reload_picks_up_hand_edited_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        bank.set_rule("style", "# Original").unwrap();

        // Hand-edit the file on disk, as an operator would.
        std::fs::write(dir.path().join("main/rules/style.md"), "# Edited by hand").unwrap();
        std::fs::write(dir.path().join("main/rules/added.md"), "# Added by hand").unwrap();
        let count = store.reload().await.unwrap();
        assert_eq!(count, 1);
        let bank = store.bank("main").await.unwrap();
        let rules = bank.list_rules().unwrap();
        let names: Vec<&str> = rules.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["added", "style"]);
        assert_eq!(rules[1].content, "# Edited by hand");
    }

    #[tokio::test]
    async fn list_pages_newest_first_with_stable_totals() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        for i in 0..5 {
            let mut m = memory(&format!("memory number {i}"));
            m.updated_at = i;
            bank.commit(m).await.unwrap();
        }
        let (page1, total) = bank.list(2, 0, false, None).await;
        let (page2, _) = bank.list(2, 2, false, None).await;
        assert_eq!(total, 5);
        assert_eq!(page1[0].updated_at, 4);
        assert_eq!(page1[1].updated_at, 3);
        assert_eq!(page2[0].updated_at, 2);
    }

    #[tokio::test]
    async fn tag_filter_narrows_list_and_tags_counts() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mut a = memory("iii uses three primitives");
        a.tags = vec!["iii".into()];
        let mut b = memory("workers self-register");
        b.tags = vec!["iii".into(), "workers".into()];
        let c = memory("untagged note");
        for m in [&a, &b, &c] {
            bank.commit(m.clone()).await.unwrap();
        }
        let (all, total) = bank.list(10, 0, false, None).await;
        assert_eq!((all.len(), total), (3, 3));
        let (iii, total) = bank.list(10, 0, false, Some("iii")).await;
        assert_eq!((iii.len(), total), (2, 2));
        let (w, total) = bank.list(10, 0, false, Some("workers")).await;
        assert_eq!((w.len(), total), (1, 1));
        assert_eq!(
            bank.tags().await,
            vec![("iii".to_string(), 2), ("workers".to_string(), 1)]
        );
    }

    #[tokio::test]
    async fn tags_survive_reopen_and_old_records_load_untagged() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().to_path_buf()).unwrap();
        let (bank, _) = store.ensure_bank("main", None).await.unwrap();
        let mut m = memory("tagged and durable");
        m.tags = vec!["billing".into()];
        bank.commit(m).await.unwrap();
        // A pre-tags record on disk (no tags field at all).
        let legacy = "{\"id\":\"fplegacy0000000000\",\"text\":\"old record\",\"confidence\":\"stated\",\"created_at\":1,\"updated_at\":1}";
        {
            use std::io::Write as _;
            let mut file = std::fs::OpenOptions::new()
                .append(true)
                .open(bank.dir().join(MEMORIES_FILE))
                .unwrap();
            file.write_all(legacy.as_bytes()).unwrap();
            file.write_all(b"\n").unwrap();
        }
        let store2 = Store::open(dir.path().to_path_buf()).unwrap();
        let bank2 = store2.bank("main").await.unwrap();
        let (tagged, _) = bank2.list(10, 0, false, Some("billing")).await;
        assert_eq!(tagged.len(), 1);
        let (all, _) = bank2.list(10, 0, false, None).await;
        assert_eq!(all.len(), 2);
        assert!(all
            .iter()
            .any(|m| m.id == "fplegacy0000000000" && m.tags.is_empty()));
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
