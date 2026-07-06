//! The coder write journal: bounded before-images of every mutating coder
//! call, keyed by effective root — the substrate for `coder::undo` and
//! `coder::checkpoints`. Recording is best-effort and NEVER fails the write
//! that triggered it (a journal error logs and the edit proceeds); restoring
//! re-validates every path through the live jail, never trusting stored
//! paths.
//!
//! Layout: `<journal.dir>/<root_key>/<seq>.json` with `<seq>-<i>.blob`
//! sidecars for before-images (raw bytes — no encoding, binary-safe).
//! `root_key` is an FNV-1a hash of the canonical root plus its last path
//! component for human readability. Oldest-first eviction keeps each root
//! under `journal.max_bytes` / `journal.max_records`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde::{Deserialize, Serialize};

use crate::code::config::CoderConfig;
use crate::fs::FsScope;

/// One file's before-state inside a record.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    /// Canonical absolute path at record time.
    pub path: String,
    /// Sidecar blob file name holding the before-image; `None` when the
    /// file did not exist before the write (undo removes it).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub blob: Option<String>,
    /// Before-image byte length (0 for absent files).
    pub before_bytes: u64,
    /// True when the before-image was too large to journal — undo cannot
    /// restore this file and reports the gap.
    #[serde(default)]
    pub skipped: bool,
}

/// One journaled mutation (a single coder call's write set).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalRecord {
    pub seq: u64,
    pub ts: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn_id: Option<String>,
    pub function_id: String,
    pub entries: Vec<JournalEntry>,
}

/// Input to [`record`]: a file about to be (or just) mutated and its
/// pre-write content (`None` = did not exist).
pub struct EntryInput {
    pub path: PathBuf,
    pub before: Option<Vec<u8>>,
    /// Force-mark unrecoverable (e.g. a directory delete/move whose tree
    /// cannot be snapshotted) — undo reports it as a gap.
    pub skipped: bool,
}

/// Journaling disabled?
pub fn enabled(cfg: &CoderConfig) -> bool {
    cfg.journal.max_records > 0
}

/// Record one mutation's before-images. Best-effort: any failure logs a
/// warning and returns — the caller's write is never blocked.
pub fn record(
    cfg: &CoderConfig,
    root: &Path,
    scope: Option<&FsScope>,
    function_id: &str,
    inputs: Vec<EntryInput>,
) {
    if !enabled(cfg) || inputs.is_empty() {
        return;
    }
    if let Err(e) = record_inner(cfg, root, scope, function_id, inputs) {
        tracing::warn!(function_id, error = %e, "coder journal record failed (write unaffected)");
    }
}

fn record_inner(
    cfg: &CoderConfig,
    root: &Path,
    scope: Option<&FsScope>,
    function_id: &str,
    inputs: Vec<EntryInput>,
) -> std::io::Result<()> {
    let dir = root_dir(cfg, root);
    std::fs::create_dir_all(&dir)?;
    let seq = next_seq(&dir);

    let mut entries = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.into_iter().enumerate() {
        let (blob, before_bytes, skipped) = match (input.skipped, input.before) {
            (true, before) => (None, before.map(|b| b.len() as u64).unwrap_or(0), true),
            (false, None) => (None, 0, false),
            (false, Some(bytes)) if (bytes.len() as u64) > cfg.max_write_bytes => {
                (None, bytes.len() as u64, true)
            }
            (false, Some(bytes)) => {
                let name = format!("{seq:08}-{i}.blob");
                std::fs::write(dir.join(&name), &bytes)?;
                (Some(name), bytes.len() as u64, false)
            }
        };
        entries.push(JournalEntry {
            path: input.path.display().to_string(),
            blob,
            before_bytes,
            skipped,
        });
    }

    let record = JournalRecord {
        seq,
        ts: now_ms(),
        session_id: scope.and_then(|s| s.session_id.clone()),
        turn_id: scope.and_then(|s| s.turn_id.clone()),
        function_id: function_id.to_string(),
        entries,
    };
    let json = serde_json::to_vec(&record)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(dir.join(format!("{seq:08}.json")), json)?;
    evict(cfg, &dir);
    Ok(())
}

/// All records for a root, ascending seq. Unreadable records are skipped.
pub fn list(cfg: &CoderConfig, root: &Path) -> Vec<JournalRecord> {
    let dir = root_dir(cfg, root);
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out: Vec<JournalRecord> = rd
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| {
            std::fs::read(e.path())
                .ok()
                .and_then(|b| serde_json::from_slice(&b).ok())
        })
        .collect();
    out.sort_by_key(|r: &JournalRecord| r.seq);
    out
}

