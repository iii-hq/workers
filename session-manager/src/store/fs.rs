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
//! Attachments are not part of the log. Each one is a pair of files under
//! `<data_dir>/attachments/<encoded_session_id>/`:
//! `<attachment_id>.bin` (the bytes) and `<attachment_id>.json` (the
//! `AttachmentMeta`), both written atomically (tmp + rename), metadata
//! last so a listed attachment always has its bytes. Deleting a session
//! removes the folder.
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
use crate::types::{AttachmentMeta, SessionEntry, SessionMeta};

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
    fn replay_file(path: &Path, session_id: &str) -> Result<LoadedSession, StoreError> {
        let contents = match std::fs::read_to_string(path) {
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
    ///
    /// The replay runs on the blocking pool and outside the cache lock: a
    /// multi-MB transcript parsed on the async thread while holding the
    /// `std::Mutex` stalled every other session's append behind it.
    async fn with_loaded<T>(
        &self,
        session_id: &str,
        f: impl FnOnce(&mut LoadedSession) -> T,
    ) -> Result<T, StoreError> {
        if !self.lock().contains_key(session_id) {
            let path = self.file_path(session_id);
            let id = session_id.to_string();
            let loaded = tokio::task::spawn_blocking(move || Self::replay_file(&path, &id))
                .await
                .map_err(|e| StoreError(format!("replay {session_id}: {e}")))??;
            // A concurrent caller may have replayed (and mutated) the same
            // session while we were off the lock; its state stays.
            self.lock().entry(session_id.to_string()).or_insert(loaded);
        }
        let mut cache = self.lock();
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

    /// Folder holding a session's attachments (`attachments/<encoded id>/`).
    /// Lives beside the `.jsonl` files; `session_ids_on_disk` only reads
    /// `.jsonl` names, so the folder is never mistaken for a session.
    fn attachments_dir(&self, session_id: &str) -> PathBuf {
        self.dir
            .join("attachments")
            .join(encode_session_id(session_id))
    }

    /// `(metadata, bytes)` paths of one attachment. Ids are store-generated,
    /// but they pass through the same encoder as session ids so a hostile
    /// value can never point outside the session's folder.
    fn attachment_paths(&self, session_id: &str, attachment_id: &str) -> (PathBuf, PathBuf) {
        let dir = self.attachments_dir(session_id);
        let stem = encode_session_id(attachment_id);
        (
            dir.join(format!("{stem}.json")),
            dir.join(format!("{stem}.bin")),
        )
    }
}

#[async_trait]
impl SessionStore for FsStore {
    async fn get_meta(&self, session_id: &str) -> Result<Option<SessionMeta>, StoreError> {
        self.with_loaded(session_id, |s| s.meta.clone()).await
    }

    async fn put_meta(&self, meta: &SessionMeta) -> Result<(), StoreError> {
        // Disk first, cache second: a failed append must never leave the
        // cache claiming state the file doesn't have (it would silently
        // revert on restart). Cold-cache replay after the append already
        // contains the record, so the closure is idempotent.
        let record = Record::Meta { meta: meta.clone() };
        self.append_record(&meta.session_id, &record)?;
        self.with_loaded(&meta.session_id, |s| s.meta = Some(meta.clone()))
            .await
    }

    async fn delete_meta(&self, session_id: &str) -> Result<(), StoreError> {
        let snapshot = self
            .with_loaded(session_id, |s| {
                s.meta = None;
                s.clone()
            })
            .await?;
        if snapshot.is_empty() {
            self.lock().remove(session_id);
        }
        self.persist_snapshot(session_id, &snapshot)
    }

    async fn list_metas(&self) -> Result<Vec<SessionMeta>, StoreError> {
        let mut metas = Vec::new();
        for session_id in self.session_ids_on_disk()? {
            if let Some(meta) = self.with_loaded(&session_id, |s| s.meta.clone()).await? {
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
            .await
    }

    async fn put_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<(), StoreError> {
        let record = Record::Entry {
            entry: entry.clone(),
        };
        self.append_record(session_id, &record)?;
        self.with_loaded(session_id, |s| {
            s.entries.insert(entry.id().to_string(), entry.clone());
        })
        .await
    }

    async fn list_entries(&self, session_id: &str) -> Result<Vec<SessionEntry>, StoreError> {
        self.with_loaded(session_id, |s| s.entries.values().cloned().collect())
            .await
    }

    async fn delete_entries(&self, session_id: &str) -> Result<(), StoreError> {
        let snapshot = self
            .with_loaded(session_id, |s| {
                s.entries.clear();
                s.clone()
            })
            .await?;
        if snapshot.is_empty() {
            self.lock().remove(session_id);
        }
        self.persist_snapshot(session_id, &snapshot)
    }

    async fn get_active_leaf(&self, session_id: &str) -> Result<Option<String>, StoreError> {
        self.with_loaded(session_id, |s| s.leaf.clone()).await
    }

    async fn set_active_leaf(&self, session_id: &str, entry_id: &str) -> Result<(), StoreError> {
        let record = Record::Leaf {
            entry_id: Some(entry_id.to_string()),
        };
        self.append_record(session_id, &record)?;
        self.with_loaded(session_id, |s| s.leaf = Some(entry_id.to_string()))
            .await
    }

    async fn delete_active_leaf(&self, session_id: &str) -> Result<(), StoreError> {
        let snapshot = self
            .with_loaded(session_id, |s| {
                s.leaf = None;
                s.clone()
            })
            .await?;
        if snapshot.is_empty() {
            self.lock().remove(session_id);
        }
        self.persist_snapshot(session_id, &snapshot)
    }

    async fn put_attachment(&self, meta: &AttachmentMeta, bytes: &[u8]) -> Result<(), StoreError> {
        let dir = self.attachments_dir(&meta.session_id);
        std::fs::create_dir_all(&dir)
            .map_err(|e| StoreError(format!("create {}: {e}", dir.display())))?;
        let (meta_path, bin_path) = self.attachment_paths(&meta.session_id, &meta.attachment_id);
        // Bytes first, metadata last: the metadata file is what makes an
        // attachment visible, so a crash between the two leaves an orphan
        // blob (harmless, overwritten by a retry) rather than a listed
        // attachment whose bytes are missing.
        write_atomically(&bin_path, bytes)?;
        let json = serde_json::to_vec(meta)
            .map_err(|e| StoreError(format!("serialize attachment meta: {e}")))?;
        write_atomically(&meta_path, &json)
    }

    async fn get_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<Option<(AttachmentMeta, Vec<u8>)>, StoreError> {
        let (meta_path, bin_path) = self.attachment_paths(session_id, attachment_id);
        let Some(meta) = read_attachment_meta(&meta_path)? else {
            return Ok(None);
        };
        let bytes = match std::fs::read(&bin_path) {
            Ok(b) => b,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Err(StoreError(format!(
                    "attachment {attachment_id} of session {session_id} has metadata but no bytes"
                )))
            }
            Err(e) => return Err(StoreError(format!("read {}: {e}", bin_path.display()))),
        };
        Ok(Some((meta, bytes)))
    }

    async fn list_attachments(&self, session_id: &str) -> Result<Vec<AttachmentMeta>, StoreError> {
        let dir = self.attachments_dir(session_id);
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(e) => return Err(StoreError(format!("read {}: {e}", dir.display()))),
        };
        let mut metas = Vec::new();
        for dent in entries {
            let dent = dent.map_err(|e| StoreError(format!("read attachments entry: {e}")))?;
            let path = dent.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            if let Some(meta) = read_attachment_meta(&path)? {
                metas.push(meta);
            }
        }
        Ok(metas)
    }

    async fn delete_attachment(
        &self,
        session_id: &str,
        attachment_id: &str,
    ) -> Result<bool, StoreError> {
        let (meta_path, bin_path) = self.attachment_paths(session_id, attachment_id);
        // Metadata first: once it is gone the attachment is invisible to
        // `list`/`get`, so a crash before the blob removal leaves only an
        // orphan `.bin` (swept with the session folder), never a listed
        // attachment with missing bytes.
        let existed = match std::fs::remove_file(&meta_path) {
            Ok(()) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(e) => return Err(StoreError(format!("remove {}: {e}", meta_path.display()))),
        };
        match std::fs::remove_file(&bin_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(StoreError(format!("remove {}: {e}", bin_path.display()))),
        }
        Ok(existed)
    }

    async fn delete_attachments(&self, session_id: &str) -> Result<(), StoreError> {
        let dir = self.attachments_dir(session_id);
        match std::fs::remove_dir_all(&dir) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(StoreError(format!("remove {}: {e}", dir.display()))),
        }
    }
}

/// Write `bytes` to `path` via a sibling temp file + rename, so a reader
/// never observes a half-written attachment.
fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), StoreError> {
    // `<name>.<ext>.tmp`, not `with_extension`: the `.bin` and `.json` of
    // one attachment must never collapse onto the same temp file.
    let mut tmp = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = PathBuf::from(tmp);
    std::fs::write(&tmp, bytes).map_err(|e| StoreError(format!("write {}: {e}", tmp.display())))?;
    std::fs::rename(&tmp, path).map_err(|e| {
        StoreError(format!(
            "rename {} -> {}: {e}",
            tmp.display(),
            path.display()
        ))
    })
}

