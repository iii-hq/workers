//! The canvas record and its persistence over the state bus.
//!
//! [`CanvasRecord`] is the single source of truth for the wire shape shared by
//! every `canvas::*` function, the state-bus payloads, and the injected
//! console UI (`ui/src/lib/types.ts` mirrors it field for field).
//!
//! Records live in the `state` worker under scope [`STATE_SCOPE`], reached
//! over the engine bus (the editor worker's bus-call precedent). The key
//! layout is deliberate:
//!
//! - `record/<id>` — one full [`CanvasRecord`] per canvas
//! - `index` — a single array of [`IndexEntry`] summaries
//!
//! The side index exists because `state::list` scans a whole scope and does
//! not scale; listing canvases must never grow with the size of every stored
//! source. Reads that miss return `None` — on the wire, `state::get` and
//! `state::delete` return the raw stored VALUE (or `null`), never a
//! `{value: …}` envelope (see `state/src/functions.rs`).
//!
//! Index writes are serialized with an in-process mutex: this worker is the
//! only writer of the `canvas` scope, so read-modify-write of the index only
//! races with itself.

use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::Mutex;

/// The `state` worker scope every canvas record is stored under.
pub const STATE_SCOPE: &str = "canvas";

/// The state key holding the [`IndexEntry`] array.
const INDEX_KEY: &str = "index";

/// Timeout for one `state::*` bus call. State reads and writes are local
/// adapter operations; anything slower than this is an outage, not latency.
const STATE_TIMEOUT_MS: u64 = 10_000;

fn record_key(id: &str) -> String {
    format!("record/{id}")
}

/// Diagram format a canvas holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CanvasFormat {
    /// Mermaid text — rendered from source by the console.
    Mermaid,
    /// A freeform whiteboard — the source is an excalidraw scene JSON.
    Freeform,
}

/// One stored canvas. The id is a stable 8-character slug that never changes
/// across updates; `source` is always the editable source of truth (mermaid
/// text, or the excalidraw scene JSON for freeform).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CanvasRecord {
    /// Stable 8-character slug identifying this canvas. Never changes across
    /// updates.
    pub id: String,

    /// Human-readable canvas name.
    pub name: String,

    /// Diagram format: `mermaid` or `freeform`.
    pub format: CanvasFormat,

    /// The editable source of truth: mermaid text for `mermaid`, the
    /// excalidraw scene JSON for `freeform`.
    pub source: String,

    /// Mermaid diagram family (`flowchart`, `sequenceDiagram`, …), derived
    /// from the source. `null` for a freeform canvas.
    pub family: Option<String>,

    /// Creation time, unix seconds.
    pub created_at: i64,

    /// Last update time, unix seconds.
    pub updated_at: i64,
}

/// One row of the side index under the `index` state key: just enough to
/// list and sort without loading any source.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IndexEntry {
    /// Stable 8-character canvas id.
    pub id: String,

    /// Human-readable canvas name.
    pub name: String,

    /// Diagram format: `mermaid` or `freeform`.
    pub format: CanvasFormat,

    /// Last update time, unix seconds — the list sort key.
    pub updated_at: i64,
}

impl IndexEntry {
    fn of(record: &CanvasRecord) -> Self {
        Self {
            id: record.id.clone(),
            name: record.name.clone(),
            format: record.format,
            updated_at: record.updated_at,
        }
    }
}

/// Where the key/value calls actually go.
///
/// `Bus` is the production path: every operation is a `state::*` trigger over
/// the engine. `Memory` is the test seam the function unit tests run against —
/// same semantics (`get` of a missing key is `None`, `delete` returns the old
/// value), no engine required.
enum Backend {
    Bus(Arc<IIIClient>),
    Memory(std::sync::Mutex<HashMap<String, Value>>),
}

/// State-bus persistence for canvas records.
///
/// Every method goes over the engine bus to the `state` worker (scope
/// [`STATE_SCOPE`]); nothing is held in process memory, so a worker restart
/// loses nothing.
pub struct Store {
    backend: Backend,
    /// Serializes read-modify-write of the index across concurrent handlers.
    index_lock: Mutex<()>,
    /// Serializes whole record read-modify-write cycles (update and the
    /// element operations). The state worker has no compare-and-set, but
    /// every mutation flows through this one worker process — holding
    /// this across load→save closes the lost-update window.
    mutation_lock: Mutex<()>,
}

