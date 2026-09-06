//! Per-pane UI state for the explorer page — the browsed folder, the open
//! editor tabs, expanded folders, sidebar view, diff options, terminal
//! layout — kept under the worker's DATA directory, never in the
//! `configuration` store.
//!
//! Why not `configuration`: the engine's configuration worker persists one
//! YAML per entry under the Compose project's `./config/`, the folder an
//! operator commits to git (worker policies, provider settings, jail
//! roots). What one developer had open in one console pane is theirs
//! alone and changes on every click; it belongs with the rest of the
//! worker's runtime state under `./data/` (gitignored, next to the `turns`
//! store). Until 0.12.x the page read-modify-wrote a single `shell-ui`
//! entry holding EVERY pane, so two panes saving at once clobbered each
//! other (the whole map was last-write-wins) and every state change
//! rewrote a committable file.
//!
//! Layout: `<data_dir>/panes/<encoded key>.json`, one file per pane key
//! (the console's pane id; the workspace tab id for saves made by older
//! consoles, which `get` still reads as a fallback). A write replaces only
//! that pane's file — temp + rename, so a reader never sees a torn
//! document — under one store-wide mutex: panes cannot lose each other's
//! state, and two writers of ONE pane serialize (last write wins for that
//! pane only, which is the same pane's own later state).
//!
//! Migration: at boot, before the functions are exposed, the legacy
//! `shell-ui` configuration entry is read once; every pane it holds that
//! has no file yet is imported, and the entry's value is then blanked so
//! `config/shell-ui.yaml` stops carrying developer state (the file itself
//! can be deleted by hand; the worker no longer registers the entry).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use tokio::sync::Mutex;

/// Where the store lives, relative to the Compose project directory
/// (`III_COMPOSE_DIR`, else the process directory — `iii_worker_paths`).
pub const DEFAULT_DATA_DIR: &str = "data/shell/ui-state";
/// The configuration entry the page persisted to before the data store.
pub const LEGACY_CONFIG_ID: &str = "shell-ui";
/// Largest state document one pane may store.
pub const MAX_STATE_BYTES: usize = 4 * 1024 * 1024;
/// Longest pane key accepted.
pub const MAX_KEY_BYTES: usize = 512;

pub const GET_FN_ID: &str = "shell::ui-state::get";
pub const SET_FN_ID: &str = "shell::ui-state::set";

pub struct UiStateStore {
    dir: PathBuf,
    /// Serializes every mutation: pane writes and the legacy import. Reads
    /// go lock-free — a rename is atomic, so a read sees the old document
    /// or the new one, never a mix.
    write_lock: Mutex<()>,
}

/// What a legacy import did with each entry of the `shell-ui` map.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LegacyImport {
    /// Panes written from the entry.
    pub imported: usize,
    /// Panes that already had a file (the file wins: it is newer).
    pub kept: usize,
    /// Entries that were not usable (bad key, not an object, too large).
    pub ignored: usize,
}

