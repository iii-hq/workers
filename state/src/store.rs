//! The key-value store abstraction and its scope/persistence semantics.

use std::{
    collections::HashMap,
    ffi::OsStr,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;

use crate::structs::StateValue;
use iii_helpers::stream::{StreamDeleteResult, StreamSetResult, StreamUpdateResult, UpdateOp};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde_json::Value;
use tokio::sync::RwLock;

const KEY_FILE_EXTENSION: &str = "bin";

// Values are immutable once installed. A persistence snapshot clones only keys
// and Arc handles, not every JSON tree; replacing a key preserves older snapshots.
type Scope = IndexMap<String, Arc<Value>>;
type Store = Arc<RwLock<HashMap<String, Scope>>>;

/// Default persistence flush cadence (ms) for file-backed stores. Used when no
/// `save_interval_ms` is configured at construction.
const DEFAULT_SAVE_INTERVAL_MS: u64 = 5000;

/// Floor for the save cadence (ms). A value below this — e.g. a hand-edited
/// adapter config that bypasses the configuration schema's `minimum: 100` — is
/// clamped up so it can never drive the save loop into a tight busy-loop.
const MIN_SAVE_INTERVAL_MS: u64 = 100;

#[derive(Archive, RkyvSerialize, RkyvDeserialize)]
struct KeyStorage(String);

#[derive(Clone, Copy, Debug)]
enum DirtyOp {
    Upsert,
    Delete,
}

fn encode_index(index: &str) -> String {
    let mut out = String::with_capacity(index.len());
    for byte in index.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

fn decode_index(encoded: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(encoded.len());
    let mut iter = encoded.as_bytes().iter().copied();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let high = iter.next()?;
            let low = iter.next()?;
            let high = (high as char).to_digit(16)? as u8;
            let low = (low as char).to_digit(16)? as u8;
            bytes.push((high << 4) | low);
        } else {
            bytes.push(byte);
        }
    }
    String::from_utf8(bytes).ok()
}

fn index_file_name(index: &str) -> String {
    format!("{}.{}", encode_index(index), KEY_FILE_EXTENSION)
}

fn index_from_path(path: &Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    let file_name = file_name.strip_suffix(&format!(".{}", KEY_FILE_EXTENSION))?;
    decode_index(file_name)
}

fn load_store_from_dir(dir: &Path) -> HashMap<String, Scope> {
    let mut store = HashMap::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) => {
            tracing::info!(error = ?err, "storage directory not found, starting empty");
            return store;
        }
    };

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        if path.extension() != Some(OsStr::new(KEY_FILE_EXTENSION)) {
            continue;
        }
        let index = match index_from_path(&path) {
            Some(index) => index,
            None => {
                tracing::warn!(path = %path.display(), "invalid index filename, skipping");
                continue;
            }
        };
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                tracing::warn!(error = ?err, path = %path.display(), "failed to read index file");
                continue;
            }
        };
        let storage = match rkyv::access::<ArchivedKeyStorage, rkyv::rancor::Error>(&bytes) {
            Ok(storage) => storage,
            Err(err) => {
                tracing::warn!(error = ?err, path = %path.display(), "failed to parse index file");
                continue;
            }
        };
        let value = match serde_json::from_str::<Scope>(storage.0.as_str()) {
            Ok(value) => value,
            Err(err) => {
                tracing::warn!(error = ?err, path = %path.display(), "failed to decode index value");
                continue;
            }
        };
        store.insert(index, value);
    }

    store
}

// Called only by the blocking flush task (or unit tests). Keep the legacy
// KeyStorage(String) archive layout, temporary filename and rename semantics.
fn persist_index_to_disk(dir: &Path, index: &str, value: &Scope) -> anyhow::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let file_name = index_file_name(index);
    let path = dir.join(&file_name);
    let temp_path = dir.join(format!("{}.tmp", file_name));
    let json = serde_json::to_string(value)?;
    let file = std::fs::File::create(&temp_path)?;
    let writer = rkyv::ser::writer::IoWriter::new(std::io::BufWriter::new(file));
    let writer = rkyv::api::high::to_bytes_in::<_, rkyv::rancor::Error>(&KeyStorage(json), writer)?;
    let mut buffered = writer.into_inner();
    // BufWriter::drop hides write failures. Do not publish until flush succeeds.
    buffered.flush()?;
    drop(buffered);
    std::fs::rename(&temp_path, &path)?;
    Ok(())
}