/// Read one entry's before-image blob.
pub fn read_blob(cfg: &CoderConfig, root: &Path, blob: &str) -> std::io::Result<Vec<u8>> {
    // Blob names are journal-generated (`{seq}-{i}.blob`); reject anything
    // path-like so a tampered record cannot read outside the journal dir.
    if blob.contains('/') || blob.contains('\\') || blob.contains("..") {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "invalid blob name",
        ));
    }
    std::fs::read(root_dir(cfg, root).join(blob))
}

/// Delete a record and its blobs (after a successful undo of it, the undo
/// itself having journaled the pre-undo state as a NEW record).
pub fn remove_record(cfg: &CoderConfig, root: &Path, record: &JournalRecord) {
    let dir = root_dir(cfg, root);
    for e in &record.entries {
        if let Some(blob) = &e.blob {
            let _ = std::fs::remove_file(dir.join(blob));
        }
    }
    let _ = std::fs::remove_file(dir.join(format!("{:08}.json", record.seq)));
}

fn evict(cfg: &CoderConfig, dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    let mut json_files: Vec<(u64, PathBuf, u64)> = Vec::new(); // (seq, path, bytes incl. blobs)
    let mut blob_sizes: HashMap<u64, u64> = HashMap::new();
    let mut blob_paths: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    for e in rd.flatten() {
        let p = e.path();
        let size = e.metadata().map(|m| m.len()).unwrap_or(0);
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if let Some(stem) = name.strip_suffix(".json") {
            if let Ok(seq) = stem.parse::<u64>() {
                json_files.push((seq, p.clone(), size));
            }
        } else if let Some(stem) = name.strip_suffix(".blob") {
            if let Some((seq_part, _)) = stem.split_once('-') {
                if let Ok(seq) = seq_part.parse::<u64>() {
                    *blob_sizes.entry(seq).or_default() += size;
                    blob_paths.entry(seq).or_default().push(p.clone());
                }
            }
        }
    }
    json_files.sort_by_key(|(seq, _, _)| *seq);
    let mut total: u64 = json_files
        .iter()
        .map(|(seq, _, sz)| sz + blob_sizes.get(seq).copied().unwrap_or(0))
        .sum();
    let mut count = json_files.len() as u32;
    for (seq, path, sz) in &json_files {
        let over_bytes = total > cfg.journal.max_bytes;
        let over_count = count > cfg.journal.max_records;
        if !over_bytes && !over_count {
            break;
        }
        let _ = std::fs::remove_file(path);
        for bp in blob_paths.get(seq).into_iter().flatten() {
            let _ = std::fs::remove_file(bp);
        }
        total = total.saturating_sub(sz + blob_sizes.get(seq).copied().unwrap_or(0));
        count -= 1;
    }
}

/// Per-root journal directory: `<dir>/<fnv(root)>-<last component>`.
fn root_dir(cfg: &CoderConfig, root: &Path) -> PathBuf {
    let canon = root.to_string_lossy();
    let tail: String = root
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
        .take(32)
        .collect();
    PathBuf::from(&cfg.journal.dir).join(format!("{:016x}-{tail}", fnv1a(canon.as_bytes())))
}

/// Stable across processes and Rust versions (unlike DefaultHasher).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Next sequence number for a root dir: max on disk + 1 at first use, then
/// a process-local counter (the shell worker is the only writer).
fn next_seq(dir: &Path) -> u64 {
    static COUNTERS: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();
    let counters = COUNTERS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut map = counters.lock().unwrap_or_else(|p| p.into_inner());
    let next = map
        .entry(dir.to_path_buf())
        .or_insert_with(|| max_seq_on_disk(dir));
    *next += 1;
    *next
}

