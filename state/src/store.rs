//! The key-value store abstraction and its scope/persistence semantics.

use std::{
    collections::HashMap,
    ffi::OsStr,
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::Arc,
};

use indexmap::IndexMap;

use iii_helpers::stream::{StreamDeleteResult, StreamSetResult, StreamUpdateResult, UpdateOp};
use rkyv::{Archive, Deserialize as RkyvDeserialize, Serialize as RkyvSerialize};
use serde_json::Value;
use tokio::sync::RwLock;

const KEY_FILE_EXTENSION: &str = "bin";

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

fn load_store_from_dir(dir: &Path) -> HashMap<String, IndexMap<String, Value>> {
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
        let storage = match rkyv::from_bytes::<KeyStorage, rkyv::rancor::Error>(&bytes) {
            Ok(storage) => storage,
            Err(err) => {
                tracing::warn!(error = ?err, path = %path.display(), "failed to parse index file");
                continue;
            }
        };
        let value = match serde_json::from_str::<IndexMap<String, Value>>(&storage.0) {
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

async fn persist_index_to_disk(
    dir: &Path,
    index: &str,
    value: &IndexMap<String, Value>,
) -> anyhow::Result<()> {
    if let Err(err) = tokio::fs::create_dir_all(dir).await {
        tracing::error!(error = ?err, path = %dir.display(), "failed to create storage directory");
        return Err(err.into());
    }

    let file_name = index_file_name(index);
    let path = dir.join(&file_name);
    let temp_path = dir.join(format!("{}.tmp", file_name));
    let json = serde_json::to_string(value)?;
    let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&KeyStorage(json))?;

    tokio::fs::write(&temp_path, bytes).await?;
    tokio::fs::rename(&temp_path, &path).await?;

    Ok(())
}

async fn delete_index_from_disk(dir: &Path, index: &str) -> anyhow::Result<()> {
    let path = dir.join(index_file_name(index));
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err.into()),
    }
}

pub struct KvStore {
    store: Arc<RwLock<HashMap<String, IndexMap<String, Value>>>>,
    file_store_dir: Option<PathBuf>,
    dirty: Arc<RwLock<HashMap<String, DirtyOp>>>,
    /// Stop signal for the current save-loop instance. Replaced (and the prior
    /// loop signalled to exit) when `save_interval_ms` is hot-reconfigured via
    /// [`KvStore::reconfigure`]. `None` for in-memory stores, which run
    /// no save loop.
    save_loop_stop: Arc<std::sync::Mutex<Option<tokio::sync::watch::Sender<bool>>>>,
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
            .unwrap_or_else(|| "in_memory".to_string());

        if store_method == "in_memory" {
            tracing::warn!(
                "DO NOT USE IN_MEMORY STORE_METHOD IN PRODUCTION - DATA WILL BE LOST ON SHUTDOWN"
            );
        }

