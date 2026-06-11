//! Filesystem backend: one append-only JSONL file per session.
//!
//! Layout: `<data_dir>/<encoded_session_id>.jsonl`, where the encoding
//! passes `[A-Za-z0-9._-]` through and percent-encodes every other
//! byte (session ids are caller-supplied via `session::ensure`, so the
//! encoding both keeps filenames portable and blocks path traversal).
//!
//! Every record is one line, discriminated by `type`:
//!
//! ```json
//! {"type":"meta","meta":{ ...SessionMeta }}
//! {"type":"entry","entry":{ ...SessionEntry }}
//! {"type":"leaf","entry_id":"e_..."}
//! ```
//!
//! Mutations append (meta rewrites, entry writes/updates, leaf moves);
//! replay is last-wins per key, so the newest meta / entry revision /
//! leaf pointer is authoritative. Deleting a session removes its file.
//!
//! A lazy per-session cache makes reads cheap: the file is replayed on
//! first access and kept write-through afterwards. This is safe because
//! the service serializes mutations per session and this worker is the
//! single writer of its data_dir. A truncated trailing line (crash
//! mid-append) is tolerated with a warning; malformed lines are
//! warn-and-skipped.

use std::collections::{BTreeMap, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::{SessionStore, StoreError};
use crate::types::{SessionEntry, SessionMeta};

/// One JSONL line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Record {
    Meta { meta: SessionMeta },
    Entry { entry: SessionEntry },
    Leaf { entry_id: Option<String> },
}

/// Materialized state of one session file.
#[derive(Debug, Default, Clone)]
struct LoadedSession {
    meta: Option<SessionMeta>,
    entries: BTreeMap<String, SessionEntry>,
    leaf: Option<String>,
}

impl LoadedSession {
    fn is_empty(&self) -> bool {
        self.meta.is_none() && self.entries.is_empty() && self.leaf.is_none()
    }

    fn apply(&mut self, record: Record) {
        match record {
            Record::Meta { meta } => self.meta = Some(meta),
            Record::Entry { entry } => {
                self.entries.insert(entry.id().to_string(), entry);
            }
            Record::Leaf { entry_id } => self.leaf = entry_id,
        }
    }

    /// All live records, in a stable replayable order.
    fn snapshot_records(&self) -> Vec<Record> {
        let mut records = Vec::with_capacity(self.entries.len() + 2);
        if let Some(meta) = &self.meta {
            records.push(Record::Meta { meta: meta.clone() });
        }
        for entry in self.entries.values() {
            records.push(Record::Entry {
                entry: entry.clone(),
            });
        }
        if self.leaf.is_some() {
            records.push(Record::Leaf {
                entry_id: self.leaf.clone(),
            });
        }
        records
    }
}