fn max_seq_on_disk(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    e.path()
                        .file_name()?
                        .to_str()?
                        .strip_suffix(".json")?
                        .parse::<u64>()
                        .ok()
                })
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn cfg_with(dir: &Path, max_records: u32, max_bytes: u64) -> CoderConfig {
        let mut c = CoderConfig::default();
        c.journal.dir = dir.display().to_string();
        c.journal.max_records = max_records;
        c.journal.max_bytes = max_bytes;
        c
    }

    fn entry(path: &str, before: Option<&[u8]>) -> EntryInput {
        EntryInput {
            path: PathBuf::from(path),
            before: before.map(|b| b.to_vec()),
            skipped: false,
        }
    }

    #[test]
    fn record_and_list_round_trip() {
        let jd = tempdir().unwrap();
        let root = tempdir().unwrap();
        let cfg = cfg_with(jd.path(), 100, 1 << 20);
        record(
            &cfg,
            root.path(),
            None,
            "coder::update-file",
            vec![entry("/w/a.rs", Some(b"old contents"))],
        );
        record(
            &cfg,
            root.path(),
            None,
            "coder::create-file",
            vec![entry("/w/new.rs", None)],
        );
        let records = list(&cfg, root.path());
        assert_eq!(records.len(), 2);
        assert_eq!(records[0].function_id, "coder::update-file");
        assert!(records[0].seq < records[1].seq);
        let blob = records[0].entries[0].blob.as_ref().unwrap();
        assert_eq!(read_blob(&cfg, root.path(), blob).unwrap(), b"old contents");
        assert!(
            records[1].entries[0].blob.is_none(),
            "absent before = no blob"
        );
    }

    #[test]
    fn scope_stamps_ride_the_record() {
        let jd = tempdir().unwrap();
        let root = tempdir().unwrap();
        let cfg = cfg_with(jd.path(), 100, 1 << 20);
        let scope = FsScope {
            root: root.path().display().to_string(),
            grants: vec![],
            session_id: Some("s-1".into()),
            turn_id: Some("t-1".into()),
        };
        record(
            &cfg,
            root.path(),
            Some(&scope),
            "coder::apply-patch",
            vec![entry("/w/a.rs", Some(b"x"))],
        );
        let r = &list(&cfg, root.path())[0];
        assert_eq!(r.session_id.as_deref(), Some("s-1"));
        assert_eq!(r.turn_id.as_deref(), Some("t-1"));
    }

    #[test]
    fn oversized_before_image_is_skipped_honestly() {
        let jd = tempdir().unwrap();
        let root = tempdir().unwrap();
        let mut cfg = cfg_with(jd.path(), 100, 1 << 20);
        cfg.max_write_bytes = 4;
        record(
            &cfg,
            root.path(),
            None,
            "coder::delete-file",
            vec![entry("/w/big.bin", Some(b"way too large"))],
        );
        let r = &list(&cfg, root.path())[0];
        assert!(r.entries[0].skipped);
        assert!(r.entries[0].blob.is_none());
        assert_eq!(r.entries[0].before_bytes, 13);
    }

    #[test]
    fn record_count_eviction_drops_oldest() {
        let jd = tempdir().unwrap();
        let root = tempdir().unwrap();
        let cfg = cfg_with(jd.path(), 3, 1 << 20);
        for i in 0..5u8 {
            record(
                &cfg,
                root.path(),
                None,
                "coder::update-file",
                vec![entry(&format!("/w/{i}.rs"), Some(&[i]))],
            );
        }
        let records = list(&cfg, root.path());
        assert_eq!(records.len(), 3, "oldest evicted");
        assert!(records[0].entries[0].path.ends_with("2.rs"));
    }

    #[test]
    fn zero_max_records_disables_journaling() {
        let jd = tempdir().unwrap();
        let root = tempdir().unwrap();
        let cfg = cfg_with(jd.path(), 0, 1 << 20);
        record(
            &cfg,
            root.path(),
            None,
            "coder::update-file",
            vec![entry("/w/a.rs", Some(b"x"))],
        );
        assert!(list(&cfg, root.path()).is_empty());
    }

    #[test]
    fn blob_names_reject_path_traversal() {
        let jd = tempdir().unwrap();
        let root = tempdir().unwrap();
        let cfg = cfg_with(jd.path(), 100, 1 << 20);
        assert!(read_blob(&cfg, root.path(), "../../etc/passwd").is_err());
    }

    #[test]
    fn distinct_roots_do_not_share_journals() {
        let jd = tempdir().unwrap();
        let root_a = tempdir().unwrap();
        let root_b = tempdir().unwrap();
        let cfg = cfg_with(jd.path(), 100, 1 << 20);
        record(
            &cfg,
            root_a.path(),
            None,
            "coder::update-file",
            vec![entry("/w/a.rs", Some(b"a"))],
        );
        assert_eq!(list(&cfg, root_a.path()).len(), 1);
        assert!(list(&cfg, root_b.path()).is_empty());
    }
}
