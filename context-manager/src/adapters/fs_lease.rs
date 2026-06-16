//! Filesystem-backed compaction-lease storage, mirroring
//! `session-manager`'s `FsStore` strategy (`session-manager/src/store/fs.rs`):
//! one file per lease key under `<lease_dir>/<scope>/`, a process-local
//! `Mutex<HashMap>` cache, and atomic `tmp + rename` writes.
//!
//! Atomicity is **process-local** — the same single-writer-per-dir
//! assumption session-manager makes — which is sufficient for a single
//! context-manager instance. `swap` (the atomic read-modify-write
//! `core::lease` relies on) holds the cache mutex across the whole
//! read/decide/write, so two concurrent acquirers in this process
//! serialize: exactly one sees a free/expired prior and wins.
//!
//! Each lease file holds exactly one `{nonce, ts}` value, so every write
//! is a full O(1) rewrite (session-manager's `persist_snapshot`
//! mechanism); there is no append/replay-merge to do. A missing file —
//! or a file whose contents aren't a `{nonce, ts}` record — reads as free
//! (`None`), matching the state-backed store's `parse_record` and
//! session-manager's malformed-skip posture.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;

use async_trait::async_trait;

use crate::ports::{LeaseRecord, LeaseStore};

/// Encode a lease key into a safe filename stem: `[A-Za-z0-9._-]` pass
/// through, everything else becomes `%XX` (uppercase hex). Keys are
/// caller-supplied (`options.lease_key`, e.g. a session id) or a sha256
/// hex digest, so the encoding both keeps filenames portable and blocks
/// path traversal. Ported from session-manager's `encode_session_id`.
fn encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for byte in key.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Inverse of [`encode_key`]; `None` for malformed escapes. Only the
/// encode direction is needed in production (keys never round-trip back
/// from filenames), so this is the exact session-manager inverse kept for
/// the encoding-safety test.
#[cfg(test)]
fn decode_key(stem: &str) -> Option<String> {
    let bytes = stem.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = bytes.get(i + 1..i + 3)?;
            let hex = std::str::from_utf8(hex).ok()?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// `(scope, key) -> Option<LeaseRecord>` cache. A present `None` means
/// "loaded and free"; an absent entry means "not yet read from disk"
/// (cold), mirroring session-manager's `contains_key` cold check.
type Cache = HashMap<(String, String), Option<LeaseRecord>>;

pub struct FsLeaseStore {
    dir: PathBuf,
    cache: Mutex<Cache>,
}

impl FsLeaseStore {
    /// Open (and create if needed) the lease directory.
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, String> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("create lease_dir {}: {e}", dir.display()))?;
        Ok(Self {
            dir,
            cache: Mutex::new(HashMap::new()),
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn scope_dir(&self, scope: &str) -> PathBuf {
        self.dir.join(encode_key(scope))
    }

    fn file_path(&self, scope: &str, key: &str) -> PathBuf {
        self.scope_dir(scope)
            .join(format!("{}.json", encode_key(key)))
    }

    /// Read the on-disk record for a key. Missing file -> `None` (free);
    /// contents that don't parse as a `{nonce, ts}` record -> `None` too.
    fn read_file(&self, scope: &str, key: &str) -> Result<Option<LeaseRecord>, String> {
        let path = self.file_path(scope, key);
        match std::fs::read_to_string(&path) {
            Ok(contents) => Ok(serde_json::from_str::<LeaseRecord>(&contents).ok()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(format!("read {}: {e}", path.display())),
        }
    }

    /// Atomically write one record: tmp + rename, creating the scope dir
    /// on demand. Mirrors session-manager's `persist_snapshot`.
    fn write_file(&self, scope: &str, key: &str, record: &LeaseRecord) -> Result<(), String> {
        let dir = self.scope_dir(scope);
        std::fs::create_dir_all(&dir)
            .map_err(|e| format!("create lease scope dir {}: {e}", dir.display()))?;
        let path = self.file_path(scope, key);
        let body =
            serde_json::to_string(record).map_err(|e| format!("serialize lease record: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, body).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path)
            .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), path.display()))?;
        Ok(())
    }

    /// Remove a key's file. A missing file is a no-op.
    fn remove_file(&self, scope: &str, key: &str) -> Result<(), String> {
        let path = self.file_path(scope, key);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", path.display())),
        }
    }

    /// Populate the cache entry for `(scope, key)` from disk on a cold
    /// miss. Must be called with `cache` already locked.
    fn ensure_loaded(&self, cache: &mut Cache, scope: &str, key: &str) -> Result<(), String> {
        use std::collections::hash_map::Entry;
        if let Entry::Vacant(slot) = cache.entry((scope.to_string(), key.to_string())) {
            let record = self.read_file(scope, key)?;
            slot.insert(record);
        }
        Ok(())
    }
}

#[async_trait]
impl LeaseStore for FsLeaseStore {
    async fn get(&self, scope: &str, key: &str) -> Result<Option<LeaseRecord>, String> {
        let mut cache = self.lock();
        self.ensure_loaded(&mut cache, scope, key)?;
        Ok(cache
            .get(&(scope.to_string(), key.to_string()))
            .cloned()
            .flatten())
    }

    async fn set(&self, scope: &str, key: &str, value: Option<LeaseRecord>) -> Result<(), String> {
        let mut cache = self.lock();
        let k = (scope.to_string(), key.to_string());
        // Disk first, cache second: a failed write must never leave the
        // cache claiming state the file doesn't have.
        match value {
            Some(record) => {
                self.write_file(scope, key, &record)?;
                cache.insert(k, Some(record));
            }
            None => {
                self.remove_file(scope, key)?;
                cache.insert(k, None);
            }
        }
        Ok(())
    }

    async fn swap(
        &self,
        scope: &str,
        key: &str,
        value: LeaseRecord,
    ) -> Result<Option<LeaseRecord>, String> {
        // The whole read-modify-write runs under the cache mutex, so two
        // concurrent acquirers serialize and exactly one observes the
        // free/expired prior. No `.await` is taken while the guard is
        // held (all I/O below is synchronous), so the future stays Send.
        let mut cache = self.lock();
        self.ensure_loaded(&mut cache, scope, key)?;
        let k = (scope.to_string(), key.to_string());
        let prior = cache.get(&k).cloned().flatten();
        self.write_file(scope, key, &value)?;
        cache.insert(k, Some(value));
        Ok(prior)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCOPE: &str = "context_lease";

    fn rec(nonce: &str, ts: i64) -> LeaseRecord {
        LeaseRecord {
            nonce: nonce.into(),
            ts,
        }
    }

    #[tokio::test]
    async fn set_get_roundtrip_and_restart_replay() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = FsLeaseStore::new(dir.path()).unwrap();
            store.set(SCOPE, "k1", Some(rec("n1", 1000))).await.unwrap();
            assert_eq!(store.get(SCOPE, "k1").await.unwrap(), Some(rec("n1", 1000)));
        }
        // Fresh store over the same dir = worker restart: cache is cold,
        // so the value must replay from disk.
        let store = FsLeaseStore::new(dir.path()).unwrap();
        assert_eq!(store.get(SCOPE, "k1").await.unwrap(), Some(rec("n1", 1000)));
    }

    #[tokio::test]
    async fn set_none_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsLeaseStore::new(dir.path()).unwrap();
        store.set(SCOPE, "k1", Some(rec("n1", 1))).await.unwrap();
        let path = store.file_path(SCOPE, "k1");
        assert!(path.exists());

        store.set(SCOPE, "k1", None).await.unwrap();
        assert!(!path.exists());
        assert_eq!(store.get(SCOPE, "k1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn swap_returns_prior_and_stores_new() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsLeaseStore::new(dir.path()).unwrap();

        // First swap on a free key: prior is None.
        assert_eq!(store.swap(SCOPE, "k1", rec("n1", 1)).await.unwrap(), None);
        // Second swap: prior is the first claim, new is stored.
        assert_eq!(
            store.swap(SCOPE, "k1", rec("n2", 2)).await.unwrap(),
            Some(rec("n1", 1))
        );
        assert_eq!(store.get(SCOPE, "k1").await.unwrap(), Some(rec("n2", 2)));
    }

    #[tokio::test]
    async fn cold_swap_sees_the_on_disk_prior() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = FsLeaseStore::new(dir.path()).unwrap();
            store.set(SCOPE, "k1", Some(rec("n1", 1))).await.unwrap();
        }
        // A fresh store (cold cache) must read the prior from disk so a
        // racing acquirer after restart can't be treated as a free win.
        let store = FsLeaseStore::new(dir.path()).unwrap();
        assert_eq!(
            store.swap(SCOPE, "k1", rec("n2", 2)).await.unwrap(),
            Some(rec("n1", 1))
        );
    }

    #[test]
    fn key_encoding_roundtrip_and_safety() {
        let hostile = "../etc/passwd: weird key?";
        let encoded = encode_key(hostile);
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains(' '));
        assert!(!encoded.contains(':'));
        assert_eq!(decode_key(&encoded).as_deref(), Some(hostile));

        // sha256-hex / session-id-style keys pass through unchanged.
        assert_eq!(encode_key("s_abc-123.x"), "s_abc-123.x");
        // Malformed escapes are rejected, not mangled.
        assert_eq!(decode_key("%ZZ"), None);
        assert_eq!(decode_key("%4"), None);
    }

    #[tokio::test]
    async fn hostile_key_file_stays_inside_dir() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsLeaseStore::new(dir.path()).unwrap();
        let key = "../escape/чат 42";
        store.set(SCOPE, key, Some(rec("n", 1))).await.unwrap();

        // Exactly one file landed under the scope dir, encoded — no
        // traversal out of `lease_dir`.
        let scope_dir = store.scope_dir(SCOPE);
        let files: Vec<_> = std::fs::read_dir(&scope_dir).unwrap().collect();
        assert_eq!(files.len(), 1);
        assert_eq!(store.get(SCOPE, key).await.unwrap(), Some(rec("n", 1)));
    }

    #[tokio::test]
    async fn malformed_or_foreign_value_reads_as_free() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsLeaseStore::new(dir.path()).unwrap();

        // A stored value that isn't a `{nonce, ts}` record reads as free.
        let path = store.file_path(SCOPE, "k1");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "\"just a string\"").unwrap();
        assert_eq!(store.get(SCOPE, "k1").await.unwrap(), None);
    }

    #[tokio::test]
    async fn unknown_key_reads_as_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsLeaseStore::new(dir.path()).unwrap();
        assert_eq!(store.get(SCOPE, "nope").await.unwrap(), None);
    }
}