/// Encode a session id into a safe filename stem: `[A-Za-z0-9._-]`
/// pass through, everything else becomes `%XX` (uppercase hex).
pub fn encode_session_id(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for byte in id.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'.' | b'_' | b'-' => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

/// Inverse of [`encode_session_id`]. `None` for malformed escapes.
pub fn decode_session_id(stem: &str) -> Option<String> {
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

pub struct FsStore {
    dir: PathBuf,
    cache: Mutex<HashMap<String, LoadedSession>>,
}

impl FsStore {
    /// Open (and create if needed) the data directory.
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, StoreError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir)
            .map_err(|e| StoreError(format!("create data_dir {}: {e}", dir.display())))?;
        Ok(Self {
            dir,
            cache: Mutex::new(HashMap::new()),
        })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file_path(&self, session_id: &str) -> PathBuf {
        self.dir
            .join(format!("{}.jsonl", encode_session_id(session_id)))
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, LoadedSession>> {
        self.cache
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    /// Replay a session file into a [`LoadedSession`]. Missing file ->
    /// empty state. Malformed lines (incl. a truncated trailing line
    /// from a crash mid-append) are skipped with a warning.
    fn replay_file(&self, session_id: &str) -> Result<LoadedSession, StoreError> {
        let path = self.file_path(session_id);
        let contents = match std::fs::read_to_string(&path) {
            Ok(c) => c,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LoadedSession::default())
            }
            Err(e) => return Err(StoreError(format!("read {}: {e}", path.display()))),
        };

        let mut loaded = LoadedSession::default();
        for (idx, line) in contents.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Record>(line) {
                Ok(record) => loaded.apply(record),
                Err(e) => tracing::warn!(
                    session_id,
                    line = idx + 1,
                    error = %e,
                    "skipping malformed session record (possibly a truncated tail)"
                ),
            }
        }
        Ok(loaded)
    }

    /// Run `f` against the loaded (cached) state of a session,
    /// replaying the file on first access.
    fn with_loaded<T>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut LoadedSession) -> T,
    ) -> Result<T, StoreError> {
        let mut cache = self.lock();
        if !cache.contains_key(session_id) {
            let loaded = self.replay_file(session_id)?;
            cache.insert(session_id.to_string(), loaded);
        }
        let loaded = cache
            .get_mut(session_id)
            .expect("session state inserted just above");
        Ok(f(loaded))
    }

    /// Append one record to the session's file (write-through is the
    /// caller's job via [`Self::with_loaded`]).
    fn append_record(&self, session_id: &str, record: &Record) -> Result<(), StoreError> {
        let path = self.file_path(session_id);
        let mut line = serde_json::to_string(record)
            .map_err(|e| StoreError(format!("serialize session record: {e}")))?;
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .map_err(|e| StoreError(format!("open {}: {e}", path.display())))?;
        file.write_all(line.as_bytes())
            .map_err(|e| StoreError(format!("append to {}: {e}", path.display())))?;
        Ok(())
    }

    /// Persist a full snapshot: rewrite the file atomically (tmp +
    /// rename), or remove it when the session state is empty. Used by
    /// the delete paths; appends never rewrite.
    fn persist_snapshot(&self, session_id: &str, loaded: &LoadedSession) -> Result<(), StoreError> {
        let path = self.file_path(session_id);
        if loaded.is_empty() {
            return match std::fs::remove_file(&path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(StoreError(format!("remove {}: {e}", path.display()))),
            };
        }

        let mut body = String::new();
        for record in loaded.snapshot_records() {
            let line = serde_json::to_string(&record)
                .map_err(|e| StoreError(format!("serialize session record: {e}")))?;
            body.push_str(&line);
            body.push('\n');
        }
        let tmp = path.with_extension("jsonl.tmp");
        std::fs::write(&tmp, body)
            .map_err(|e| StoreError(format!("write {}: {e}", tmp.display())))?;
        std::fs::rename(&tmp, &path).map_err(|e| {
            StoreError(format!(
                "rename {} -> {}: {e}",
                tmp.display(),
                path.display()
            ))
        })?;
        Ok(())
    }

    /// Session ids present on disk (decoded from filenames).
    fn session_ids_on_disk(&self) -> Result<Vec<String>, StoreError> {
        let mut ids = Vec::new();
        let dir = std::fs::read_dir(&self.dir)
            .map_err(|e| StoreError(format!("read data_dir {}: {e}", self.dir.display())))?;
        for dent in dir {
            let dent = dent.map_err(|e| StoreError(format!("read data_dir entry: {e}")))?;
            let name = dent.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".jsonl") else {
                continue;
            };
            match decode_session_id(stem) {
                Some(id) => ids.push(id),
                None => tracing::warn!(file = name, "skipping undecodable session file name"),
            }
        }
        Ok(ids)
    }
}

#[async_trait]
impl SessionStore for FsStore {
    async fn get_meta(&self, session_id: &str) -> Result<Option<SessionMeta>, StoreError> {
        self.with_loaded(session_id, |s| s.meta.clone())
    }