fn delete_index_from_disk(dir: &Path, index: &str) -> anyhow::Result<()> {
    let path = dir.join(index_file_name(index));
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

/// Re-mark a scope dirty after a failed disk operation — but never on top of
/// a NEWER one. The scope left the dirty map before the write started, so a
/// concurrent `set`/`delete` may have queued fresh intent for it meanwhile;
/// overwriting that would persist the stale intent instead (a failed delete
/// landing on a fresh upsert deletes live data on the next flush).
fn requeue(dirty: &Arc<RwLock<HashMap<String, DirtyOp>>>, index: String, op: DirtyOp) {
    dirty.blocking_write().entry(index).or_insert(op);
}

pub struct KvStore {
    store: Store,
    file_store_dir: Option<PathBuf>,
    dirty: Arc<RwLock<HashMap<String, DirtyOp>>>,
    /// Stop signal for the current save-loop instance. Replaced (and the prior
    /// loop signalled to exit) when `save_interval_ms` is hot-reconfigured via
    /// [`KvStore::reconfigure`]. `None` for in-memory stores, which run
    /// no save loop.
    save_loop_stop: Arc<std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
    /// Serialises `flush_dirty` across the periodic save loop and the
    /// shutdown flush. Both drain the whole dirty map, so without it the
    /// shutdown flush can find the map already empty while the loop's write
    /// is still in flight — and process exit then cancels that write,
    /// losing precisely the update this store is here to keep.
    flush_lock: Arc<tokio::sync::Mutex<()>>,
    /// The boot-configured save cadence (ms, already floored). A reconfigure
    /// that clears `save_interval_ms` reverts to this rather than to the global
    /// default, so clearing the runtime knob restores the adapter's configured
    /// cadence instead of silently dropping to 5000.
    default_interval: u64,
}

impl KvStore {
    pub fn new(config: Option<Value>) -> Self {
        tracing::debug!("Initializing KvStore with config: {:?}", config);
        let store_method = config
            .clone()
            .and_then(|cfg| {
                cfg.get("store_method")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "file_based".to_string());

        if store_method == "in_memory" {
            tracing::warn!(
                "DO NOT USE IN_MEMORY STORE_METHOD IN PRODUCTION - DATA WILL BE LOST ON SHUTDOWN"
            );
        }

        let configured_path = config
            .clone()
            .and_then(|cfg| {
                cfg.get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| iii_worker_paths::default_path("data/state"));
        let file_path = iii_worker_paths::resolve_path(configured_path);

        let interval = config
            .clone()
            .and_then(|cfg| cfg.get("save_interval_ms").and_then(|v| v.as_u64()))
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_SAVE_INTERVAL_MS)
            .max(MIN_SAVE_INTERVAL_MS);

        let file_store_dir = match store_method.as_str() {
            "file_based" => {
                let dir = file_path;
                if let Err(err) = std::fs::create_dir_all(&dir) {
                    tracing::error!(error = ?err, path = %dir.display(), "failed to create storage directory");
                }
                Some(dir)
            }
            "in_memory" => None,
            other => {
                tracing::warn!(store_method = %other, "Unknown store_method, defaulting to in_memory");
                None
            }
        };

        let data_from_disk = match &file_store_dir {
            Some(dir) => load_store_from_dir(dir),
            None => HashMap::new(),
        };
        let store = Arc::new(RwLock::new(data_from_disk));
        let dirty = Arc::new(RwLock::new(HashMap::new()));

        let kv = Self {
            store,
            file_store_dir,
            dirty,
            save_loop_stop: Arc::new(std::sync::Mutex::new(None)),
            flush_lock: Arc::new(tokio::sync::Mutex::new(())),
            default_interval: interval,
        };

        // File-backed stores run a background save loop; in-memory stores have
        // nothing to persist. `spawn_save_loop` is a no-op when not file-backed.
        kv.spawn_save_loop(interval);

        kv
    }

    /// (Re)start the background save loop at `interval_ms`, signalling any prior
    /// instance to exit so only the newest cadence persists. No-op for
    /// in-memory stores. Called once from `new` and again from `reconfigure`.
    fn spawn_save_loop(&self, interval_ms: u64) {
        let Some(dir) = self.file_store_dir.clone() else {
            return;
        };

        let (stop_tx, stop_rx) = tokio::sync::watch::channel(false);
        if let Some(previous) = self
            .save_loop_stop
            .lock()
            .expect("save_loop_stop mutex poisoned")
            .replace(stop_tx)
        {
            let _ = previous.send(true);
        }

        let store = Arc::clone(&self.store);
        let dirty = Arc::clone(&self.dirty);
        let flush_lock = Arc::clone(&self.flush_lock);
        tokio::spawn(async move {
            Self::save_loop(store, dirty, flush_lock, interval_ms, dir, stop_rx).await;
        });
    }

    /// Hot-reconfigure the store. Currently honors `save_interval_ms`: when the
    /// store is file-backed, respawn the save loop at the new cadence. A clear /
    /// invalid value reverts to the boot-configured cadence (`default_interval`),
    /// and any value is floored to `MIN_SAVE_INTERVAL_MS`. No-op for in-memory
    /// stores.
    pub fn reconfigure(&self, config: &Value) {
        if self.file_store_dir.is_none() {
            return;
        }
        let interval = config
            .get("save_interval_ms")
            .and_then(|v| v.as_u64())
            .filter(|&n| n > 0)
            .unwrap_or(self.default_interval)
            .max(MIN_SAVE_INTERVAL_MS);
        tracing::info!(
            save_interval_ms = interval,
            "[KvStore] respawning save loop at new cadence"
        );
        self.spawn_save_loop(interval);
    }

    async fn save_loop(
        store: Store,
        dirty: Arc<RwLock<HashMap<String, DirtyOp>>>,
        flush_lock: Arc<tokio::sync::Mutex<()>>,
        polling_interval: u64,
        dir: PathBuf,
        mut stop_rx: tokio::sync::watch::Receiver<bool>,
    ) {
        let mut interval =
            tokio::time::interval(std::time::Duration::from_millis(polling_interval));
        loop {
            tokio::select! {
                changed = stop_rx.changed() => {
                    // Sender dropped (Err) or signalled `true` → this instance
                    // was replaced by a reconfigure (or the store was dropped).
                    // Exit so only the newest loop persists.
                    if changed.is_err() || *stop_rx.borrow() {
                        tracing::debug!("[KvStore] save loop stopped");
                        break;
                    }
                }
                _ = interval.tick() => {
                    // Every failure is already logged and requeued per scope;
                    // the aggregate is only actionable on the shutdown path.
                    let _ = Self::flush_dirty(&store, &dirty, &flush_lock, &dir).await;
                }
            }
        }
    }

    /// Write every dirty scope to disk now. Failed entries are re-marked dirty
    /// so a later flush retries them, and the count of failures comes back as
    /// an error so the shutdown path can report that the final persistence
    /// attempt did not fully succeed. Shared by the periodic save loop and
    /// [`KvStore::flush`], which `flush_lock` keeps from overlapping.
    async fn flush_dirty(
        store: &Store,
        dirty: &Arc<RwLock<HashMap<String, DirtyOp>>>,
        flush_lock: &Arc<tokio::sync::Mutex<()>>,
        dir: &Path,
    ) -> anyhow::Result<()> {
        // Move the lock into the blocking task BEFORE it drains dirty intent.
        // Aborting this future cannot let another flush overtake the writer or
        // let shutdown return while a previous write still owns the temp file.
        let guard = Arc::clone(flush_lock).lock_owned().await;
        // Serialize with any older writer before checking: shutdown must still
        // wait for in-flight work even if that writer has drained dirty intent.
        // Empty ticks need neither Arc clones nor a blocking task.
        if dirty.read().await.is_empty() {
            return Ok(());
        }
        let store = Arc::clone(store);
        let dirty = Arc::clone(dirty);
        let dir = dir.to_path_buf();
        tokio::task::spawn_blocking(move || {
            let _guard = guard;
            Self::flush_dirty_blocking(&store, &dirty, &dir)
        })
        .await?
    }

    fn flush_dirty_blocking(
        store: &Store,
        dirty: &Arc<RwLock<HashMap<String, DirtyOp>>>,
        dir: &Path,
    ) -> anyhow::Result<()> {
        let batch = dirty.blocking_write().drain().collect::<Vec<_>>();
        let mut failed = 0usize;
        for (index, op) in batch {
            // DirtyOp records work that must be retried; it may be stale by the
            // time the flush owns the writer. Derive the disk action from the
            // current scope so a stale delete cannot remove a live scope, and a
            // stale upsert cannot resurrect an empty or absent one.
            let snapshot = store.blocking_read().get(&index).cloned();
            // Drop the read lock before serializing or touching disk.
            let result = match snapshot {
                Some(value) if !value.is_empty() => persist_index_to_disk(dir, &index, &value),
                _ => delete_index_from_disk(dir, &index),
            };
            if let Err(error) = result {
                tracing::error!(error = ?error, index = %index, "failed to persist index");
                failed += 1;
                // Preserve any newer mutation intent queued after the batch
                // was drained; otherwise retry this operation next time.
                requeue(dirty, index, op);
            }
        }
        if failed > 0 {
            anyhow::bail!("{failed} scope(s) failed to persist and were requeued");
        }
        Ok(())
    }

    /// Flush pending dirty scopes to disk immediately — the shutdown path,
    /// so a stop right after a write doesn't lose the last save window.
    /// No-op for in-memory stores.
    pub async fn flush(&self) -> anyhow::Result<()> {
        match &self.file_store_dir {
            Some(dir) => Self::flush_dirty(&self.store, &self.dirty, &self.flush_lock, dir).await,
            None => Ok(()),
        }
    }

    pub async fn set(&self, index: String, key: String, data: Value) -> StreamSetResult {
        let old = {
            let mut store = self.store.write().await;
            store
                .entry(index.clone())
                .or_default()
                .insert(key, Arc::new(data.clone()))
        };
        if self.file_store_dir.is_some() {
            self.dirty.write().await.insert(index, DirtyOp::Upsert);
        }
        StreamSetResult {
            old_value: old.map(Arc::unwrap_or_clone),
            new_value: data,
        }
    }

    /// Return the selected immutable version; callers serialize or inspect it
    /// directly instead of deep-cloning before the JSON response is built.
    pub async fn get(&self, index: String, key: String) -> Option<StateValue> {
        self.store
            .read()
            .await
            .get(&index)
            .and_then(|scope| scope.get(&key))
            .map(|value| StateValue(Arc::clone(value)))
    }

    pub(crate) async fn remove_shared(&self, index: String, key: String) -> Option<Arc<Value>> {
        let (removed, dirty_op) = {
            let mut store = self.store.write().await;
            let index_map = store.get_mut(&index);

            if let Some(index_map) = index_map {
                let removed = index_map.shift_remove(&key);
                let dirty_op = if removed.is_some() {
                    if index_map.is_empty() {
                        Some(DirtyOp::Delete)
                    } else {
                        Some(DirtyOp::Upsert)
                    }
                } else {
                    None
                };
                (removed, dirty_op)
            } else {
                (None, None)
            }
        };

        if removed.is_some()
            && self.file_store_dir.is_some()
            && let Some(dirty_op) = dirty_op
        {
            self.dirty.write().await.insert(index, dirty_op);
        }

        removed
    }

    pub async fn delete(&self, index: String, key: String) -> StreamDeleteResult {
        let removed = self.remove_shared(index, key).await;
        StreamDeleteResult {
            old_value: removed.map(Arc::unwrap_or_clone),
        }
    }

    /// Swap `key` from `expected` to `value`, atomically. Returns the observed
    /// old value on success, or the current value when the expectation missed.
    ///
    /// The whole point is the read and the write happening under ONE lock. A
    /// caller doing `get` then `set` cannot tell "nobody touched it" from "two
    /// of us read the same value and both wrote", which is how two concurrent
    /// consumers of the same counter each believe they claimed slot N.
    pub async fn compare_and_set(
        &self,
        index: String,
        key: String,
        expected: Option<&Value>,
        value: Value,
    ) -> crate::adapters::CompareAndSetOutcome {
        let mut store = self.store.write().await;
        let current = store
            .get(&index)
            .and_then(|index_map| index_map.get(&key))
            .map(|value| value.as_ref().clone());

        // `expected: None` means "I expect this key to be absent" — the
        // set-if-absent form a claim needs. A stored `null` counts as absent so
        // a deleted-and-rewritten key behaves the same as a never-written one.
        if !crate::adapters::cas_matches(expected, current.as_ref()) {
            return crate::adapters::CompareAndSetOutcome::NotSwapped {
                current: current.unwrap_or(Value::Null),
            };
        }

        store
            .entry(index.clone())
            .or_insert_with(IndexMap::new)
            .insert(key, Arc::new(value));
        drop(store);

        if self.file_store_dir.is_some() {
            self.dirty.write().await.insert(index, DirtyOp::Upsert);
        }
        crate::adapters::CompareAndSetOutcome::Swapped { old_value: current }
    }

    /// Apply one barrier arrival under the SAME write lock `update` uses.
    ///
    /// Atomicity is the whole point: a barrier is a read-modify-write on one
    /// key, and two children completing at once would otherwise each read
    /// "n-1 arrived" and both answer `allow`, spawning the downstream twice.
    /// Holding the lock across the decision makes the completing arrival
    /// unambiguous.
    pub async fn barrier_arrive(
        &self,
        index: String,
        key: String,
        cfg: &crate::barrier::BarrierConfig,
        event: &Value,
    ) -> Result<crate::barrier::Decision, String> {
        let mut store = self.store.write().await;
        let current = store
            .get(&index)
            .and_then(|index_map| index_map.get(&key))
            .map(|value| value.as_ref().clone());

        let (next, decision) = crate::barrier::arrive(current.as_ref(), cfg, event)?;
        let encoded =
            serde_json::to_value(&next).map_err(|e| format!("barrier state serialize: {e}"))?;
        let unchanged = match current.as_ref() {
            None | Some(Value::Null) => next.arrived.is_empty() && !next.complete,
            Some(current) => current == &encoded,
        };
        if unchanged {
            return Ok(decision);
        }
        store
            .entry(index.clone())
            .or_insert_with(IndexMap::new)
            .insert(key, Arc::new(encoded));
        drop(store);

        if self.file_store_dir.is_some() {
            self.dirty.write().await.insert(index, DirtyOp::Upsert);
        }
        Ok(decision)
    }

    pub async fn update(
        &self,
        index: String,
        key: String,
        ops: Vec<UpdateOp>,
    ) -> StreamUpdateResult {
        let mut store = self.store.write().await;

        // Automatically create index_map if it doesn't exist
        let index_map = store.entry(index.clone()).or_insert_with(IndexMap::new);

        // Only one working copy is needed. Keep the original map entry until
        // update completes, then reuse that displaced version in the response
        // when no in-flight persistence/list snapshot still shares it.
        let current = index_map.get(&key).map(|value| value.as_ref().clone());
        let (updated_value, errors) = crate::update_ops::apply_update_ops(current, &ops);
        let previous = index_map.insert(key, Arc::new(updated_value.clone()));

        drop(store);

        if self.file_store_dir.is_some() {
            self.dirty
                .write()
                .await
                .insert(index.clone(), DirtyOp::Upsert);
        }

        StreamUpdateResult {
            old_value: previous.map(Arc::unwrap_or_clone),
            new_value: updated_value,
            errors,
        }
    }

    /// Capture membership, order and immutable versions under the read lock.
    /// Values stay shared until serialization; no second owned-list path exists.
    pub async fn list(&self, index: String) -> Vec<StateValue> {
        self.store
            .read()
            .await
            .get(&index)
            .map_or_else(Vec::new, |scope| {
                scope
                    .values()
                    .map(|value| StateValue(Arc::clone(value)))
                    .collect()
            })
    }

    /// One immutable keyed snapshot, not a key enumeration followed by reads.
    pub async fn list_entries(&self, index: String) -> Vec<(String, StateValue)> {
        self.store
            .read()
            .await
            .get(&index)
            .map_or_else(Vec::new, |scope| {
                scope
                    .iter()
                    .map(|(key, value)| (key.clone(), StateValue(Arc::clone(value))))
                    .collect()
            })
    }

    pub async fn list_keys(&self, index: String) -> Vec<String> {
        let store = self.store.read().await;
        store
            .get(&index)
            .map_or(vec![], |topic| topic.keys().cloned().collect())
    }

    pub async fn list_groups(&self) -> Vec<String> {
        let store = self.store.read().await;
        store.keys().cloned().collect()
    }
}

/// The default store_method is file_based — tests that don't exercise
/// persistence pin in_memory so they never touch `./data/state`.
#[cfg(test)]
fn in_memory_store() -> KvStore {
    KvStore::new(Some(serde_json::json!({"store_method": "in_memory"})))
}

#[cfg(test)]
mod test {
    use super::*;

    fn temp_store_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("kv_store_{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_file_based_load_set_delete() {
        let dir = temp_store_dir();
        let index = "test";
        let key = "test_group::item1";
        let data = serde_json::json!({"key": "value"});
        let index_data = IndexMap::from([(key.to_string(), Arc::new(data.clone()))]);
        let file_path = dir.join(index_file_name(index));

        persist_index_to_disk(&dir, index, &index_data).unwrap();

        let config = serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
            "save_interval_ms": 5
        });
        let kv_store = KvStore::new(Some(config));

        let loaded = kv_store.get(index.to_string(), key.to_string()).await;
        assert_eq!(loaded.as_deref(), Some(&data));

        let updated = serde_json::json!({"key": "updated"});
        kv_store
            .set(index.to_string(), key.to_string(), updated.clone())
            .await;

        let timeout = std::time::Duration::from_secs(5);
        let start = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            let bytes = std::fs::read(&file_path).unwrap();
            let storage = rkyv::from_bytes::<KeyStorage, rkyv::rancor::Error>(&bytes).unwrap();
            let on_disk: IndexMap<String, Value> = serde_json::from_str(&storage.0).unwrap();
            if on_disk.get(key) == Some(&updated) {
                break;
            }
            assert!(
                start.elapsed() < timeout,
                "Timed out waiting for updated value to be persisted to disk"
            );
        }

        kv_store.delete(index.to_string(), key.to_string()).await;
        let start = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if !file_path.exists() {
                break;
            }
            assert!(
                start.elapsed() < timeout,
                "Timed out waiting for file to be deleted from disk"
            );
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A failed write must never bury newer intent for the same scope.
    /// Ordering: a `Delete` is drained and its disk op fails; meanwhile a
    /// `set` queues an `Upsert`. Requeueing the failed `Delete` with a plain
    /// `insert` would overwrite that `Upsert`, and the next flush would then
    /// delete a scope the store still holds — silent data loss.
    #[test]
    fn requeue_never_overwrites_newer_intent() {
        let dirty: Arc<RwLock<HashMap<String, DirtyOp>>> = Arc::new(RwLock::new(HashMap::new()));

        dirty
            .blocking_write()
            .insert("scope".to_string(), DirtyOp::Upsert);
        requeue(&dirty, "scope".to_string(), DirtyOp::Delete);
        assert!(
            matches!(dirty.blocking_read().get("scope"), Some(DirtyOp::Upsert)),
            "the newer Upsert must survive the failed Delete's requeue"
        );

        // With nothing newer queued, the failed op is restored so the next
        // flush retries it.
        dirty.blocking_write().clear();
        requeue(&dirty, "scope".to_string(), DirtyOp::Delete);
        assert!(matches!(
            dirty.blocking_read().get("scope"),
            Some(DirtyOp::Delete)
        ));
    }

    /// A failing disk write has to reach the caller: `BootHandle::shutdown`
    /// logs a warning off this result, and it was unreachable while `flush`
    /// returned `()`. A directory sitting where the index file belongs makes
    /// the write fail without touching permissions.
    #[tokio::test(flavor = "multi_thread")]
    async fn flush_reports_failure_and_requeues_the_scope() {
        let dir = temp_store_dir();
        let index = "blocked";
        let store = KvStore::new(Some(serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
            // Long cadence: this test owns the flush, not the save loop.
            "save_interval_ms": 600_000,
        })));

        std::fs::create_dir_all(dir.join(index_file_name(index))).unwrap();
        store
            .set(index.to_string(), "k".to_string(), serde_json::json!(1))
            .await;

        assert!(
            store.flush().await.is_err(),
            "a failed persist must surface, not just log"
        );
        assert!(
            store.dirty.read().await.contains_key(index),
            "the failed scope stays dirty so a later flush retries it"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reconfigure_respawns_save_loop() {
        let dir = temp_store_dir();
        let index = "recfg";
        let key = "k1";
        let config = serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
            "save_interval_ms": 1000
        });
        let kv_store = KvStore::new(Some(config));

        // Retune to a much faster cadence; the prior loop is signalled to exit
        // and a fresh one takes over. A stop sender must remain registered.
        kv_store.reconfigure(&serde_json::json!({ "save_interval_ms": 5 }));
        assert!(
            kv_store
                .save_loop_stop
                .lock()
                .expect("save_loop_stop mutex")
                .is_some()
        );

        let data = serde_json::json!({ "v": 1 });
        kv_store
            .set(index.to_string(), key.to_string(), data.clone())
            .await;

        // The respawned loop must still persist writes to disk.
        let file_path = dir.join(index_file_name(index));
        let timeout = std::time::Duration::from_secs(5);
        let start = tokio::time::Instant::now();
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            if let Ok(bytes) = std::fs::read(&file_path) {
                let storage = rkyv::from_bytes::<KeyStorage, rkyv::rancor::Error>(&bytes).unwrap();
                let on_disk: IndexMap<String, Value> = serde_json::from_str(&storage.0).unwrap();
                if on_disk.get(key) == Some(&data) {
                    break;
                }
            }
            assert!(
                start.elapsed() < timeout,
                "value not persisted after reconfigure respawn"
            );
        }

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reconfigure_reverts_to_boot_interval_when_cleared() {
        let dir = temp_store_dir();
        let config = serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
            "save_interval_ms": 250
        });
        let kv_store = KvStore::new(Some(config));
        assert_eq!(kv_store.default_interval, 250);

        // Clearing the knob reverts to the boot cadence (250), NOT the global
        // default; the loop stays alive.
        kv_store.reconfigure(&serde_json::json!({}));
        assert!(
            kv_store
                .save_loop_stop
                .lock()
                .expect("save_loop_stop mutex")
                .is_some()
        );
        assert_eq!(kv_store.default_interval, 250);

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_boot_interval_is_floored() {
        let dir = temp_store_dir();
        // A sub-floor value (e.g. hand-edited adapter config bypassing the
        // schema) is clamped up so it cannot drive a tight save loop.
        let config = serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
            "save_interval_ms": 1
        });
        let kv_store = KvStore::new(Some(config));
        assert_eq!(kv_store.default_interval, MIN_SAVE_INTERVAL_MS);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_reconfigure_in_memory_is_noop() {
        let kv_store = in_memory_store(); // in-memory: no save loop
        kv_store.reconfigure(&serde_json::json!({ "save_interval_ms": 100 }));
        assert!(
            kv_store
                .save_loop_stop
                .lock()
                .expect("save_loop_stop mutex")
                .is_none(),
            "in-memory store must not spawn a save loop"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn test_kv_store_invalid_store_method() {
        // when this happens it should default to in_memory
        let config = serde_json::json!({
            "store_method": "unknown_method"
        });
        let kv_store = KvStore::new(Some(config));
        assert!(kv_store.store.read().await.is_empty());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn loads_a_directory_written_by_the_builtin_format() {
        // A file persisted with persist_index_to_disk IS the builtin's format
        // (same rkyv KeyStorage + percent-encoded name); a fresh store must load it.
        let dir = temp_store_dir();
        let data = IndexMap::from([(
            "user-1".to_string(),
            Arc::new(serde_json::json!({"name": "Alice"})),
        )]);
        persist_index_to_disk(&dir, "users", &data).unwrap();

        let store = KvStore::new(Some(serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
        })));
        assert_eq!(
            store.get("users".into(), "user-1".into()).await,
            Some(StateValue::from(serde_json::json!({"name": "Alice"})))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
mod barrier_tests {
    use super::*;
    use crate::barrier::{BarrierConfig, Decision, Expect};

    /// The property the barrier exists for: N producers finishing at the same
    /// moment must produce ONE completion, not N. A get-then-set from outside
    /// the store fails this — every racer reads "not yet complete" and every
    /// racer answers `allow`, so the downstream runs N times.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_arrivals_complete_a_barrier_exactly_once() {
        const N: usize = 32;
        let store = Arc::new(in_memory_store());
        let cfg = Arc::new(BarrierConfig {
            id: "race".into(),
            expect: Expect::Count(N as u64),
            key_from: Some("/key".into()),
            carry: None,
        });

        let mut handles = Vec::new();
        for i in 0..N {
            let store = store.clone();
            let cfg = cfg.clone();
            handles.push(tokio::spawn(async move {
                let event = serde_json::json!({ "key": format!("w{i}") });
                store
                    .barrier_arrive("state_barrier".into(), cfg.id.clone(), &cfg, &event)
                    .await
                    .unwrap()
            }));
        }

        let mut allows = 0;
        for h in handles {
            if matches!(h.await.unwrap(), Decision::Allow { .. }) {
                allows += 1;
            }
        }
        assert_eq!(allows, 1, "exactly one arrival may complete the barrier");
    }

    /// Redelivery is the normal case with at-least-once triggers: the same
    /// arrival racing itself must still count once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn duplicate_arrivals_racing_do_not_over_count() {
        let store = Arc::new(in_memory_store());
        let cfg = Arc::new(BarrierConfig {
            id: "dupes".into(),
            expect: Expect::Count(2),
            key_from: Some("/key".into()),
            carry: None,
        });

        // Eight deliveries of the SAME single arrival: the barrier expects two
        // distinct producers, so none of these may complete it.
        let mut handles = Vec::new();
        for _ in 0..8 {
            let store = store.clone();
            let cfg = cfg.clone();
            handles.push(tokio::spawn(async move {
                let event = serde_json::json!({ "key": "only-one" });
                store
                    .barrier_arrive("state_barrier".into(), cfg.id.clone(), &cfg, &event)
                    .await
                    .unwrap()
            }));
        }
        for h in handles {
            assert!(
                matches!(h.await.unwrap(), Decision::Skip { .. }),
                "one producer delivered eight times is still one arrival"
            );
        }
    }
}

#[cfg(test)]
mod cas_tests {
    use super::*;
    use crate::adapters::CompareAndSetOutcome::{NotSwapped, Swapped};
    use serde_json::json;

    #[tokio::test]
    async fn a_swap_happens_only_when_the_expectation_holds() {
        let store = in_memory_store();
        // Absent → set-if-absent succeeds.
        assert_eq!(
            store
                .compare_and_set("s".into(), "k".into(), None, json!(1))
                .await,
            Swapped { old_value: None }
        );
        // Absent again → now occupied, so the same call reports what is there.
        assert_eq!(
            store
                .compare_and_set("s".into(), "k".into(), None, json!(2))
                .await,
            NotSwapped { current: json!(1) }
        );
        // Correct expectation swaps.
        assert_eq!(
            store
                .compare_and_set("s".into(), "k".into(), Some(&json!(1)), json!(2))
                .await,
            Swapped {
                old_value: Some(json!(1))
            }
        );
        // Stale expectation does not, and hands back the current value so the
        // caller can recompute instead of re-reading.
        assert_eq!(
            store
                .compare_and_set("s".into(), "k".into(), Some(&json!(1)), json!(3))
                .await,
            NotSwapped { current: json!(2) }
        );
        assert_eq!(
            store.get("s".into(), "k".into()).await.as_deref(),
            Some(&json!(2))
        );
    }

    #[tokio::test]
    async fn failed_or_ignored_atomic_operations_do_not_create_scopes() {
        let store = in_memory_store();
        assert_eq!(
            store
                .compare_and_set("phantom".into(), "k".into(), Some(&json!(1)), json!(2))
                .await,
            NotSwapped {
                current: Value::Null
            }
        );

        let cfg = crate::barrier::BarrierConfig {
            id: "invalid".into(),
            expect: crate::barrier::Expect::Count(0),
            key_from: None,
            carry: None,
        };
        assert!(
            store
                .barrier_arrive(
                    crate::barrier::BARRIER_SCOPE.into(),
                    cfg.id.clone(),
                    &cfg,
                    &json!({ "key": "a" }),
                )
                .await
                .is_err()
        );

        let cfg = crate::barrier::BarrierConfig {
            id: "ignored".into(),
            expect: crate::barrier::Expect::Keys(vec!["expected".into()]),
            key_from: None,
            carry: None,
        };
        assert!(matches!(
            store
                .barrier_arrive(
                    crate::barrier::BARRIER_SCOPE.into(),
                    cfg.id.clone(),
                    &cfg,
                    &json!({ "key": "unexpected" }),
                )
                .await,
            Ok(crate::barrier::Decision::Skip { .. })
        ));
        assert!(store.list_groups().await.is_empty());
    }

    /// The bug this exists for: N consumers claiming slots off one counter must
    /// produce N distinct slots. With `get` then `set` they collide — two read
    /// the same value, both write, and both believe they hold that slot.
    #[tokio::test(flavor = "multi_thread", worker_threads = 8)]
    async fn concurrent_claimers_each_get_a_distinct_slot() {
        const N: usize = 40;
        let store = Arc::new(in_memory_store());
        store
            .compare_and_set("claims".into(), "counter".into(), None, json!(0))
            .await;

        let mut handles = Vec::new();
        for _ in 0..N {
            let store = store.clone();
            handles.push(tokio::spawn(async move {
                // Retry until this claimer wins a slot — the loop a caller
                // writes on top of the primitive.
                loop {
                    let current = store
                        .get("claims".into(), "counter".into())
                        .await
                        .unwrap_or_else(|| StateValue::from(json!(0)));
                    let next = current.as_u64().unwrap_or(0) + 1;
                    if matches!(
                        store
                            .compare_and_set(
                                "claims".into(),
                                "counter".into(),
                                Some(&current),
                                json!(next),
                            )
                            .await,
                        Swapped { .. }
                    ) {
                        return next;
                    }
                }
            }));
        }

        let mut slots = Vec::new();
        for h in handles {
            slots.push(h.await.unwrap());
        }
        slots.sort_unstable();
        let distinct: std::collections::HashSet<_> = slots.iter().collect();
        assert_eq!(distinct.len(), N, "every claimer must hold its own slot");
        assert_eq!(slots, (1..=N as u64).collect::<Vec<_>>());
        assert_eq!(
            store
                .get("claims".into(), "counter".into())
                .await
                .as_deref(),
            Some(&json!(N))
        );
    }

    #[tokio::test]
    async fn a_stored_null_counts_as_absent() {
        // A deleted-then-rewritten key must behave like a never-written one, or
        // set-if-absent would refuse forever after the first delete.
        let store = in_memory_store();
        store.set("s".into(), "k".into(), Value::Null).await;
        assert_eq!(
            store
                .compare_and_set("s".into(), "k".into(), None, json!("claimed"))
                .await,
            Swapped {
                old_value: Some(Value::Null)
            }
        );
    }
}

#[cfg(test)]
#[path = "store_memory_tests.rs"]
mod memory_tests;