impl UiStateStore {
    pub fn new(dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            dir,
            write_lock: Mutex::new(()),
        })
    }

    /// The store at its default location, resolved against the Compose
    /// project directory.
    pub fn open_default() -> Arc<Self> {
        Self::new(iii_worker_paths::resolve_path(DEFAULT_DATA_DIR))
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn pane_path(&self, key: &str) -> PathBuf {
        self.dir
            .join("panes")
            .join(format!("{}.json", encode_key(key)))
    }

    /// The stored state for `key`, else for `legacy_key` (the workspace tab
    /// id saves were keyed by before panes had ids), else `None`.
    pub async fn get(&self, key: &str, legacy_key: Option<&str>) -> Result<Option<Value>, Error> {
        validate_key(key)?;
        if let Some(state) = read_pane(&self.pane_path(key)).await? {
            return Ok(Some(state));
        }
        match legacy_key {
            Some(legacy) if legacy != key && validate_key(legacy).is_ok() => {
                read_pane(&self.pane_path(legacy)).await
            }
            _ => Ok(None),
        }
    }

    /// Replace the state stored for `key`. Returns the document size.
    pub async fn set(&self, key: &str, state: &Value) -> Result<usize, Error> {
        validate_key(key)?;
        let bytes = encode_state(state)?;
        let _guard = self.write_lock.lock().await;
        write_atomic(&self.pane_path(key), &bytes).await?;
        Ok(bytes.len())
    }

    /// Import the panes of a legacy `{ [key]: state }` map. Only keys with
    /// no file yet are written: a file is a save made AFTER the map was
    /// last written, so it is the newer state.
    pub async fn import_legacy(&self, tabs: &Map<String, Value>) -> Result<LegacyImport, Error> {
        let _guard = self.write_lock.lock().await;
        let mut report = LegacyImport::default();
        for (key, state) in tabs {
            if validate_key(key).is_err() {
                report.ignored += 1;
                continue;
            }
            let Ok(bytes) = encode_state(state) else {
                report.ignored += 1;
                continue;
            };
            let path = self.pane_path(key);
            if tokio::fs::metadata(&path).await.is_ok() {
                report.kept += 1;
                continue;
            }
            write_atomic(&path, &bytes).await?;
            report.imported += 1;
        }
        Ok(report)
    }

    /// One-time move off the `shell-ui` configuration entry. Runs at boot
    /// BEFORE `register`, so no page can read an empty store and save its
    /// defaults over state that was about to be imported. Every failure is
    /// a warning, never fatal: the entry is only blanked after a successful
    /// import, so the next boot picks up where this one left off.
    pub async fn migrate_legacy(&self, iii: &IIIClient) {
        let value = match crate::configuration::try_get_value(iii, LEGACY_CONFIG_ID, true).await {
            Ok(Some(value)) => value,
            // Not registered on this engine: nothing was ever stored.
            Ok(None) => return,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    "could not read the legacy shell-ui configuration entry; explorer pane \
                     state it may hold is not migrated this boot"
                );
                return;
            }
        };
        let Some(tabs) = value.get("tabs").and_then(Value::as_object) else {
            return;
        };
        if tabs.is_empty() {
            return;
        }
        let report = match self.import_legacy(tabs).await {
            Ok(report) => report,
            Err(error) => {
                tracing::warn!(
                    error = %error,
                    dir = %self.dir.display(),
                    "importing explorer pane state from the shell-ui configuration entry failed"
                );
                return;
            }
        };
        tracing::info!(
            imported = report.imported,
            kept = report.kept,
            ignored = report.ignored,
            dir = %self.dir.display(),
            "explorer pane state moved from the shell-ui configuration entry to the data directory"
        );
        match crate::configuration::trigger_configuration_with_retry(
            iii,
            "configuration::set",
            json!({ "id": LEGACY_CONFIG_ID, "value": {} }),
        )
        .await
        {
            Ok(_) => tracing::info!(
                "legacy shell-ui configuration entry blanked; config/shell-ui.yaml can be deleted"
            ),
            Err(error) => tracing::warn!(
                error = %error,
                "could not blank the legacy shell-ui configuration entry; it will be retried \
                 next boot"
            ),
        }
    }
}

/// A pane key as a file name: letters, digits, `-` and `_` stay, every
/// other byte becomes `%XX` (`%` included, so the mapping is injective —
/// `a:b` and `a-b` and `a%3Ab` all land on different files).
pub fn encode_key(key: &str) -> String {
    let mut out = String::with_capacity(key.len());
    for byte in key.bytes() {
        if byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_' {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn validate_key(key: &str) -> Result<(), Error> {
    if key.is_empty() {
        return Err(Error::Handler("ui state key must not be empty".into()));
    }
    if key.len() > MAX_KEY_BYTES {
        return Err(Error::Handler(format!(
            "ui state key is too long ({} bytes; at most {MAX_KEY_BYTES})",
            key.len()
        )));
    }
    Ok(())
}

fn encode_state(state: &Value) -> Result<Vec<u8>, Error> {
    if !state.is_object() {
        return Err(Error::Handler("ui state must be a JSON object".into()));
    }
    let bytes = serde_json::to_vec(state).map_err(|e| Error::Handler(e.to_string()))?;
    if bytes.len() > MAX_STATE_BYTES {
        return Err(Error::Handler(format!(
            "ui state is too large ({} bytes; at most {MAX_STATE_BYTES})",
            bytes.len()
        )));
    }
    Ok(bytes)
}

/// A document that is missing, unreadable or not an object reads as
/// "nothing stored": the page boots fresh and its next save replaces the
/// file. Only an I/O failure other than "not there" is an error.
async fn read_pane(path: &Path) -> Result<Option<Value>, Error> {
    match tokio::fs::read(path).await {
        Ok(bytes) => match serde_json::from_slice::<Value>(&bytes) {
            Ok(value) if value.is_object() => Ok(Some(value)),
            Ok(_) => {
                tracing::warn!(path = %path.display(), "pane state is not an object; ignoring it");
                Ok(None)
            }
            Err(error) => {
                tracing::warn!(
                    path = %path.display(),
                    error = %error,
                    "pane state is not valid JSON; ignoring it"
                );
                Ok(None)
            }
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(Error::Handler(format!(
            "ui state read failed ({}): {error}",
            path.display()
        ))),
    }
}

/// Write through a sibling temp file and rename it over `path`. The temp
/// name carries the `.tmp.<32 hex>` shape `events::is_own_temp` filters, so
/// a `shell::changed` watch over the data directory never reports it.
async fn write_atomic(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    let io = |what: &str, error: std::io::Error| {
        Error::Handler(format!(
            "ui state {what} failed ({}): {error}",
            path.display()
        ))
    };
    let parent = path
        .parent()
        .ok_or_else(|| Error::Handler("ui state path has no parent".into()))?;
    tokio::fs::create_dir_all(parent)
        .await
        .map_err(|e| io("directory create", e))?;
    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("pane.json");
    let tmp = parent.join(format!("{file_name}.tmp.{}", uuid::Uuid::new_v4().simple()));
    if let Err(error) = tokio::fs::write(&tmp, bytes).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(io("write", error));
    }
    if let Err(error) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(io("rename", error));
    }
    Ok(())
}

