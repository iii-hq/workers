//! Artifact, version and share records, and their persistence over the state
//! bus.
//!
//! Everything lives in the `state` worker under scope [`STATE_SCOPE`]:
//!
//! - `artifact/<id>` — one [`ArtifactRecord`] (metadata and version list)
//! - `content/<id>/<version>` — the content of one version, a JSON string
//! - `index` — an array of [`IndexEntry`] summaries, so listing never loads
//!   content
//! - `share/<token>` — one [`ShareRecord`], the share server's lookup key
//! - `shares` — an array of every live [`ShareRecord`]
//! - `tunnel/lease` — the quick-tunnel [`LeaseRecord`] this worker holds
//!
//! Versions are immutable: an update writes a new `content/<id>/<n>` and never
//! rewrites an old one, which is what lets a share freeze a version.
//!
//! This worker is the only writer of the scope, so read-modify-write cycles
//! are serialized with one in-process mutex ([`Store::lock`]). The share
//! server only reads.

use std::collections::HashMap;
use std::sync::Arc;

use iii_sdk::{CallRequest, IIIClient};
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::{Mutex, MutexGuard};

/// The `state` worker scope every record is stored under.
pub const STATE_SCOPE: &str = "artifact";

const INDEX_KEY: &str = "index";
const SHARES_KEY: &str = "shares";
const LEASE_KEY: &str = "tunnel/lease";

/// Timeout for one `state::*` bus call.
const STATE_TIMEOUT_MS: u64 = 10_000;

fn artifact_key(id: &str) -> String {
    format!("artifact/{id}")
}

fn content_key(id: &str, version: u32) -> String {
    format!("content/{id}/{version}")
}

fn share_key(token: &str) -> String {
    format!("share/{token}")
}

/// How an artifact's content is rendered when it is shared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ArtifactFormat {
    /// A self-contained HTML document (inline CSS and JS), served as is in a
    /// sandbox with no network access. Designs and clickable prototypes.
    Html,
    /// Markdown, rendered to a clean page with raw HTML escaped. RFCs, docs.
    Markdown,
    /// An SVG image. Drawings, diagrams, icons.
    Svg,
}

/// One stored version of an artifact (its content lives under its own key).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct VersionMeta {
    /// Version number, starting at 1.
    pub version: u32,
    /// Creation time, unix seconds.
    pub created_at: i64,
    /// Content size in bytes.
    pub bytes: usize,
    /// What changed in this version, when the caller said.
    pub note: Option<String>,
}

/// One stored artifact. The id never changes; every update appends a version.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ArtifactRecord {
    /// Stable 8-character id.
    pub id: String,
    /// Human-readable title.
    pub title: String,
    /// Free-form category, lowercase: `design`, `prototype`, `rfc`, `doc`, …
    pub kind: String,
    /// How the content renders: `html`, `markdown` or `svg`.
    pub format: ArtifactFormat,
    /// Highest version number.
    pub latest_version: u32,
    /// Every version, oldest first.
    pub versions: Vec<VersionMeta>,
    /// Creation time, unix seconds.
    pub created_at: i64,
    /// Last update time, unix seconds.
    pub updated_at: i64,
}

/// One row of the artifact index: enough to list without loading content.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct IndexEntry {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub format: ArtifactFormat,
    pub latest_version: u32,
    pub updated_at: i64,
}

impl IndexEntry {
    pub fn of(record: &ArtifactRecord) -> Self {
        Self {
            id: record.id.clone(),
            title: record.title.clone(),
            kind: record.kind.clone(),
            format: record.format,
            latest_version: record.latest_version,
            updated_at: record.updated_at,
        }
    }
}

/// One share: a frozen version reachable at `/s/<token>` until `expires_at`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ShareRecord {
    /// 64-hex-character bearer token; whoever holds the link can read it.
    pub token: String,
    pub artifact_id: String,
    /// The version this share serves, fixed when the share was made.
    pub version: u32,
    /// Whether the share is published through the quick-tunnel.
    pub public: bool,
    /// Creation time, unix seconds.
    pub created_at: i64,
    /// Expiry, unix seconds; the link answers `410 Gone` from then on.
    pub expires_at: i64,
}

impl ShareRecord {
    pub fn is_live(&self, now: i64) -> bool {
        self.expires_at > now
    }
}

/// The quick-tunnel lease this worker holds for public shares.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LeaseRecord {
    pub lease_id: String,
    pub tunnel_id: String,
    /// Lease expiry, unix seconds.
    pub expires_at: i64,
    /// When the last public share ended (unix seconds); the lease is
    /// released once the configured grace has passed since then.
    #[serde(default)]
    pub idle_since: Option<i64>,
}

enum Backend {
    Bus(Arc<IIIClient>),
    Memory(std::sync::Mutex<HashMap<String, Value>>),
}

/// State-bus persistence. `Store::in_memory` is the engine-free test seam with
/// the same key semantics.
pub struct Store {
    backend: Backend,
    mutation_lock: Mutex<()>,
}