    async fn put_meta(&self, meta: &SessionMeta) -> Result<(), StoreError> {
        let record = Record::Meta { meta: meta.clone() };
        self.with_loaded(&meta.session_id, |s| s.meta = Some(meta.clone()))?;
        self.append_record(&meta.session_id, &record)
    }

    async fn delete_meta(&self, session_id: &str) -> Result<(), StoreError> {
        let snapshot = self.with_loaded(session_id, |s| {
            s.meta = None;
            s.clone()
        })?;
        if snapshot.is_empty() {
            self.lock().remove(session_id);
        }
        self.persist_snapshot(session_id, &snapshot)
    }

    async fn list_metas(&self) -> Result<Vec<SessionMeta>, StoreError> {
        let mut metas = Vec::new();
        for session_id in self.session_ids_on_disk()? {
            if let Some(meta) = self.with_loaded(&session_id, |s| s.meta.clone())? {
                metas.push(meta);
            }
        }
        Ok(metas)
    }

    async fn get_entry(
        &self,
        session_id: &str,
        entry_id: &str,
    ) -> Result<Option<SessionEntry>, StoreError> {
        self.with_loaded(session_id, |s| s.entries.get(entry_id).cloned())
    }

    async fn put_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<(), StoreError> {
        let record = Record::Entry {
            entry: entry.clone(),
        };
        self.with_loaded(session_id, |s| {
            s.entries.insert(entry.id().to_string(), entry.clone());
        })?;
        self.append_record(session_id, &record)
    }

    async fn list_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, StoreError> {
        self.with_loaded(session_id, |s| s.entries.values().cloned().collect())
    }

    async fn delete_entries(&self, session_id: &str) -> Result<(), StoreError> {
        let snapshot = self.with_loaded(session_id, |s| {
            s.entries.clear();
            s.clone()
        })?;
        if snapshot.is_empty() {
            self.lock().remove(session_id);
        }
        self.persist_snapshot(session_id, &snapshot)
    }

    async fn get_active_leaf(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        self.with_loaded(session_id, |s| s.leaf.clone())
    }

    async fn set_active_leaf(&self, session_id: &str, entry_id: &str) -> Result<(), StoreError> {
        let record = Record::Leaf {
            entry_id: Some(entry_id.to_string()),
        };
        self.with_loaded(session_id, |s| s.leaf = Some(entry_id.to_string()))?;
        self.append_record(session_id, &record)
    }

