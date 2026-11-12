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
//! Every mutation appends to the log under the bank write lock, then updates
//! map and index: file, map, and index update together or not at all. An
//! extraction effect appends its records and its source receipt line (never a
//! memory) in one write, so a redelivered job adds no second revision.

use std::collections::{HashMap, HashSet};
use std::io::{BufRead, Read, Seek, Write};
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
    /// question of pronouns only ("who am I"), or of `MEMORY_VERBS` asked
    /// of "me" ("what do you remember about me").
    pub matched: Vec<String>,
    /// How many content terms the question has.
    pub terms: usize,
    /// The question asks what memory holds of the user ("what do you
    /// remember about me?"), not a question of pronouns alone ("what should
    /// I do now?").
    pub of_me: bool,
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
    /// Extraction receipts this bank's log holds, replayed at open.
    receipts: HashSet<String>,
}

/// A source receipt line in `memories.jsonl`: one extraction effect was
/// published, or skipped because its expected snapshot conflicted.
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ReceiptLine {
    extraction_receipt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    disposition: Option<Disposition>,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
enum Disposition {
    ExpectedConflict,
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

/// Append whole lines to a bank log in one write, then fsync. A last line
/// without its newline (a hand edit, a torn write) would fuse with the first
/// appended record: both unparseable on replay, and a lost tombstone brings
/// its memory back. So it is closed first, in the same write.
fn append_log(path: &Path, lines: &[u8]) -> Result<(), MemoryError> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(path)?;
    let mut buf = Vec::with_capacity(lines.len() + 1);
    if file.metadata()?.len() > 0 {
        let mut last = [0u8];
        file.seek(std::io::SeekFrom::End(-1))?;
        file.read_exact(&mut last)?;
        if last[0] != b'\n' {
            buf.push(b'\n');
        }
    }
    buf.extend_from_slice(lines);
    file.write_all(&buf)?;
    file.sync_data()?;
    Ok(())
}

/// One JSON line for [`append_log`].
fn push_line(buf: &mut Vec<u8>, value: &impl serde::Serialize) -> Result<(), MemoryError> {
    serde_json::to_writer(&mut *buf, value).map_err(|e| MemoryError::Storage(e.to_string()))?;
    buf.push(b'\n');
    Ok(())
}

/// Replace a small file whole: hidden temp file beside it, fsync, rename
/// over. A crash leaves the old content or the new, never a torn file.
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

/// English function words dropped from QUERIES before lexical scoring (the
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
];

/// Verbs that ask what memory holds ("what do you remember about me"); English only, by decision.
/// Not "tell": its "me" is who is told ("tell me more").
const MEMORY_VERBS: &[&str] = &["remember", "know", "recall"];

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
    let first_person = tokens
        .iter()
        .any(|t| matches!(t.as_str(), "i" | "me" | "my" | "mine" | "myself"));
    if tokens.iter().any(|t| !STOPWORDS.contains(&t.as_str())) {
        tokens.retain(|t| !STOPWORDS.contains(&t.as_str()));
    }
    let expanded = first_person && !tokens.iter().any(|t| t == "user");
    if expanded {
        tokens.push("user".to_string());
    }
    (tokens, expanded)
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
pub(crate) fn run_blocking<T>(
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

/// Captured source is retained independently of the queue's retry lifetime.
/// It holds what the model reads of each entry (redacted, a long text cut to
/// its head and tail) and a digest over the full text.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingExtraction {
    pub version: u32,
    pub source_digest: Option<String>,
    pub entries: Vec<(String, String, String, u64)>,
    pub bank: String,
    pub derived: bool,
    pub agent: Option<String>,
    pub turn_id: Option<String>,
    pub attempts: u32,
}

impl PendingExtraction {
    /// Conservative escaped JSON budget without creating a Value or digest.
    fn checked_bytes(&self) -> Result<usize, MemoryError> {
        for id in std::iter::once(self.bank.as_str())
            .chain(self.agent.as_deref())
            .chain(self.turn_id.as_deref())
            .chain(self.source_digest.as_deref())
        {
            extraction_identifier(id)?;
        }
        Self::entries_bytes(&self.entries)?;
        let mut size = 1024;
        let cap = crate::extraction_proof::HISTORY_BYTES;
        let strings = std::iter::once(self.bank.as_str())
            .chain(self.agent.as_deref())
            .chain(self.turn_id.as_deref())
            .chain(self.source_digest.as_deref())
            .chain(
                self.entries
                    .iter()
                    .flat_map(|e| [e.0.as_str(), e.1.as_str(), e.2.as_str()]),
            );
        for s in strings {
            let bytes = s
                .len()
                .checked_mul(6)
                .and_then(|n| n.checked_add(64))
                .ok_or_else(|| MemoryError::InvalidInput("extraction byte overflow".into()))?;
            crate::extraction_proof::charge(&mut size, bytes, cap)
                .map_err(MemoryError::InvalidInput)?;
        }
        Ok(size)
    }