// ── functions ─────────────────────────────────────────────────────────

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRequest {
    /// The pane's state key (the console pane id).
    pub key: String,
    /// The workspace tab id: saves made before panes had ids are read from
    /// here when `key` has nothing stored.
    #[serde(default)]
    pub legacy_key: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct GetResponse {
    pub key: String,
    /// The stored object, or null when nothing is stored.
    pub state: Option<Value>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetRequest {
    pub key: String,
    /// The pane's whole state; replaces what was stored.
    pub state: Value,
}

#[derive(Debug, Serialize, JsonSchema, PartialEq, Eq)]
pub struct SetResponse {
    pub key: String,
    /// Size of the stored document.
    pub bytes: usize,
}

/// Register `shell::ui-state::get` / `set`. Call after `migrate_legacy`.
pub fn register(iii: &IIIClient, store: Arc<UiStateStore>) {
    {
        let store = store.clone();
        iii.register_function(
            GET_FN_ID,
            RegisterFunction::new_async(move |req: GetRequest| {
                let store = store.clone();
                crate::telemetry::record_call(GET_FN_ID, async move {
                    let state = store.get(&req.key, req.legacy_key.as_deref()).await?;
                    Ok(GetResponse {
                        key: req.key,
                        state,
                    })
                })
            })
            .description(
                "Console-only: the explorer page's stored state for one pane (`key` is the \
                 console pane id; `legacy_key`, the workspace tab id, is read when the pane has \
                 nothing stored). `state` is null when nothing is stored. Kept under the \
                 worker's data directory, per pane.",
            )
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let store = store.clone();
        iii.register_function(
            SET_FN_ID,
            RegisterFunction::new_async(move |req: SetRequest| {
                let store = store.clone();
                crate::telemetry::record_call(SET_FN_ID, async move {
                    let bytes = store.set(&req.key, &req.state).await?;
                    Ok(SetResponse {
                        key: req.key,
                        bytes,
                    })
                })
            })
            .description(
                "Console-only: replace the explorer page's stored state for one pane. Writes \
                 only that pane's file, atomically, so panes never clobber each other.",
            )
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, Arc<UiStateStore>) {
        let dir = tempfile::tempdir().unwrap();
        let store = UiStateStore::new(dir.path().join("ui-state"));
        (dir, store)
    }

    fn state(root: &str) -> Value {
        json!({ "root": root, "open": [], "active": null, "expanded": ["src"] })
    }

    #[test]
    fn encode_key_keeps_keys_apart_and_off_the_path() {
        assert_eq!(encode_key("tab-1"), "tab-1");
        assert_eq!(encode_key("tab-1:pane:0"), "tab-1%3Apane%3A0");
        assert_eq!(encode_key("a%3Ab"), "a%253Ab");
        assert_ne!(encode_key("a:b"), encode_key("a-b"));
        assert_ne!(encode_key("a:b"), encode_key("a%3Ab"));
        let escaped = encode_key("../../etc/passwd");
        assert!(!escaped.contains('/'));
        assert!(!escaped.contains(".."));
    }

    #[tokio::test]
    async fn roundtrip_and_legacy_fallback() {
        let (_dir, store) = store();
        assert_eq!(
            store.get("tab-1:pane:0", Some("tab-1")).await.unwrap(),
            None
        );

        store.set("tab-1", &state("/legacy")).await.unwrap();
        // The pane has nothing of its own yet: the workspace tab's save is read.
        assert_eq!(
            store.get("tab-1:pane:0", Some("tab-1")).await.unwrap(),
            Some(state("/legacy"))
        );
        assert_eq!(store.get("tab-1:pane:0", None).await.unwrap(), None);

        store.set("tab-1:pane:0", &state("/own")).await.unwrap();
        assert_eq!(
            store.get("tab-1:pane:0", Some("tab-1")).await.unwrap(),
            Some(state("/own"))
        );
        // The other pane of the same tab still sees the tab-level save.
        assert_eq!(
            store.get("tab-1:pane:1", Some("tab-1")).await.unwrap(),
            Some(state("/legacy"))
        );
        // One file per key, under the store's panes directory.
        assert!(store.dir().join("panes/tab-1.json").is_file());
        assert!(store.dir().join("panes/tab-1%3Apane%3A0.json").is_file());
    }

    #[tokio::test]
    async fn missing_and_corrupt_documents_read_as_nothing_stored() {
        let (_dir, store) = store();
        let panes = store.dir().join("panes");
        std::fs::create_dir_all(&panes).unwrap();
        std::fs::write(panes.join("broken.json"), b"{ not json").unwrap();
        std::fs::write(panes.join("list.json"), b"[1, 2]").unwrap();
        assert_eq!(store.get("broken", None).await.unwrap(), None);
        assert_eq!(store.get("list", None).await.unwrap(), None);
        assert_eq!(store.get("never", None).await.unwrap(), None);
        // A save replaces the broken document.
        store.set("broken", &state("/fixed")).await.unwrap();
        assert_eq!(
            store.get("broken", None).await.unwrap(),
            Some(state("/fixed"))
        );
    }

    #[tokio::test]
    async fn rejects_bad_keys_and_non_object_state() {
        let (_dir, store) = store();
        assert!(store.set("", &state("/x")).await.is_err());
        assert!(store.get("", None).await.is_err());
        let long = "k".repeat(MAX_KEY_BYTES + 1);
        assert!(store.set(&long, &state("/x")).await.is_err());
        assert!(store.set("tab-1", &json!([1])).await.is_err());
        assert!(store.set("tab-1", &Value::Null).await.is_err());
        assert!(store.set("tab-1", &json!("text")).await.is_err());
        let huge = json!({ "blob": "x".repeat(MAX_STATE_BYTES) });
        assert!(store.set("tab-1", &huge).await.is_err());
        // Nothing was written by the rejected calls.
        assert!(!store.dir().join("panes").exists());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_writers_leave_whole_documents_and_no_temps() {
        let (_dir, store) = store();
        let mut tasks = Vec::new();
        for i in 0..48u32 {
            let store = store.clone();
            tasks.push(tokio::spawn(async move {
                // Two panes interleaving: neither may lose the other's file.
                let key = if i % 2 == 0 { "pane-a" } else { "pane-b" };
                let body =
                    json!({ "root": format!("/r{i}"), "pad": "x".repeat(2_000 + i as usize) });
                store.set(key, &body).await.unwrap();
            }));
        }
        for task in tasks {
            task.await.unwrap();
        }
        for key in ["pane-a", "pane-b"] {
            let stored = store
                .get(key, None)
                .await
                .unwrap()
                .expect("both panes stored");
            // Whichever write landed last, its two fields belong together.
            let i: usize = stored["root"].as_str().unwrap()[2..].parse().unwrap();
            assert_eq!(stored["pad"].as_str().unwrap().len(), 2_000 + i);
        }
        let leftovers: Vec<_> = std::fs::read_dir(store.dir().join("panes"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.contains(".tmp."))
            .collect();
        assert!(
            leftovers.is_empty(),
            "temp files left behind: {leftovers:?}"
        );
    }

    #[tokio::test]
    async fn legacy_import_fills_gaps_only() {
        let (_dir, store) = store();
        store.set("tab-1:pane:0", &state("/newer")).await.unwrap();
        let mut tabs = Map::new();
        tabs.insert("tab-1:pane:0".into(), state("/older"));
        tabs.insert("tab-1".into(), state("/tab"));
        tabs.insert("tab-2:pane:0".into(), state("/two"));
        tabs.insert("".into(), state("/nokey"));
        tabs.insert("junk".into(), json!("not an object"));

        let report = store.import_legacy(&tabs).await.unwrap();
        assert_eq!(
            report,
            LegacyImport {
                imported: 2,
                kept: 1,
                ignored: 2
            }
        );
        // The file written after the map wins over the map's copy.
        assert_eq!(
            store.get("tab-1:pane:0", None).await.unwrap(),
            Some(state("/newer"))
        );
        assert_eq!(store.get("tab-1", None).await.unwrap(), Some(state("/tab")));
        assert_eq!(
            store.get("tab-2:pane:0", None).await.unwrap(),
            Some(state("/two"))
        );
        assert_eq!(store.get("junk", None).await.unwrap(), None);

        // Running it again changes nothing.
        let again = store.import_legacy(&tabs).await.unwrap();
        assert_eq!(
            again,
            LegacyImport {
                imported: 0,
                kept: 3,
                ignored: 2
            }
        );
    }
}