    async fn delete_active_leaf(&self, session_id: &str) -> Result<(), StoreError> {
        let snapshot = self.with_loaded(session_id, |s| {
            s.leaf = None;
            s.clone()
        })?;
        if snapshot.is_empty() {
            self.lock().remove(session_id);
        }
        self.persist_snapshot(session_id, &snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{AgentMessage, ContentBlock, SessionStatus};

    fn meta(session_id: &str, title: &str) -> SessionMeta {
        SessionMeta {
            session_id: session_id.into(),
            title: title.into(),
            description: String::new(),
            status: SessionStatus::Idle,
            status_reason: None,
            metadata: None,
            forked_from: None,
            created_at: 1,
            updated_at: 1,
            message_count: 0,
        }
    }

    fn entry(id: &str, parent: Option<&str>, text: &str, revision: u64) -> SessionEntry {
        SessionEntry::Message {
            id: id.into(),
            parent_id: parent.map(str::to_string),
            timestamp: 1,
            revision,
            origin: None,
            message: AgentMessage::User {
                content: vec![ContentBlock::Text { text: text.into() }],
                timestamp: 1,
            },
        }
    }

    #[test]
    fn session_id_encoding_roundtrip_and_safety() {
        let hostile = "../etc/passwd: weird id?";
        let encoded = encode_session_id(hostile);
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains(' '));
        assert!(!encoded.contains(':'));
        assert_eq!(decode_session_id(&encoded).as_deref(), Some(hostile));

        // Safe ids pass through unchanged.
        assert_eq!(encode_session_id("s_abc-123.x"), "s_abc-123.x");
        // Malformed escapes are rejected, not mangled.
        assert_eq!(decode_session_id("%ZZ"), None);
        assert_eq!(decode_session_id("%4"), None);
    }

    #[tokio::test]
    async fn roundtrip_and_restart_replay() {
        let dir = tempfile::tempdir().unwrap();

        {
            let store = FsStore::new(dir.path()).unwrap();
            store.put_meta(&meta("s_1", "first")).await.unwrap();
            store
                .put_entry("s_1", &entry("e_1", None, "one", 0))
                .await
                .unwrap();
            store
                .put_entry("s_1", &entry("e_2", Some("e_1"), "two", 0))
                .await
                .unwrap();
            store.set_active_leaf("s_1", "e_2").await.unwrap();
            // Streaming update: same entry id, higher revision.
            store
                .put_entry("s_1", &entry("e_2", Some("e_1"), "two edited", 3))
                .await
                .unwrap();
            // Meta rewrite: title refined later.
            store.put_meta(&meta("s_1", "renamed")).await.unwrap();
        }

        // Fresh store over the same directory = worker restart.
        let store = FsStore::new(dir.path()).unwrap();
        let m = store.get_meta("s_1").await.unwrap().unwrap();
        assert_eq!(m.title, "renamed");
        let e2 = store.get_entry("s_1", "e_2").await.unwrap().unwrap();
        assert_eq!(e2.revision(), 3);
        assert_eq!(
            store.get_active_leaf("s_1").await.unwrap().as_deref(),
            Some("e_2")
        );
        assert_eq!(store.list_entries("s_1").await.unwrap().len(), 2);
        let metas = store.list_metas().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].session_id, "s_1");
    }

    #[tokio::test]
    async fn truncated_tail_is_tolerated() {
        let dir = tempfile::tempdir().unwrap();
        {
            let store = FsStore::new(dir.path()).unwrap();
            store.put_meta(&meta("s_1", "ok")).await.unwrap();
            store
                .put_entry("s_1", &entry("e_1", None, "one", 0))
                .await
                .unwrap();
        }
        // Simulate a crash mid-append: garbage half-line at the tail.
        let path = dir
            .path()
            .join(format!("{}.jsonl", encode_session_id("s_1")));
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .unwrap();
        file.write_all(b"{\"type\":\"entry\",\"entry\":{\"kind\":\"mess")
            .unwrap();
        drop(file);

        let store = FsStore::new(dir.path()).unwrap();
        assert_eq!(store.get_meta("s_1").await.unwrap().unwrap().title, "ok");
        assert_eq!(store.list_entries("s_1").await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_sequence_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        store.put_meta(&meta("s_1", "doomed")).await.unwrap();
        store
            .put_entry("s_1", &entry("e_1", None, "one", 0))
            .await
            .unwrap();
        store.set_active_leaf("s_1", "e_1").await.unwrap();

        let path = dir
            .path()
            .join(format!("{}.jsonl", encode_session_id("s_1")));
        assert!(path.exists());

        // The service's delete order.
        store.delete_entries("s_1").await.unwrap();
        store.delete_active_leaf("s_1").await.unwrap();
        store.delete_meta("s_1").await.unwrap();

        assert!(!path.exists());
        assert!(store.get_meta("s_1").await.unwrap().is_none());
        assert!(store.list_metas().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn hostile_ids_store_and_list_fine() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        let id = "../escape attempt/чат 42";
        store.put_meta(&meta(id, "hostile")).await.unwrap();

        // The file landed inside data_dir, encoded.
        let entries: Vec<_> = std::fs::read_dir(dir.path()).unwrap().collect();
        assert_eq!(entries.len(), 1);

        let metas = store.list_metas().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].session_id, id);
        assert_eq!(metas[0].title, "hostile");
    }

    #[tokio::test]
    async fn unknown_session_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        assert!(store.get_meta("nope").await.unwrap().is_none());
        assert!(store.list_entries("nope").await.unwrap().is_empty());
        assert!(store.get_active_leaf("nope").await.unwrap().is_none());
    }
}