impl Store {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            backend: Backend::Bus(iii),
            index_lock: Mutex::new(()),
            mutation_lock: Mutex::new(()),
        }
    }

    /// An engine-free store over a process-local map — the unit-test seam.
    /// Key semantics match the state worker exactly.
    pub fn in_memory() -> Self {
        Self {
            backend: Backend::Memory(std::sync::Mutex::new(HashMap::new())),
            index_lock: Mutex::new(()),
            mutation_lock: Mutex::new(()),
        }
    }

    /// Hold for the duration of a record read-modify-write.
    pub async fn mutation_guard(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.mutation_lock.lock().await
    }

    async fn bus_call(
        iii: &Arc<IIIClient>,
        function_id: &str,
        payload: Value,
    ) -> Result<Value, String> {
        iii.trigger(TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await
        .map_err(|e| format!("{function_id} failed: {e}"))
    }

    /// `state::get` — the raw stored value, `None` when the key is unset.
    async fn state_get(&self, key: &str) -> Result<Option<Value>, String> {
        match &self.backend {
            Backend::Bus(iii) => {
                let value = Self::bus_call(
                    iii,
                    "state::get",
                    json!({ "scope": STATE_SCOPE, "key": key }),
                )
                .await?;
                Ok(if value.is_null() { None } else { Some(value) })
            }
            Backend::Memory(map) => Ok(map.lock().expect("store map lock").get(key).cloned()),
        }
    }

    async fn state_set(&self, key: &str, value: Value) -> Result<(), String> {
        match &self.backend {
            Backend::Bus(iii) => Self::bus_call(
                iii,
                "state::set",
                json!({ "scope": STATE_SCOPE, "key": key, "value": value }),
            )
            .await
            .map(|_| ()),
            Backend::Memory(map) => {
                map.lock()
                    .expect("store map lock")
                    .insert(key.to_string(), value);
                Ok(())
            }
        }
    }

    /// `state::delete` — returns the deleted value, `None` when nothing was
    /// stored (the state worker reads before deleting).
    async fn state_delete(&self, key: &str) -> Result<Option<Value>, String> {
        match &self.backend {
            Backend::Bus(iii) => {
                let value = Self::bus_call(
                    iii,
                    "state::delete",
                    json!({ "scope": STATE_SCOPE, "key": key }),
                )
                .await?;
                Ok(if value.is_null() { None } else { Some(value) })
            }
            Backend::Memory(map) => Ok(map.lock().expect("store map lock").remove(key)),
        }
    }

    /// The raw index array; a missing key is an empty store, not an error.
    async fn read_index(&self) -> Result<Vec<IndexEntry>, String> {
        match self.state_get(INDEX_KEY).await? {
            None => Ok(Vec::new()),
            Some(value) => serde_json::from_value(value)
                .map_err(|e| format!("stored canvas index is malformed: {e}")),
        }
    }

    async fn write_index(&self, index: &[IndexEntry]) -> Result<(), String> {
        let value = serde_json::to_value(index).map_err(|e| format!("index serialize: {e}"))?;
        self.state_set(INDEX_KEY, value).await
    }

    /// Persist one record under its id and upsert its index row (create and
    /// update share this path). The record is written first: a failure between
    /// the two writes leaves a readable record that is momentarily unlisted,
    /// never an index row pointing at nothing.
    pub async fn save(&self, record: &CanvasRecord) -> Result<(), String> {
        let _guard = self.index_lock.lock().await;
        let value = serde_json::to_value(record).map_err(|e| format!("record serialize: {e}"))?;
        self.state_set(&record_key(&record.id), value).await?;
        let mut index = self.read_index().await?;
        index.retain(|entry| entry.id != record.id);
        index.push(IndexEntry::of(record));
        self.write_index(&index).await
    }

    /// Load one record by id. `Ok(None)` when the id is unknown.
    pub async fn load(&self, id: &str) -> Result<Option<CanvasRecord>, String> {
        match self.state_get(&record_key(id)).await? {
            None => Ok(None),
            Some(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|e| format!("stored canvas {id} is malformed: {e}")),
        }
    }

    /// Every index row, newest `updated_at` first (id breaks ties, so the
    /// order is deterministic within one second).
    pub async fn index(&self) -> Result<Vec<IndexEntry>, String> {
        let mut index = self.read_index().await?;
        index.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(index)
    }

    /// Delete one record by id, dropping its index row. `Ok(false)` when the
    /// id was unknown — the index row is still removed if one was left behind
    /// by an interrupted save.
    pub async fn delete(&self, id: &str) -> Result<bool, String> {
        let _guard = self.index_lock.lock().await;
        let old = self.state_delete(&record_key(id)).await?;
        let mut index = self.read_index().await?;
        let before = index.len();
        index.retain(|entry| entry.id != id);
        if index.len() != before {
            self.write_index(&index).await?;
        }
        Ok(old.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, name: &str, updated_at: i64) -> CanvasRecord {
        CanvasRecord {
            id: id.to_string(),
            name: name.to_string(),
            format: CanvasFormat::Mermaid,
            source: "flowchart TD\n  a --> b".to_string(),
            family: Some("flowchart".to_string()),
            created_at: updated_at,
            updated_at,
        }
    }

    /// The wire contract: field names and format values are shared with the
    /// UI and the state bus, so a rename here is a breaking change everywhere.
    #[test]
    fn record_serializes_with_the_contract_field_names() {
        let record = CanvasRecord {
            id: "abc12345".to_string(),
            name: "checkout flow".to_string(),
            format: CanvasFormat::Mermaid,
            source: "flowchart TD\n  a --> b".to_string(),
            family: Some("flowchart".to_string()),
            created_at: 1_755_000_000,
            updated_at: 1_755_000_000,
        };
        let json = serde_json::to_value(&record).expect("record serializes");
        assert_eq!(json["id"], "abc12345");
        assert_eq!(json["name"], "checkout flow");
        assert_eq!(json["format"], "mermaid");
        assert_eq!(json["source"], "flowchart TD\n  a --> b");
        assert_eq!(json["family"], "flowchart");
        assert_eq!(json["created_at"], 1_755_000_000);
        assert_eq!(json["updated_at"], 1_755_000_000);
    }

    #[test]
    fn freeform_format_serializes_lowercase_with_null_family() {
        let record = CanvasRecord {
            id: "xyz98765".to_string(),
            name: "whiteboard".to_string(),
            format: CanvasFormat::Freeform,
            source: "{\"elements\":[]}".to_string(),
            family: None,
            created_at: 0,
            updated_at: 0,
        };
        let json = serde_json::to_value(&record).expect("record serializes");
        assert_eq!(json["format"], "freeform");
        assert!(json["family"].is_null());
    }

    #[test]
    fn record_round_trips_through_json() {
        let record = CanvasRecord {
            id: "abc12345".to_string(),
            name: "n".to_string(),
            format: CanvasFormat::Freeform,
            source: "{}".to_string(),
            family: None,
            created_at: 1,
            updated_at: 2,
        };
        let back: CanvasRecord =
            serde_json::from_value(serde_json::to_value(&record).expect("serializes"))
                .expect("round trips");
        assert_eq!(record, back);
    }

    /// The index entry's field names are part of the stored contract too — a
    /// rename orphans every existing index.
    #[test]
    fn index_entry_serializes_with_the_contract_field_names() {
        let entry = IndexEntry::of(&record("abc12345", "flow", 7));
        let json = serde_json::to_value(&entry).expect("entry serializes");
        assert_eq!(json["id"], "abc12345");
        assert_eq!(json["name"], "flow");
        assert_eq!(json["format"], "mermaid");
        assert_eq!(json["updated_at"], 7);
    }

    #[tokio::test]
    async fn save_then_load_round_trips_and_missing_ids_are_none() {
        let store = Store::in_memory();
        let rec = record("aaaaaaaa", "one", 10);
        store.save(&rec).await.expect("save");
        assert_eq!(store.load("aaaaaaaa").await.expect("load"), Some(rec));
        assert_eq!(store.load("missing1").await.expect("load"), None);
    }

    #[tokio::test]
    async fn saving_twice_upserts_one_index_row() {
        let store = Store::in_memory();
        store
            .save(&record("aaaaaaaa", "before", 10))
            .await
            .expect("save");
        store
            .save(&record("aaaaaaaa", "after", 20))
            .await
            .expect("save");
        let index = store.index().await.expect("index");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].name, "after");
        assert_eq!(index[0].updated_at, 20);
    }

    #[tokio::test]
    async fn index_is_sorted_newest_first() {
        let store = Store::in_memory();
        store
            .save(&record("aaaaaaaa", "old", 10))
            .await
            .expect("save");
        store
            .save(&record("bbbbbbbb", "new", 30))
            .await
            .expect("save");
        store
            .save(&record("cccccccc", "mid", 20))
            .await
            .expect("save");
        let index = store.index().await.expect("index");
        let ids: Vec<&str> = index.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(ids, vec!["bbbbbbbb", "cccccccc", "aaaaaaaa"]);
    }

    #[tokio::test]
    async fn delete_removes_the_record_and_its_index_row() {
        let store = Store::in_memory();
        store
            .save(&record("aaaaaaaa", "one", 10))
            .await
            .expect("save");
        store
            .save(&record("bbbbbbbb", "two", 20))
            .await
            .expect("save");

        assert!(store.delete("aaaaaaaa").await.expect("delete"));
        assert_eq!(store.load("aaaaaaaa").await.expect("load"), None);
        let index = store.index().await.expect("index");
        assert_eq!(index.len(), 1);
        assert_eq!(index[0].id, "bbbbbbbb");

        assert!(!store.delete("aaaaaaaa").await.expect("second delete"));
    }
}