impl Store {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            backend: Backend::Bus(iii),
            mutation_lock: Mutex::new(()),
        }
    }

    pub fn in_memory() -> Self {
        Self {
            backend: Backend::Memory(std::sync::Mutex::new(HashMap::new())),
            mutation_lock: Mutex::new(()),
        }
    }

    /// Hold for the duration of any read-modify-write.
    pub async fn lock(&self) -> MutexGuard<'_, ()> {
        self.mutation_lock.lock().await
    }

    async fn bus_call(iii: &IIIClient, function_id: &str, payload: Value) -> Result<Value, String> {
        iii.call(CallRequest {
            function_id: function_id.to_string(),
            payload,
            action: None,
            timeout_ms: Some(STATE_TIMEOUT_MS),
        })
        .await
        .map_err(|e| format!("{function_id} failed: {e}"))
    }

    async fn get_raw(&self, key: &str) -> Result<Option<Value>, String> {
        match &self.backend {
            Backend::Bus(iii) => {
                let value = Self::bus_call(
                    iii,
                    "state::get",
                    json!({ "scope": STATE_SCOPE, "key": key }),
                )
                .await?;
                Ok((!value.is_null()).then_some(value))
            }
            Backend::Memory(map) => Ok(map.lock().expect("store map lock").get(key).cloned()),
        }
    }

    async fn set_raw(&self, key: &str, value: Value) -> Result<(), String> {
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

    async fn delete_raw(&self, key: &str) -> Result<(), String> {
        match &self.backend {
            Backend::Bus(iii) => Self::bus_call(
                iii,
                "state::delete",
                json!({ "scope": STATE_SCOPE, "key": key }),
            )
            .await
            .map(|_| ()),
            Backend::Memory(map) => {
                map.lock().expect("store map lock").remove(key);
                Ok(())
            }
        }
    }

    async fn get_typed<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, String> {
        match self.get_raw(key).await? {
            None => Ok(None),
            Some(value) => serde_json::from_value(value)
                .map(Some)
                .map_err(|e| format!("stored `{key}` is malformed: {e}")),
        }
    }

    async fn set_typed<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let value = serde_json::to_value(value).map_err(|e| format!("serialize `{key}`: {e}"))?;
        self.set_raw(key, value).await
    }

    // ---- artifacts -------------------------------------------------------

    pub async fn load_artifact(&self, id: &str) -> Result<Option<ArtifactRecord>, String> {
        self.get_typed(&artifact_key(id)).await
    }

    /// Write one version's content. Call before [`Self::save_artifact`] so a
    /// record never points at a version that is not stored.
    pub async fn save_content(&self, id: &str, version: u32, content: &str) -> Result<(), String> {
        self.set_raw(
            &content_key(id, version),
            Value::String(content.to_string()),
        )
        .await
    }

    pub async fn load_content(&self, id: &str, version: u32) -> Result<Option<String>, String> {
        self.get_typed(&content_key(id, version)).await
    }

    /// Persist the record and upsert its index row.
    pub async fn save_artifact(&self, record: &ArtifactRecord) -> Result<(), String> {
        self.set_typed(&artifact_key(&record.id), record).await?;
        let mut index = self.index_raw().await?;
        index.retain(|entry| entry.id != record.id);
        index.push(IndexEntry::of(record));
        self.set_typed(INDEX_KEY, &index).await
    }

    /// Delete the record, every version's content and its index row.
    pub async fn delete_artifact(&self, record: &ArtifactRecord) -> Result<(), String> {
        let mut index = self.index_raw().await?;
        index.retain(|entry| entry.id != record.id);
        self.set_typed(INDEX_KEY, &index).await?;
        self.delete_raw(&artifact_key(&record.id)).await?;
        for meta in &record.versions {
            self.delete_raw(&content_key(&record.id, meta.version))
                .await?;
        }
        Ok(())
    }

    async fn index_raw(&self) -> Result<Vec<IndexEntry>, String> {
        Ok(self.get_typed(INDEX_KEY).await?.unwrap_or_default())
    }

    /// Every index row, most recently updated first (id breaks ties).
    pub async fn index(&self) -> Result<Vec<IndexEntry>, String> {
        let mut index = self.index_raw().await?;
        index.sort_by(|a, b| {
            b.updated_at
                .cmp(&a.updated_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(index)
    }

    // ---- shares ----------------------------------------------------------

    pub async fn load_share(&self, token: &str) -> Result<Option<ShareRecord>, String> {
        self.get_typed(&share_key(token)).await
    }

    pub async fn shares(&self) -> Result<Vec<ShareRecord>, String> {
        Ok(self.get_typed(SHARES_KEY).await?.unwrap_or_default())
    }

    pub async fn save_share(&self, share: &ShareRecord) -> Result<(), String> {
        self.set_typed(&share_key(&share.token), share).await?;
        let mut shares = self.shares().await?;
        shares.retain(|s| s.token != share.token);
        shares.push(share.clone());
        self.set_typed(SHARES_KEY, &shares).await
    }

    /// Remove every share `drop` selects; returns the removed ones. The list
    /// row goes first, so an interrupted removal never leaves a listed share
    /// whose link is gone.
    pub async fn remove_shares(
        &self,
        drop: impl Fn(&ShareRecord) -> bool,
    ) -> Result<Vec<ShareRecord>, String> {
        let shares = self.shares().await?;
        let (removed, kept): (Vec<_>, Vec<_>) = shares.into_iter().partition(|s| drop(s));
        if removed.is_empty() {
            return Ok(removed);
        }
        self.set_typed(SHARES_KEY, &kept).await?;
        for share in &removed {
            self.delete_raw(&share_key(&share.token)).await?;
        }
        Ok(removed)
    }

    // ---- tunnel lease ----------------------------------------------------

    pub async fn lease(&self) -> Result<Option<LeaseRecord>, String> {
        self.get_typed(LEASE_KEY).await
    }

    pub async fn save_lease(&self, lease: &LeaseRecord) -> Result<(), String> {
        self.set_typed(LEASE_KEY, lease).await
    }

    pub async fn clear_lease(&self) -> Result<(), String> {
        self.delete_raw(LEASE_KEY).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(id: &str, updated_at: i64) -> ArtifactRecord {
        ArtifactRecord {
            id: id.to_string(),
            title: format!("title {id}"),
            kind: "design".to_string(),
            format: ArtifactFormat::Html,
            latest_version: 1,
            versions: vec![VersionMeta {
                version: 1,
                created_at: updated_at,
                bytes: 4,
                note: None,
            }],
            created_at: updated_at,
            updated_at,
        }
    }

    fn share(token: &str, expires_at: i64) -> ShareRecord {
        ShareRecord {
            token: token.to_string(),
            artifact_id: "aaaaaaaa".to_string(),
            version: 1,
            public: false,
            created_at: 0,
            expires_at,
        }
    }

    #[test]
    fn records_serialize_with_the_contract_field_names() {
        let json = serde_json::to_value(record("abc12345", 7)).expect("serializes");
        assert_eq!(json["format"], "html");
        assert_eq!(json["latest_version"], 1);
        assert_eq!(json["versions"][0]["version"], 1);
        assert_eq!(json["kind"], "design");
    }

    #[tokio::test]
    async fn artifact_content_and_index_round_trip() {
        let store = Store::in_memory();
        let rec = record("aaaaaaaa", 10);
        store
            .save_content("aaaaaaaa", 1, "<p>")
            .await
            .expect("content");
        store.save_artifact(&rec).await.expect("save");
        assert_eq!(
            store.load_artifact("aaaaaaaa").await.expect("load"),
            Some(rec)
        );
        assert_eq!(
            store.load_content("aaaaaaaa", 1).await.expect("load"),
            Some("<p>".to_string())
        );
        assert_eq!(store.index().await.expect("index").len(), 1);
    }

    #[tokio::test]
    async fn index_is_newest_first_and_upserts() {
        let store = Store::in_memory();
        store
            .save_artifact(&record("aaaaaaaa", 10))
            .await
            .expect("save");
        store
            .save_artifact(&record("bbbbbbbb", 30))
            .await
            .expect("save");
        store
            .save_artifact(&record("aaaaaaaa", 40))
            .await
            .expect("save");
        let ids: Vec<String> = store
            .index()
            .await
            .expect("index")
            .into_iter()
            .map(|e| e.id)
            .collect();
        assert_eq!(ids, vec!["aaaaaaaa", "bbbbbbbb"]);
    }

    #[tokio::test]
    async fn delete_removes_record_content_and_row() {
        let store = Store::in_memory();
        let rec = record("aaaaaaaa", 10);
        store
            .save_content("aaaaaaaa", 1, "x")
            .await
            .expect("content");
        store.save_artifact(&rec).await.expect("save");
        store.delete_artifact(&rec).await.expect("delete");
        assert_eq!(store.load_artifact("aaaaaaaa").await.expect("load"), None);
        assert_eq!(store.load_content("aaaaaaaa", 1).await.expect("load"), None);
        assert!(store.index().await.expect("index").is_empty());
    }

    #[tokio::test]
    async fn remove_shares_drops_both_the_link_and_the_row() {
        let store = Store::in_memory();
        store.save_share(&share("t1", 100)).await.expect("save");
        store.save_share(&share("t2", 300)).await.expect("save");
        let removed = store
            .remove_shares(|s| !s.is_live(200))
            .await
            .expect("remove");
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].token, "t1");
        assert_eq!(store.load_share("t1").await.expect("load"), None);
        assert!(store.load_share("t2").await.expect("load").is_some());
        assert_eq!(store.shares().await.expect("list").len(), 1);
    }
}