    fn borrowed_identifiers(value: &serde_json::Value) -> Result<(), MemoryError> {
        for field in ["bank", "agent", "turn_id", "source_digest"] {
            if let Some(id) = value.get(field).and_then(serde_json::Value::as_str) {
                extraction_identifier(id)?;
            }
        }
        if let Some(entries) = value.get("entries").and_then(serde_json::Value::as_array) {
            if entries.len() > 65536 {
                return Err(MemoryError::InvalidInput("extraction entry bound".into()));
            }
            for entry in entries {
                for index in [0, 1] {
                    if let Some(id) = entry.get(index).and_then(serde_json::Value::as_str) {
                        extraction_identifier(id)?;
                    }
                }
                if entry
                    .get(2)
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|text| text.len() > crate::extraction_proof::TEXT_BYTES)
                {
                    return Err(MemoryError::InvalidInput(
                        "extraction text byte bound".into(),
                    ));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn entries_bytes(
        entries: &[(String, String, String, u64)],
    ) -> Result<(), MemoryError> {
        let mut size = 1024;
        for (id, role, text, _) in entries {
            extraction_identifier(id)?;
            extraction_identifier(role)?;
            if text.len() > crate::extraction_proof::TEXT_BYTES {
                return Err(MemoryError::InvalidInput(
                    "extraction text byte bound".into(),
                ));
            }
            for s in [id.as_str(), role.as_str(), text.as_str()] {
                let bytes = s
                    .len()
                    .checked_mul(6)
                    .and_then(|n| n.checked_add(64))
                    .ok_or_else(|| MemoryError::InvalidInput("extraction byte overflow".into()))?;
                crate::extraction_proof::charge(
                    &mut size,
                    bytes,
                    crate::extraction_proof::HISTORY_BYTES,
                )
                .map_err(MemoryError::InvalidInput)?;
            }
        }
        Ok(())
    }

    pub fn parse(value: serde_json::Value) -> Result<Self, MemoryError> {
        Self::borrowed_identifiers(&value)?;
        crate::extraction_proof::value_bytes(&value, crate::extraction_proof::HISTORY_BYTES)
            .map_err(MemoryError::Storage)?;
        let pending: Self = serde_json::from_value(value)
            .map_err(|e| MemoryError::Storage(format!("invalid pending extraction: {e}")))?;
        pending.checked_bytes()?;
        validate_name(&pending.bank)?;
        if !matches!(
            (pending.version, pending.source_digest.as_deref()),
            (1, Some(_))
        ) || pending.source_digest.as_ref().is_some_and(|digest| {
            digest.len() != 32 || !digest.bytes().all(|b| b.is_ascii_hexdigit())
        }) {
            return Err(MemoryError::Storage(
                "unsupported pending extraction schema or digest".into(),
            ));
        }
        let mut ids = HashSet::new();
        if pending.entries.is_empty()
            || pending.entries.iter().any(|(id, role, _, _)| {
                id.is_empty()
                    || !ids.insert(id)
                    || !matches!(role.as_str(), "user" | "assistant" | "notice")
            })
        {
            return Err(MemoryError::Storage(
                "invalid pending extraction range".into(),
            ));
        }
        Ok(pending)
    }

    /// Non-cryptographic change detector over the full unredacted ordered
    /// source. Never authority: exact captured source AND trusted policy must
    /// also match.
    pub(crate) fn digest(entries: &[(String, String, String, u64)]) -> String {
        let bytes = serde_json::to_vec(entries).expect("source tuples serialize");
        let hash = bytes
            .iter()
            .fold(0x6c62272e07bb014262b821756295c58d_u128, |h, b| {
                (h ^ u128::from(*b)).wrapping_mul(0x0000000001000000000000000000013b)
            });
        format!("{hash:032x}")
    }

    pub fn same_source(&self, other: &Self) -> bool {
        self.entries == other.entries
            && self.version == other.version
            && self.source_digest == other.source_digest
            && self.bank == other.bank
            && self.derived == other.derived
            && self.agent == other.agent
            && self.turn_id == other.turn_id
    }

    /// The plan-store key of this run: session, scope, turn and ordered
    /// entry ids, so a retry of the exact run finds its model decision.
    pub(crate) fn source_key(&self, session: &str) -> String {
        serde_json::json!([
            session,
            self.bank,
            self.derived,
            self.agent,
            self.turn_id,
            self.entries.iter().map(|e| &e.0).collect::<Vec<_>>()
        ])
        .to_string()
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtractionRange {
    pub pending: PendingExtraction,
    pub completed: bool,
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ExtractionRanges {
    version: u32,
    ranges: Vec<ExtractionRange>,
}

/// Serializes `.extraction` reads and writes. Process-wide: a Store from
/// before a reload shares the directory with the current one.
static EXTRACTION: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn extraction_lock() -> std::sync::MutexGuard<'static, ()> {
    EXTRACTION.lock().unwrap_or_else(|e| e.into_inner())
}

fn remove_if_exists(path: &Path) -> Result<(), MemoryError> {
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

/// The most ranges one session's ledger keeps.
const MAX_RANGES: usize = 128;

/// A plan key's length ([`PendingExtraction::source_key`]); session-level
/// records (`pending`, `ranges`, history) have two-element keys.
const PLAN_KEY_LEN: usize = 6;

fn ranges_key(session: &str) -> String {
    serde_json::json!(["ranges", session]).to_string()
}

fn history_key(session: &str) -> String {
    serde_json::json!(["authenticated-history-v1", session]).to_string()
}

fn extraction_identifier(id: &str) -> Result<(), MemoryError> {
    if !crate::extraction_proof::bounded(id) {
        return Err(MemoryError::InvalidInput(
            "extraction identifier byte bound".into(),
        ));
    }
    Ok(())
}

impl Store {
    fn extraction_plan_path(&self, source: &str) -> PathBuf {
        // Hash is only a locator: the exact source is verified on every read.
        let hash = source.bytes().fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
        });
        self.root
            .join(".extraction")
            .join(format!("{hash:016x}.json"))
    }

    /// Durable model decision and original prompt snapshots, before any effect.
    /// Internal data, not a public memory or schema surface.
    pub fn extraction_plan(&self, source: &str) -> Result<Option<serde_json::Value>, MemoryError> {
        let _lock = extraction_lock();
        self.extraction_plan_unlocked(source)
    }

    fn extraction_plan_unlocked(
        &self,
        source: &str,
    ) -> Result<Option<serde_json::Value>, MemoryError> {
        Ok(self
            .extraction_record(source)?
            .and_then(|mut record| record.remove("plan")))
    }

    /// One `.extraction` record: `source`, `plan`, and on a plan the banks
    /// whose learned-rule merge the run already settled (`rules`).
    fn extraction_record(
        &self,
        source: &str,
    ) -> Result<Option<serde_json::Map<String, serde_json::Value>>, MemoryError> {
        if source.len() > crate::extraction_proof::SOURCE_BYTES / 6 {
            return Err(MemoryError::InvalidInput(
                "extraction source byte bound".into(),
            ));
        }
        // Cap the actual read (including a concurrent file growth) BEFORE
        // deserialization. This applies to every extraction-plan public reader.
        let file = match std::fs::File::open(self.extraction_plan_path(source)) {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e.into()),
        };
        let cap = crate::extraction_proof::RECORD_BYTES;
        if file.metadata()?.len() > cap as u64 {
            return Err(MemoryError::Storage("extraction record byte bound".into()));
        }
        let mut bytes = Vec::new();
        file.take(cap as u64 + 1).read_to_end(&mut bytes)?;
        if bytes.len() > cap {
            return Err(MemoryError::Storage("extraction record byte bound".into()));
        }
        let record: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| MemoryError::Storage(e.to_string()))?;
        crate::extraction_proof::value_bytes(&record, cap).map_err(MemoryError::Storage)?;
        let serde_json::Value::Object(record) = record else {
            return Err(MemoryError::Storage(
                "invalid extraction plan envelope".into(),
            ));
        };
        if !record.contains_key("plan")
            || record
                .keys()
                .any(|key| !matches!(key.as_str(), "source" | "plan" | "rules"))
            || record.get("rules").is_some_and(|rules| {
                !rules
                    .as_array()
                    .is_some_and(|banks| banks.iter().all(serde_json::Value::is_string))
            })
        {
            return Err(MemoryError::Storage(
                "invalid extraction plan envelope".into(),
            ));
        }
        if record.get("source").and_then(serde_json::Value::as_str) != Some(source) {
            return Err(MemoryError::Storage(
                "extraction source identity mismatch".into(),
            ));
        }
        Ok(Some(record))
    }

    fn write_extraction_record(
        &self,
        source: &str,
        record: serde_json::Value,
    ) -> Result<(), MemoryError> {
        let path = self.extraction_plan_path(source);
        std::fs::create_dir_all(path.parent().unwrap())?;
        let bytes = serde_json::to_vec(&record).map_err(|e| MemoryError::Storage(e.to_string()))?;
        write_atomic(&path, &bytes)?;
        Ok(())
    }

    /// Only the authenticated receiver writes these rows; queue payloads never do.
    pub(crate) fn authenticated_history(
        &self,
        session: &str,
    ) -> Result<Vec<crate::extraction_proof::Row>, MemoryError> {
        extraction_identifier(session)?;
        Self::decode_authenticated_history(self.extraction_plan(&history_key(session))?)
    }

    fn decode_authenticated_history(
        value: Option<serde_json::Value>,
    ) -> Result<Vec<crate::extraction_proof::Row>, MemoryError> {
        let Some(value) = value else {
            return Ok(Vec::new());
        };
        crate::extraction_proof::value_bytes(&value, crate::extraction_proof::HISTORY_BYTES)
            .map_err(MemoryError::Storage)?;
        let array = value
            .as_array()
            .filter(|a| a.len() <= 1024)
            .ok_or_else(|| MemoryError::Storage("invalid history rows".into()))?;
        for row in array {
            crate::extraction_proof::value_bytes(row, crate::extraction_proof::ROW_BYTES)
                .map_err(MemoryError::Storage)?;
        }
        let rows: Vec<crate::extraction_proof::Row> = serde_json::from_value(value)
            .map_err(|e| MemoryError::Storage(format!("invalid authenticated history: {e}")))?;
        crate::extraction_proof::rows_bytes(&rows).map_err(MemoryError::Storage)?;
        Ok(rows)
    }

    /// Bounded, immutable, Memory-owned evidence, persisted before acknowledgement.
    pub(crate) fn save_authenticated_history(
        &self,
        session: &str,
        incoming: &[crate::extraction_proof::Row],
    ) -> Result<(), MemoryError> {
        extraction_identifier(session)?;
        let key = history_key(session);
        let _lock = extraction_lock();
        crate::extraction_proof::rows_bytes(incoming).map_err(MemoryError::InvalidInput)?;
        let mut rows = Self::decode_authenticated_history(self.extraction_plan_unlocked(&key)?)?;
        // Check the union using borrowed rows before cloning any new strings.
        let mut additions = Vec::new();
        for row in incoming {
            if let Some(old) = rows
                .iter()
                .find(|old| old.attestation.turn_id == row.attestation.turn_id)
            {
                if old != row {
                    return Err(MemoryError::InvalidInput("history proof changed".into()));
                }
            } else {
                additions.push(row);
            }
        }
        if additions.is_empty() {
            return Ok(());
        }
        crate::extraction_proof::rows_bytes_iter(rows.iter().chain(additions.iter().copied()))
            .map_err(MemoryError::InvalidInput)?;
        rows.extend(additions.into_iter().cloned());
        self.write_extraction_record(&key, serde_json::json!({"source":key,"plan":rows}))
    }

    /// Drop the history rows of entries the confirmed cursor already passed:
    /// extraction never reads them again.
    pub(crate) fn prune_authenticated_history(
        &self,
        session: &str,
        confirmed: &HashSet<&str>,
    ) -> Result<(), MemoryError> {
        extraction_identifier(session)?;
        let _lock = extraction_lock();
        self.prune_history_unlocked(session, |id| confirmed.contains(id))
    }

    fn prune_history_unlocked(
        &self,
        session: &str,
        confirmed: impl Fn(&str) -> bool,
    ) -> Result<(), MemoryError> {
        let key = history_key(session);
        let mut rows = Self::decode_authenticated_history(self.extraction_plan_unlocked(&key)?)?;
        let before = rows.len();
        rows.retain(|row| !confirmed(&row.attestation.entry_id));
        if rows.len() == before {
            Ok(())
        } else if rows.is_empty() {
            remove_if_exists(&self.extraction_plan_path(&key))
        } else {
            self.write_extraction_record(&key, serde_json::json!({"source":key,"plan":rows}))
        }
    }

    pub fn save_extraction_plan(
        &self,
        source: &str,
        plan: &serde_json::Value,
    ) -> Result<(), MemoryError> {
        if source.len() > crate::extraction_proof::SOURCE_BYTES / 6 {
            return Err(MemoryError::InvalidInput(
                "extraction source byte bound".into(),
            ));
        }
        crate::extraction_proof::value_bytes(plan, crate::extraction_proof::RECORD_BYTES / 2)
            .map_err(MemoryError::InvalidInput)?;
        let _lock = extraction_lock();
        if let Some(existing) = self.extraction_plan_unlocked(source)? {
            if existing != *plan {
                return Err(MemoryError::InvalidInput(
                    "extraction decision already recorded".into(),
                ));
            }
            return Ok(());
        }
        self.write_extraction_record(source, serde_json::json!({"source":source,"plan":plan}))
    }

    /// Drop a finished run's plan, and with it its rule receipts: the cursor
    /// or its completed range already keeps the run from executing again.
    pub fn delete_extraction_plan(&self, source: &str) -> Result<(), MemoryError> {
        let _lock = extraction_lock();
        remove_if_exists(&self.extraction_plan_path(source))
    }

    /// Banks whose learned-rule merge this run already settled.
    pub fn rule_receipts(&self, source: &str) -> Result<Vec<String>, MemoryError> {
        let _lock = extraction_lock();
        Ok(self
            .extraction_record(source)?
            .and_then(|mut record| serde_json::from_value(record.remove("rules")?).ok())
            .unwrap_or_default())
    }

    /// Record a settled learned-rule merge in the run's plan record, so the
    /// receipt goes when the plan does.
    pub fn save_rule_receipt(&self, source: &str, bank: &str) -> Result<(), MemoryError> {
        let _lock = extraction_lock();
        let mut record = self
            .extraction_record(source)?
            .ok_or_else(|| MemoryError::Storage("rule receipt without its plan".into()))?;
        let rules = record
            .entry("rules")
            .or_insert_with(|| serde_json::json!([]));
        if rules
            .as_array()
            .is_some_and(|banks| banks.iter().any(|b| b == bank))
        {
            return Ok(());
        }
        if let Some(banks) = rules.as_array_mut() {
            banks.push(bank.into());
        }
        self.write_extraction_record(source, serde_json::Value::Object(record))
    }

    /// One in-flight source interval per session; persisted before its model or
    /// effects, replayed before any newer job can widen the delta.
    pub fn pending_extraction(
        &self,
        session: &str,
    ) -> Result<Option<serde_json::Value>, MemoryError> {
        extraction_identifier(session)?;
        let key = serde_json::json!(["pending", session]).to_string();
        self.extraction_plan(&key)
            .map(|value| value.filter(|v| !v.is_null()))
    }

    pub fn set_pending_extraction(
        &self,
        session: &str,
        value: serde_json::Value,
    ) -> Result<(), MemoryError> {
        extraction_identifier(session)?;
        PendingExtraction::borrowed_identifiers(&value)?;
        crate::extraction_proof::value_bytes(&value, crate::extraction_proof::RECORD_BYTES / 2)
            .map_err(MemoryError::InvalidInput)?;
        if session.len() > 256 {
            return Err(MemoryError::InvalidInput(
                "extraction session byte bound".into(),
            ));
        }
        let key = serde_json::json!(["pending", session]).to_string();
        let _lock = extraction_lock();
        self.write_extraction_record(&key, serde_json::json!({"source":key,"plan":value}))
    }

    fn extraction_ranges_unlocked(
        &self,
        session: &str,
    ) -> Result<Vec<ExtractionRange>, MemoryError> {
        extraction_identifier(session)?;
        let Some(value) = self.extraction_plan_unlocked(&ranges_key(session))? else {
            return Ok(Vec::new());
        };
        crate::extraction_proof::value_bytes(&value, crate::extraction_proof::HISTORY_BYTES)
            .map_err(MemoryError::Storage)?;
        if let Some(ranges) = value["ranges"].as_array() {
            for range in ranges {
                PendingExtraction::borrowed_identifiers(&range["pending"])?;
            }
        }
        let ledger: ExtractionRanges = serde_json::from_value(value)
            .map_err(|e| MemoryError::Storage(format!("invalid extraction ranges: {e}")))?;
        if ledger.version != 1 || ledger.ranges.len() > MAX_RANGES {
            return Err(MemoryError::Storage(
                "unsupported extraction ranges version or bound".into(),
            ));
        }
        let mut ids = HashSet::new();
        for range in &ledger.ranges {
            range.pending.checked_bytes()?;
            PendingExtraction::parse(
                serde_json::to_value(&range.pending)
                    .map_err(|e| MemoryError::Storage(e.to_string()))?,
            )?;
            if range
                .pending
                .entries
                .iter()
                .any(|e| !ids.insert(e.0.clone()))
            {
                return Err(MemoryError::Storage("overlapping extraction ranges".into()));
            }
        }
        Ok(ledger.ranges)
    }

    fn write_ranges(&self, session: &str, ranges: Vec<ExtractionRange>) -> Result<(), MemoryError> {
        let key = ranges_key(session);
        let record =
            serde_json::json!({"source":key,"plan":ExtractionRanges { version: 1, ranges }});
        self.write_extraction_record(&key, record)
    }

    /// Internal per-session retention receipts, never permission to publish.
    pub fn extraction_ranges(&self, session: &str) -> Result<Vec<ExtractionRange>, MemoryError> {
        let _lock = extraction_lock();
        self.extraction_ranges_unlocked(session)
    }

    /// Preserve every entry before releasing the single pending slot. Completed
    /// ranges are receipts, not a cursor: an unresolved prefix still pins it.
    /// A completed run never executes again, so its plan goes now.
    pub fn retain_extraction_range(
        &self,
        session: &str,
        pending: PendingExtraction,
        completed: bool,
    ) -> Result<(), MemoryError> {
        extraction_identifier(session)?;
        pending.checked_bytes()?;
        PendingExtraction::parse(
            serde_json::to_value(&pending).map_err(|e| MemoryError::Storage(e.to_string()))?,
        )?;
        let plan = pending.source_key(session);
        let _lock = extraction_lock();
        let mut ranges = self.extraction_ranges_unlocked(session)?;
        if let Some(existing) = ranges.iter_mut().find(|r| r.pending.same_source(&pending)) {
            existing.completed |= completed;
            existing.pending.attempts = existing.pending.attempts.max(pending.attempts);
        } else {
            if ranges.iter().any(|r| {
                r.pending
                    .entries
                    .iter()
                    .any(|e| pending.entries.iter().any(|p| e.0 == p.0))
            }) {
                return Err(MemoryError::Storage("extraction retention overlap".into()));
            }
            ranges.push(ExtractionRange { pending, completed });
        }
        // Source still waiting for extraction is never evicted.
        let mut budget = 1024;
        for range in &ranges {
            crate::extraction_proof::charge(
                &mut budget,
                range.pending.checked_bytes()?,
                crate::extraction_proof::HISTORY_BYTES,
            )
            .map_err(MemoryError::Storage)?;
        }
        if ranges.len() > MAX_RANGES {
            return Err(MemoryError::Storage("extraction retention bound".into()));
        }
        self.write_ranges(session, ranges)?;
        if completed {
            remove_if_exists(&self.extraction_plan_path(&plan))?;
        }
        Ok(())
    }

    /// Remove only a completed range after the caller confirmed its cursor,
    /// with the history rows of its entries: the cursor passed them.
    /// Uncompleted source is never evicted here.
    pub fn acknowledge_extraction_range(
        &self,
        session: &str,
        pending: &PendingExtraction,
    ) -> Result<(), MemoryError> {
        extraction_identifier(session)?;
        pending.checked_bytes()?;
        let _lock = extraction_lock();
        let mut ranges = self.extraction_ranges_unlocked(session)?;
        ranges.retain(|range| !(range.completed && range.pending.same_source(pending)));
        self.write_ranges(session, ranges)?;
        self.prune_history_unlocked(session, |id| pending.entries.iter().any(|e| e.0 == id))?;
        remove_if_exists(&self.extraction_plan_path(&pending.source_key(session)))
    }

    /// Drop a range that can never run as recorded (its source left the
    /// path or changed), with its plan.
    pub fn forget_extraction_range(
        &self,
        session: &str,
        pending: &PendingExtraction,
    ) -> Result<(), MemoryError> {
        extraction_identifier(session)?;
        let _lock = extraction_lock();
        let mut ranges = self.extraction_ranges_unlocked(session)?;
        let before = ranges.len();
        ranges.retain(|range| !range.pending.same_source(pending));
        if ranges.len() != before {
            self.write_ranges(session, ranges)?;
        }
        remove_if_exists(&self.extraction_plan_path(&pending.source_key(session)))
    }

    /// Delete every `.extraction` record `stale` selects by its parsed key.
    /// A rare path (session delete, bank trash): it scans.
    fn purge_extraction(
        &self,
        stale: impl Fn(&[serde_json::Value]) -> bool,
    ) -> Result<(), MemoryError> {
        let _lock = extraction_lock();
        let entries = match std::fs::read_dir(self.root.join(".extraction")) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.into()),
        };
        for entry in entries {
            let path = entry?.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let Ok(record) = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|bytes| {
                    serde_json::from_slice::<serde_json::Value>(&bytes).map_err(|e| e.to_string())
                })
            else {
                continue;
            };
            let key: Vec<serde_json::Value> = record["source"]
                .as_str()
                .and_then(|key| serde_json::from_str(key).ok())
                .unwrap_or_default();
            if stale(&key) {
                remove_if_exists(&path)?;
            }
        }
        Ok(())
    }

    /// A deleted session's pending slot, ranges, history ledger and plans.
    pub fn purge_session_extraction(&self, session: &str) -> Result<(), MemoryError> {
        self.purge_extraction(|key| match key {
            [owner, ..] if key.len() == PLAN_KEY_LEN => owner == session,
            [_, owner] => owner == session,
            _ => false,
        })
    }

    /// Plans of runs into a trashed bank. A run that only reads or teaches
    /// it keeps its plan: replaying that plan is what keeps its receipts
    /// matching its effects, and the plan goes when the run completes.
    fn purge_bank_plans(&self, bank: &str) -> Result<(), MemoryError> {
        self.purge_extraction(|key| key.len() == PLAN_KEY_LEN && key[1] == bank)
    }

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
        self.ensure_bank_inner(name, description, false).await
    }

    /// Automatic extraction cannot undo a concurrent explicit deletion,
    /// except of `default`, which materializes on use like everywhere else.
    /// The policy check shares the bank-map lock with `trash_bank`.
    pub(crate) async fn ensure_extraction_bank(
        &self,
        name: &str,
        default: &str,
    ) -> Result<(Arc<Bank>, bool), MemoryError> {
        self.ensure_bank_inner(name, None, name != default).await
    }

    async fn ensure_bank_inner(
        &self,
        name: &str,
        description: Option<&str>,
        reject_deleted: bool,
    ) -> Result<(Arc<Bank>, bool), MemoryError> {
        validate_name(name)?;
        let mut banks = self.banks.write().await;
        if reject_deleted && self.is_deleted(name) {
            return Err(MemoryError::BankNotFound(name.to_string()));
        }
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
    pub async fn trash_bank(&self, name: &str, default_bank: &str) -> Result<String, MemoryError> {
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
        // The default bank's runs go on (it materializes on use): its plans
        // stay. The bank is already gone: a failed purge is logged, not reported.
        if name != default_bank {
            if let Err(e) = run_blocking(|| self.purge_bank_plans(name)).await {
                tracing::warn!(bank = name, error = %e, "extraction plan purge failed");
            }
        }
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
        let mut receipts = HashSet::new();
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
                    Err(e) => match serde_json::from_str::<ReceiptLine>(&line) {
                        Ok(receipt) => {
                            receipts.insert(receipt.extraction_receipt);
                        }
                        Err(_) => {
                            tracing::warn!(bank = name, line = lineno + 1, error = %e, "skipping corrupt memory line");
                        }
                    },
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
                receipts,
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
        Ok(self
            .commit_inner(memory, None)
            .await?
            .expect("an unguarded commit has no snapshot to conflict with"))
    }

    /// Guard read/modify/write callers against a consolidation or edit that
    /// committed after their read. `expected` None means the id must still
    /// be absent. `Ok(None)`: the record changed, nothing was written.
    pub async fn commit_expected(
        &self,
        memory: Memory,
        expected: Option<&Memory>,
    ) -> Result<Option<CommitKind>, MemoryError> {
        self.commit_inner(memory, Some(expected)).await
    }

    async fn commit_inner(
        &self,
        mut memory: Memory,
        expected: Option<Option<&Memory>>,
    ) -> Result<Option<CommitKind>, MemoryError> {
        self.redact_record(&mut memory);
        let mut inner = self.inner.write().await;
        if expected.is_some_and(|expected| inner.memories.get(&memory.id) != expected) {
            return Ok(None);
        }
        let mut line = Vec::new();
        push_line(&mut line, &memory)?;
        // The write lock must span the fsync (append order = replay
        // order), but a slow disk must not stall this Tokio worker thread.
        let path = self.dir.join(MEMORIES_FILE);
        run_blocking(move || append_log(&path, &line)).await?;

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
        Ok(Some(kind))
    }

    /// The backstop for every write path: no known secret shape reaches
    /// memories.jsonl, whoever built the record (the entry points redact
    /// first, so dedup fingerprints the redacted text), a record stored
    /// before a pattern existed included.
    fn redact_record(&self, memory: &mut Memory) {
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
    }

    /// Publish one extraction effect and its source receipt in one append.
    /// `old`, when given, is retired by `replacement` in the same write.
    /// `Ok(None)`: the receipt was already recorded, or a snapshot changed
    /// since the extraction read it; a conflict appends a terminal skipped
    /// receipt and publishes neither record. A stored record never fails
    /// the effect: one carrying a secret is written redacted, and one the
    /// injection screen flags (only a stored twin can be) skips it.
    pub async fn extraction_expected(
        &self,
        receipt: &str,
        mut replacement: Memory,
        expected: Option<&Memory>,
        old: Option<&Memory>,
    ) -> Result<Option<Option<Memory>>, MemoryError> {
        if old.is_some_and(|old| old.id == replacement.id || old.pinned || !old.is_live())
            || !replacement.is_live()
            || expected.is_some_and(|m| !m.is_live() || (m.pinned && m != &replacement))
        {
            return Err(MemoryError::InvalidInput(
                "invalid extraction boundary".into(),
            ));
        }
        let flagged = crate::redact::reject_injection("extraction", &replacement.text).is_err();
        self.redact_record(&mut replacement);
        let mut inner = self.inner.write().await;
        if inner.receipts.contains(receipt) {
            return Ok(None);
        }
        let conflict = flagged
            || inner.memories.get(&replacement.id) != expected
            || old.is_some_and(|old| inner.memories.get(&old.id) != Some(old));
        let retired = old.filter(|_| !conflict).map(|old| {
            let mut retired = crate::functions::items::supersede_transition(
                old.clone(),
                replacement.clone(),
                false,
                now_ms(),
            )
            .0;
            self.redact_record(&mut retired);
            retired
        });
        let mut lines = Vec::new();
        if !conflict {
            push_line(&mut lines, &replacement)?;
            if let Some(record) = &retired {
                push_line(&mut lines, record)?;
            }
        }
        push_line(
            &mut lines,
            &ReceiptLine {
                extraction_receipt: receipt.to_string(),
                disposition: conflict.then_some(Disposition::ExpectedConflict),
            },
        )?;
        let path = self.dir.join(MEMORIES_FILE);
        run_blocking(move || append_log(&path, &lines)).await?;
        inner.receipts.insert(receipt.to_string());
        if conflict {
            return Ok(None);
        }
        if inner
            .memories
            .get(&replacement.id)
            .is_some_and(|m| m.text != replacement.text)
        {
            inner.vectors.remove(&replacement.id);
        }
        inner.index.add(&replacement);
        inner.memories.insert(replacement.id.clone(), replacement);
        if let Some(retired) = &retired {
            inner.index.remove(&retired.id);
            inner.vectors.remove(&retired.id);
            inner.memories.insert(retired.id.clone(), retired.clone());
        }
        Ok(Some(retired))
    }

    /// Whether this bank's log already holds an extraction receipt.
    pub async fn extraction_seen(&self, receipt: &str) -> bool {
        self.inner.read().await.receipts.contains(receipt)
    }

    /// Joint compare-and-write for consolidation. The bank write lock covers
    /// BOTH preconditions, the transition, the append and index publication.
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
        let (mut retired, mut survivor) =
            crate::functions::items::supersede_transition(loser, winner, reinforce, now_ms());
        // A loaded record can predate commit-time redaction: it is written
        // back redacted, never with its secret again.
        for memory in std::iter::once(&mut retired).chain(survivor.iter_mut()) {
            self.redact_record(memory);
        }
        let mut lines = Vec::new();
        for memory in std::iter::once(&retired).chain(survivor.iter()) {
            push_line(&mut lines, memory)?;
        }
        let path = self.dir.join(MEMORIES_FILE);
        run_blocking(move || append_log(&path, &lines)).await?;
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
            .score(&query_tokens(query).0)
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
        let has = |w: &str| words.iter().any(|t| t == w);
        let verb = content.iter().any(|t| MEMORY_VERBS.contains(t));
        // "tell" asks it only beside a memory verb or "myself" ("tell me about myself").
        let tell = verb || has("myself");
        let of_me = content
            .iter()
            .all(|t| MEMORY_VERBS.contains(t) || *t == "anything" || (*t == "tell" && tell))
            // Or asked of the user addressed as "I": "do you know who I am?".
            && (has("me") || has("myself") || (has("you") && verb && has("i")));
        if expanded && (content.is_empty() || of_me) {
            // A question of pronouns alone, or of what memory holds of them, is about the user.
            content = vec!["user"];
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
                (include_superseded || f.is_live()).then(|| (f.clone(), s, sem.get(&id).copied()))
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
        let mut evidence = HashMap::with_capacity(candidates.len());
        let mut scored: Vec<(Memory, f32)> = Vec::with_capacity(candidates.len());
        for (f, s, cos) in candidates {
            // Semantic joins lexical BEFORE fusion so recency decays
            // both alike.
            let sem = ((cos.unwrap_or(0.0) - anchor) / (1.0 - anchor)).max(0.0);
            let relevance = s / max_bm25 + weight * sem;
            let fused =
                crate::index::fused_score(relevance, &f, &query_tokens, now, half_life_days);
            evidence.insert(
                f.id.clone(),
                Evidence {
                    matched: content
                        .iter()
                        .filter(|t| inner.index.has(t, &f.id))
                        .map(|t| t.to_string())
                        .collect(),
                    terms: content.len(),
                    of_me: expanded && of_me,
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
        let tokens = query_tokens(query).0;
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

    /// Publish a learned-rule merge unless the rule changed since `expected`
    /// was read. Both sides are compared redacted: `expected` is the redacted
    /// text the merge started from. `Ok(None)`: it changed, nothing written;
    /// `Ok(Some(changed))`: published, `changed` when the file was rewritten.
    pub fn extraction_rule_expected(
        &self,
        name: &str,
        expected: &str,
        content: &str,
    ) -> Result<Option<bool>, MemoryError> {
        validate_name(name)?;
        let path = self.rules_dir().join(format!("{name}.md"));
        let current = match std::fs::read_to_string(&path) {
            Ok(current) => current,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(e.into()),
        };
        if crate::redact::redact(&current).text != expected {
            return Ok(None);
        }
        if content == expected {
            return Ok(Some(false));
        }
        self.set_rule(name, content)?;
        Ok(Some(true))
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

    #[test]
    fn authenticated_history_bytes_are_atomic_across_reopen_and_retry() {
        use crate::extraction_proof::{Attestation, Row, Terminal};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        let row = Row {
            source_kind: "direct".into(),
            attestation: Attestation {
                version: 1,
                session_id: "synthetic".into(),
                turn_id: "original".into(),
                entry_id: "entry".into(),
                text_sha256: "a".repeat(64),
                policy: "on".into(),
                bank: Some("private".into()),
                layer: None,
                agent: None,
                derived: false,
                terminal: Terminal {
                    status: "failed".into(),
                    exchange_finished: true,
                },
            },
        };
        store
            .save_authenticated_history("synthetic", std::slice::from_ref(&row))
            .unwrap();
        let key = serde_json::json!(["authenticated-history-v1", "synthetic"]).to_string();
        let path = store.extraction_plan_path(&key);
        let bytes = std::fs::read(&path).unwrap();
        let pending = serde_json::json!({"saved":"original pending"});
        store
            .set_pending_extraction("synthetic", pending.clone())
            .unwrap();
        let mut huge = row.clone();
        huge.attestation.turn_id = "retry".into();
        huge.attestation.bank = Some("b".repeat(17000));
        assert!(store
            .save_authenticated_history("synthetic", &[huge])
            .is_err());
        assert!(store
            .save_authenticated_history("synthetic", &[row.clone(), row.clone()])
            .is_err());
        let aggregate: Vec<_> = (0..1024)
            .map(|n| {
                let mut r = row.clone();
                r.attestation.turn_id = format!("{n:04}{}", "t".repeat(252));
                r.attestation.session_id = "s".repeat(256);
                r.attestation.entry_id = "e".repeat(256);
                r.attestation.bank = Some("b".repeat(256));
                r.attestation.layer = Some("l".repeat(256));
                r.attestation.agent = Some("a".repeat(256));
                r
            })
            .collect();
        assert!(store
            .save_authenticated_history("synthetic", &aggregate)
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        // Individually bounded deliveries cannot overflow a persisted union.
        let union_dir = tempfile::tempdir().unwrap();
        let union = Store::open(union_dir.path().into()).unwrap();
        union
            .save_authenticated_history("synthetic", &aggregate[..350])
            .unwrap();
        let union_path = union.extraction_plan_path(&key);
        let union_before = std::fs::read(&union_path).unwrap();
        assert!(union
            .save_authenticated_history("synthetic", &aggregate[350..700])
            .is_err());
        assert_eq!(std::fs::read(&union_path).unwrap(), union_before);
        union
            .save_authenticated_history("synthetic", std::slice::from_ref(&row))
            .unwrap();
        assert_eq!(union.authenticated_history("synthetic").unwrap().len(), 351);
        let reopened = Store::open(dir.path().into()).unwrap();
        assert_eq!(
            reopened.authenticated_history("synthetic").unwrap(),
            vec![row.clone()]
        );
        assert_eq!(
            reopened.pending_extraction("synthetic").unwrap(),
            Some(pending.clone())
        );
        // The exact raw-file limit remains readable: whitespace counts, but
        // does not change the authenticated complete JSON record.
        let mut exact = bytes.clone();
        exact.resize(crate::extraction_proof::RECORD_BYTES, b' ');
        std::fs::write(&path, &exact).unwrap();
        assert_eq!(
            Store::open(dir.path().into())
                .unwrap()
                .authenticated_history("synthetic")
                .unwrap(),
            vec![row.clone()]
        );
        // A small enough file can still have a forbidden per-row allocation.
        let mut malformed: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        malformed["plan"][0]["attestation"]["bank"] = serde_json::json!("x".repeat(17000));
        let malformed = serde_json::to_vec(&malformed).unwrap();
        std::fs::write(&path, &malformed).unwrap();
        let stale = Store::open(dir.path().into()).unwrap();
        assert!(stale.authenticated_history("synthetic").is_err());
        assert!(stale
            .save_authenticated_history("synthetic", std::slice::from_ref(&row))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), malformed);
        // On-disk oversized record must be refused before JSON decoding, even
        // when trailing whitespace could otherwise deserialize successfully.
        let mut oversized = bytes.clone();
        oversized.resize(crate::extraction_proof::RECORD_BYTES + 1, b' ');
        std::fs::write(&path, &oversized).unwrap();
        let reopened = Store::open(dir.path().into()).unwrap();
        assert!(reopened.authenticated_history("synthetic").is_err());
        assert!(reopened
            .save_authenticated_history("synthetic", std::slice::from_ref(&row))
            .is_err());
        assert_eq!(std::fs::read(&path).unwrap(), oversized);
        assert_eq!(
            reopened.pending_extraction("synthetic").unwrap(),
            Some(pending)
        );
        std::fs::write(&path, bytes).unwrap();
        let mut retry = row;
        retry.attestation.turn_id = "valid-retry".into();
        // A maximum permitted identifier is a real persisted boundary control.
        retry.attestation.entry_id = "e".repeat(256);
        reopened
            .save_authenticated_history("synthetic", std::slice::from_ref(&retry))
            .unwrap();
        let before = std::fs::read(&path).unwrap();
        reopened
            .save_authenticated_history("synthetic", &[retry])
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            Store::open(dir.path().into())
                .unwrap()
                .authenticated_history("synthetic")
                .unwrap()
                .len(),
            2
        );
    }

    #[test]
    fn an_acknowledged_range_takes_its_plan_and_history_rows_with_it() {
        use crate::extraction_proof::{Attestation, Row, Terminal};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        let row = |entry: &str| Row {
            source_kind: "direct".into(),
            attestation: Attestation {
                version: 1,
                session_id: "s".into(),
                turn_id: format!("turn-{entry}"),
                entry_id: entry.into(),
                text_sha256: "a".repeat(64),
                policy: "on".into(),
                bank: Some("private".into()),
                layer: None,
                agent: None,
                derived: false,
                terminal: Terminal {
                    status: "failed".into(),
                    exchange_finished: true,
                },
            },
        };
        store
            .save_authenticated_history("s", &[row("e1"), row("e2")])
            .unwrap();
        let entries = vec![("e1".into(), "user".into(), "source".into(), 1)];
        let range = PendingExtraction {
            version: 1,
            source_digest: Some(PendingExtraction::digest(&entries)),
            entries,
            bank: "private".into(),
            derived: false,
            agent: None,
            turn_id: Some("turn-e1".into()),
            attempts: 0,
        };
        let plan = range.source_key("s");
        store
            .save_extraction_plan(&plan, &serde_json::json!({"items":[]}))
            .unwrap();
        store
            .retain_extraction_range("s", range.clone(), false)
            .unwrap();
        assert!(store.extraction_plan(&plan).unwrap().is_some());
        store
            .retain_extraction_range("s", range.clone(), true)
            .unwrap();
        // Completed: never executed again, so its plan is gone already.
        assert_eq!(store.extraction_plan(&plan).unwrap(), None);
        store.acknowledge_extraction_range("s", &range).unwrap();
        assert!(store.extraction_ranges("s").unwrap().is_empty());
        let rows = store.authenticated_history("s").unwrap();
        assert_eq!(rows, [row("e2")], "rows at or before the cursor go");
        // Pruning the last row removes the ledger file.
        store
            .prune_authenticated_history("s", &HashSet::from(["e2"]))
            .unwrap();
        assert!(store.authenticated_history("s").unwrap().is_empty());
        assert!(!store.extraction_plan_path(&history_key("s")).exists());
    }

    #[test]
    fn extraction_public_identifiers_reject_atomically_before_keys_and_retry() {
        use serde_json::Value;
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().into()).unwrap();
        let entries = vec![("entry".into(), "user".into(), "source".into(), 1)];
        let pending = PendingExtraction {
            version: 1,
            source_digest: Some(PendingExtraction::digest(&entries)),
            entries,
            bank: "private".into(),
            derived: false,
            agent: None,
            turn_id: None,
            attempts: 0,
        };
        let session = "s".repeat(256);
        store
            .set_pending_extraction(&session, serde_json::to_value(&pending).unwrap())
            .unwrap();
        store
            .retain_extraction_range(&session, pending.clone(), false)
            .unwrap();
        let snapshot = || {
            let mut files = std::fs::read_dir(dir.path().join(".extraction"))
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    (path.clone(), std::fs::read(path).unwrap())
                })
                .collect::<Vec<_>>();
            files.sort();
            files
        };
        let before = snapshot();
        for invalid in ["s".repeat(257), String::new()] {
            assert!(store.pending_extraction(&invalid).is_err());
            assert!(store.extraction_ranges(&invalid).is_err());
            assert!(store.set_pending_extraction(&invalid, Value::Null).is_err());
            assert!(store
                .retain_extraction_range(&invalid, pending.clone(), true)
                .is_err());
            assert!(store
                .acknowledge_extraction_range(&invalid, &pending)
                .is_err());
            assert!(store.authenticated_history(&invalid).is_err());
            assert!(store.save_authenticated_history(&invalid, &[]).is_err());
            assert_eq!(snapshot(), before);
        }
        let oversized_source = "x".repeat(crate::extraction_proof::SOURCE_BYTES / 6 + 1);
        assert!(store.extraction_plan(&oversized_source).is_err());
        assert!(store
            .save_extraction_plan(&oversized_source, &Value::Null)
            .is_err());
        let mut bad = pending.clone();
        bad.entries[0].0 = "i".repeat(257);
        assert!(store
            .retain_extraction_range(&session, bad.clone(), true)
            .is_err());
        assert!(store.acknowledge_extraction_range(&session, &bad).is_err());
        assert!(store
            .set_pending_extraction(&session, serde_json::to_value(&bad).unwrap())
            .is_err());
        bad.entries[0].0 = "entry".into();
        bad.entries[0].2 = "x".repeat(crate::extraction_proof::TEXT_BYTES + 1);
        assert!(store.retain_extraction_range(&session, bad, true).is_err());
        assert_eq!(snapshot(), before);
        let mut aggregate = pending.clone();
        aggregate.entries = (0..9)
            .map(|n| {
                (
                    format!("entry-{n}"),
                    "user".into(),
                    "x".repeat(80 * 1024),
                    1,
                )
            })
            .collect();
        assert!(store
            .retain_extraction_range(&session, aggregate.clone(), true)
            .is_err());
        assert!(store
            .set_pending_extraction(&session, serde_json::to_value(aggregate).unwrap())
            .is_err());
        assert_eq!(snapshot(), before);
        let reopened = Store::open(dir.path().into()).unwrap();
        assert_eq!(
            reopened.pending_extraction(&session).unwrap(),
            Some(serde_json::to_value(&pending).unwrap())
        );
        reopened
            .retain_extraction_range(&session, pending.clone(), true)
            .unwrap();
        reopened
            .acknowledge_extraction_range(&session, &pending)
            .unwrap();
        assert!(reopened.extraction_ranges(&session).unwrap().is_empty());
    }

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
    fn query_tokens_drop_stopwords_but_never_empty_the_query() {
        assert_eq!(
            query_tokens("what is my dog's name").0,
            ["dog", "name", "user"]
        );
        // `.1` says the pronoun added `user`; a typed one is no expansion.
        let (tokens, expanded) = query_tokens("which OS do I use");
        assert_eq!(tokens, ["os", "use", "user"]);
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
        store.trash_bank("blog-old", "main").await.unwrap();
        assert!(store.is_deleted("blog-old"));
        assert!(
            !store.is_deleted("blog"),
            "another bank's trash is not this one's"
        );
        assert!(
            !store.is_deleted("never"),
            "a bank that never existed is not deleted"
        );
        store.trash_bank("blog", "main").await.unwrap();
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
        let dest = store.trash_bank("scratch", "main").await.unwrap();
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
    async fn a_bank_too_small_for_z_still_injects_on_meaning() {
        // A new bank: five memories, each question close in meaning to one,
        // sharing one word with it or none.
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
                let picked = crate::hooks::select(hits, &ev, &std::collections::HashSet::new(), 6);
                let picked: Vec<&str> = picked.iter().map(|(m, _)| m.text.as_str()).collect();
                assert!(
                    picked.contains(&texts[target].text.as_str()),
                    "{model} {question}: {picked:?}"
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