        let file_path = config
            .clone()
            .and_then(|cfg| {
                cfg.get("file_path")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_else(|| "kv_store_data.db".to_string());

        let interval = config
            .clone()
            .and_then(|cfg| cfg.get("save_interval_ms").and_then(|v| v.as_u64()))
            .filter(|&n| n > 0)
            .unwrap_or(DEFAULT_SAVE_INTERVAL_MS)
            .max(MIN_SAVE_INTERVAL_MS);

        let file_store_dir = match store_method.as_str() {
            "file_based" => {
                let dir = PathBuf::from(&file_path);
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
        tokio::spawn(async move {
            Self::save_loop(store, dirty, interval_ms, dir, stop_rx).await;
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
        store: Arc<RwLock<HashMap<String, IndexMap<String, Value>>>>,
        dirty: Arc<RwLock<HashMap<String, DirtyOp>>>,
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
                    let batch = {
                        let mut dirty = dirty.write().await;
                        if dirty.is_empty() {
                            continue;
                        }
                        dirty.drain().collect::<Vec<_>>()
                    };

                    for (index, op) in batch {
                        match op {
                            DirtyOp::Upsert => {
                                let value = {
                                    let store = store.read().await;
                                    store.get(&index).cloned()
                                };
                                if let Some(value) = value
                                    && let Err(err) =
                                        persist_index_to_disk(&dir, &index, &value).await
                                {
                                    tracing::error!(error = ?err, index = %index, "failed to persist index");
                                    let mut dirty = dirty.write().await;
                                    dirty.insert(index, DirtyOp::Upsert);
                                }
                            }
                            DirtyOp::Delete => {
                                if let Err(err) = delete_index_from_disk(&dir, &index).await {
                                    tracing::error!(error = ?err, index = %index, "failed to delete index");
                                    let mut dirty = dirty.write().await;
                                    dirty.insert(index, DirtyOp::Delete);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    pub async fn set(&self, index: String, key: String, data: Value) -> StreamSetResult {
        let result = {
            let mut store = self.store.write().await;
            let index_map = store.get_mut(&index);

            if let Some(index_map) = index_map {
                let old_value = index_map.get(&key).cloned();
                index_map.insert(key.clone(), data.clone());

                StreamSetResult {
                    old_value,
                    new_value: data.clone(),
                }
            } else {
                let mut index_map = IndexMap::new();
                index_map.insert(key, data.clone());
                store.insert(index.clone(), index_map);

                StreamSetResult {
                    old_value: None,
                    new_value: data.clone(),
                }
            }
        };

        if self.file_store_dir.is_some() {
            self.dirty.write().await.insert(index, DirtyOp::Upsert);
        }

        result
    }

    pub async fn get(&self, index: String, key: String) -> Option<Value> {
        let store = self.store.read().await;
        let index = store.get(&index);

        if let Some(index) = index {
            return index.get(&key).cloned();
        }

        None
    }

    pub async fn delete(&self, index: String, key: String) -> StreamDeleteResult {
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
                (StreamDeleteResult { old_value: removed }, dirty_op)
            } else {
                (StreamDeleteResult { old_value: None }, None)
            }
        };

        if removed.old_value.is_some()
            && self.file_store_dir.is_some()
            && let Some(dirty_op) = dirty_op
        {
            self.dirty.write().await.insert(index, dirty_op);
        }

        removed
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

        let old_value = index_map.get(&key).cloned();
        let (updated_value, errors) = crate::update_ops::apply_update_ops(old_value.clone(), &ops);

        // Write the updated value back to the store
        index_map.insert(key.clone(), updated_value.clone());

        drop(store);

        if self.file_store_dir.is_some() {
            self.dirty
                .write()
                .await
                .insert(index.clone(), DirtyOp::Upsert);
        }

        StreamUpdateResult {
            old_value,
            new_value: updated_value,
            errors,
        }
    }

    pub async fn list(&self, index: String) -> Vec<Value> {
        let store = self.store.read().await;
        store
            .get(&index)
            .map_or(vec![], |topic| topic.values().cloned().collect())
    }

    pub async fn list_groups(&self) -> Vec<String> {
        let store = self.store.read().await;
        store.keys().cloned().collect()
    }
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
        let index_data = IndexMap::from([(key.to_string(), data.clone())]);
        let file_path = dir.join(index_file_name(index));

        persist_index_to_disk(&dir, index, &index_data.clone())
            .await
            .unwrap();

        let config = serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
            "save_interval_ms": 5
        });
        let kv_store = KvStore::new(Some(config));

        let loaded = kv_store.get(index.to_string(), key.to_string()).await;
        assert_eq!(loaded, Some(data.clone()));

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
        let kv_store = KvStore::new(None); // in-memory: no save loop
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
        let data = IndexMap::from([("user-1".to_string(), serde_json::json!({"name": "Alice"}))]);
        persist_index_to_disk(&dir, "users", &data).await.unwrap();

        let store = KvStore::new(Some(serde_json::json!({
            "store_method": "file_based",
            "file_path": dir.to_string_lossy(),
        })));
        assert_eq!(
            store.get("users".into(), "user-1".into()).await,
            Some(serde_json::json!({"name": "Alice"}))
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