/// Parse one attachment metadata file; `None` when it does not exist.
fn read_attachment_meta(path: &Path) -> Result<Option<AttachmentMeta>, StoreError> {
    let raw = match std::fs::read(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(StoreError(format!("read {}: {e}", path.display()))),
    };
    serde_json::from_slice(&raw)
        .map(Some)
        .map_err(|e| StoreError(format!("malformed attachment meta {}: {e}", path.display())))
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
            draft: None,
            draft_attachments: None,
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
            message: Box::new(AgentMessage::User {
                content: vec![ContentBlock::Text { text: text.into() }],
                timestamp: 1,
            }),
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
            // Meta rewrite: title refined later, a composer draft parked.
            let mut renamed = meta("s_1", "renamed");
            renamed.draft = Some("unsent input".into());
            store.put_meta(&renamed).await.unwrap();
        }

        // Fresh store over the same directory = worker restart.
        let store = FsStore::new(dir.path()).unwrap();
        let m = store.get_meta("s_1").await.unwrap().unwrap();
        assert_eq!(m.title, "renamed");
        assert_eq!(m.draft.as_deref(), Some("unsent input"));
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
    async fn failed_append_leaves_cache_consistent_with_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        store.put_meta(&meta("s_1", "before")).await.unwrap();

        // Force the next append to fail: swap the session file for a
        // directory so the append open errors deterministically.
        let path = dir
            .path()
            .join(format!("{}.jsonl", encode_session_id("s_1")));
        std::fs::remove_file(&path).unwrap();
        std::fs::create_dir(&path).unwrap();

        assert!(store.put_meta(&meta("s_1", "after")).await.is_err());
        assert!(store
            .put_entry("s_1", &entry("e_1", None, "one", 0))
            .await
            .is_err());
        assert!(store.set_active_leaf("s_1", "e_1").await.is_err());

        // The cache still reflects the last durable state — a failed
        // write must not leave phantom in-memory state.
        assert_eq!(
            store.get_meta("s_1").await.unwrap().unwrap().title,
            "before"
        );
        assert!(store.list_entries("s_1").await.unwrap().is_empty());
        assert!(store.get_active_leaf("s_1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn unknown_session_reads_as_empty() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        assert!(store.get_meta("nope").await.unwrap().is_none());
        assert!(store.list_entries("nope").await.unwrap().is_empty());
        assert!(store.get_active_leaf("nope").await.unwrap().is_none());
    }

    fn attachment(session_id: &str, id: &str, name: &str, bytes: &[u8]) -> AttachmentMeta {
        AttachmentMeta {
            attachment_id: id.into(),
            session_id: session_id.into(),
            name: name.into(),
            mime: "application/octet-stream".into(),
            size: bytes.len() as u64,
            sha256: "00".into(),
            created_at: 1,
        }
    }

    #[tokio::test]
    async fn attachments_roundtrip_survive_restart_and_list() {
        let dir = tempfile::tempdir().unwrap();
        let bytes = b"%PDF-1.4 not really";
        {
            let store = FsStore::new(dir.path()).unwrap();
            store.put_meta(&meta("s_1", "with files")).await.unwrap();
            store
                .put_attachment(&attachment("s_1", "a_1", "report.pdf", bytes), bytes)
                .await
                .unwrap();
            store
                .put_attachment(&attachment("s_1", "a_2", "notes.txt", b"hi"), b"hi")
                .await
                .unwrap();
        }

        // Worker restart: attachments are plain files, nothing to replay.
        let store = FsStore::new(dir.path()).unwrap();
        let (meta, data) = store.get_attachment("s_1", "a_1").await.unwrap().unwrap();
        assert_eq!(meta.name, "report.pdf");
        assert_eq!(meta.size, bytes.len() as u64);
        assert_eq!(data, bytes);

        let mut listed = store.list_attachments("s_1").await.unwrap();
        listed.sort_by(|a, b| a.attachment_id.cmp(&b.attachment_id));
        let ids: Vec<&str> = listed.iter().map(|m| m.attachment_id.as_str()).collect();
        assert_eq!(ids, vec!["a_1", "a_2"]);

        // The attachments folder must never be mistaken for a session file.
        let metas = store.list_metas().await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].session_id, "s_1");

        assert!(store.get_attachment("s_1", "a_9").await.unwrap().is_none());
        assert!(store.get_attachment("s_x", "a_1").await.unwrap().is_none());
        assert!(store.list_attachments("s_x").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn put_attachment_replaces_and_delete_removes_the_folder() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        store
            .put_attachment(&attachment("s_1", "a_1", "v1.txt", b"one"), b"one")
            .await
            .unwrap();
        store
            .put_attachment(&attachment("s_1", "a_1", "v2.txt", b"two!"), b"two!")
            .await
            .unwrap();
        let (meta, data) = store.get_attachment("s_1", "a_1").await.unwrap().unwrap();
        assert_eq!(meta.name, "v2.txt");
        assert_eq!(data, b"two!");

        // Single removal: gone from get/list, other attachments untouched,
        // a repeat reports `false`.
        store
            .put_attachment(&attachment("s_1", "a_2", "other.txt", b"o"), b"o")
            .await
            .unwrap();
        assert!(store.delete_attachment("s_1", "a_1").await.unwrap());
        assert!(!store.delete_attachment("s_1", "a_1").await.unwrap());
        assert!(store.get_attachment("s_1", "a_1").await.unwrap().is_none());
        assert_eq!(store.list_attachments("s_1").await.unwrap().len(), 1);
        assert!(!store.delete_attachment("s_x", "a_1").await.unwrap());

        let folder = store.attachments_dir("s_1");
        assert!(folder.is_dir());
        store.delete_attachments("s_1").await.unwrap();
        assert!(!folder.exists());
        assert!(store.list_attachments("s_1").await.unwrap().is_empty());
        // Deleting again is a no-op, not an error.
        store.delete_attachments("s_1").await.unwrap();
    }

    #[tokio::test]
    async fn hostile_attachment_ids_stay_inside_the_session_folder() {
        let dir = tempfile::tempdir().unwrap();
        let store = FsStore::new(dir.path()).unwrap();
        let id = "../../escape";
        store
            .put_attachment(&attachment("s_1", id, "x", b"x"), b"x")
            .await
            .unwrap();
        let folder = store.attachments_dir("s_1");
        let names: Vec<String> = std::fs::read_dir(&folder)
            .unwrap()
            .map(|d| d.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert!(names.iter().all(|n| !n.contains('/')), "{names:?}");
        assert!(store.get_attachment("s_1", id).await.unwrap().is_some());
    }
}
