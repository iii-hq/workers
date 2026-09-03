//! Durable per-session change history.
//!
//! The console's review pane learned about an agent's edits from two live-only
//! channels: the `shell::changed` watcher, which carries no session, and the
//! harness turn events, which only reach a page that is open at the time. A
//! chat reopened later had nothing to show. This module records every write
//! that crosses the bus through a shell or coder function, tagged with the
//! session and turn the harness hook envelope names, and stores it in the
//! `state` worker before anything else happens, so a surface that opens after
//! the work can still ask what a session did.
//!
//! Two harness hooks do the work. `pre-trigger` reads the file the call is
//! about to change and keeps that pre-image aside; `post-trigger` records the
//! change together with it. Both are advisory and fail open: a viewer must
//! never be able to block a write. Writes that bypass these functions (a
//! `sed -i` through `shell::exec`, a formatter) are not seen here.
//!
//! Storage is this worker's own data directory, never the state worker: one
//! small JSON record per session (paths, kinds, revisions) and a
//! content-addressed blob store for pre-image bodies, shared across turns and
//! sessions and pruned oldest-first past a byte cap, so history cannot grow
//! into the engine's state.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;

use crate::code::functions::create_file::content_revision;
use crate::config::TurnsConfig;

const PRE_HOOK_FN_ID: &str = "shell::turns::on-pre-trigger";
const POST_HOOK_FN_ID: &str = "shell::turns::on-post-trigger";
const STARTED_FN_ID: &str = "shell::turns::on-turn-started";
const COMPLETED_FN_ID: &str = "shell::turns::on-turn-completed";
const ADOPT_ROOT_FN_ID: &str = "shell::turns::adopt-root";
// Every shell call is hooked, not only the file verbs: calls that name no
// paths (`shell::exec`, pty writes) still tell the observer which
// session, turn and root are live so the workspace watch can start.
const HOOK_FUNCTIONS: &[&str] = &["shell::*", "coder::*"];
const HOOK_TIMEOUT_MS: u64 = 3_000;

pub const MAX_TURNS_PER_SESSION: usize = 40;
pub const MAX_FILES_PER_TURN: usize = 400;
pub const MAX_PRE_IMAGE_BYTES: usize = 64 * 1024;
/// Bodies inflated into one `shell::turns::get` response.
pub const MAX_RESPONSE_BODY_BYTES: usize = 1024 * 1024;

/// A session id as a file name: ids made of safe characters stay readable,
/// anything else becomes a digest so no id can escape the directory.
pub fn session_file_stem(session_id: &str) -> String {
    let safe = !session_id.is_empty()
        && session_id.len() <= 128
        && session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        && !session_id.starts_with('.');
    if safe {
        session_id.to_string()
    } else {
        format!("h-{:x}", Sha256::digest(session_id.as_bytes()))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct PreImage {
    /// `sha256:<hex>` of the bytes before the write; absent when the file
    /// did not exist or could not be read. Doubles as the blob key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    /// UTF-8 body before the write. Never stored in the record; filled in by
    /// `shell::turns::get` from the blob store, up to `MAX_PRE_IMAGE_BYTES`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The body was larger than the cap (not kept) or did not fit the
    /// response budget; the revision still identifies it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// A blob for `revision` exists in the store.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub stored: bool,
    /// The path did not exist before the call.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub missing: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub binary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct FileRecord {
    /// Absolute path on the host.
    pub path: String,
    /// The session's workspace root at the time, when the harness named one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
    /// `created` | `modified` | `deleted` | `moved`.
    pub kind: String,
    /// Function id that made the change.
    pub cause: String,
    pub first_seen: u64,
    pub last_seen: u64,
    /// Set for a `moved` record: where the file came from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<PreImage>,
    /// Revision after the last write, when the file could be read back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_revision: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct TurnRecord {
    pub turn_id: String,
    pub started_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    #[serde(default)]
    pub files: Vec<FileRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
pub struct SessionRecord {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub turns: Vec<TurnRecord>,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct HookCall {
    pub function_id: String,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct HookResult {
    #[serde(default)]
    pub is_error: bool,
}

/// The part of the harness hook envelope this module reads.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct HookInput {
    #[serde(default)]
    pub metadata: Option<Value>,
    #[serde(default)]
    pub call: Option<HookCall>,
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(default)]
    pub result: Option<HookResult>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct HookOutput {
    pub decision: &'static str,
}

impl Default for HookOutput {
    fn default() -> Self {
        Self {
            decision: "continue",
        }
    }
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct TurnEvent {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct Ack {
    pub ok: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Touch {
    pub path: String,
    pub kind: &'static str,
    pub from: Option<String>,
}

/// Every path a call writes, with the change kind. Empty for a read.
/// `shell::exec` is excluded on purpose: argv does not say what it writes.
/// `chmod` is excluded too: it changes mode, not content, so it has no diff.
///
/// The batch verbs matter here. `coder::create-file`/`update-file` carry a
/// `files: [{path}]` array, `coder::move` a `files: [{from, to}]` array,
/// `coder::delete-file` a `paths: []` array, and `shell::fs::sed` a `files`
/// array of path strings (or a single `path`). A one-file assumption would
/// silently drop every file but the first.
pub fn touches(call: &HookCall) -> Vec<Touch> {
    let args = &call.arguments;
    let str_at = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
    let one = |kind: &'static str, path: Option<String>, from: Option<String>| {
        path.map(|path| Touch { path, kind, from })
            .into_iter()
            .collect::<Vec<_>>()
    };
    let file_paths = |kind: &'static str| {
        args.get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|f| str_at(f, "path"))
                    .map(|path| Touch {
                        path,
                        kind,
                        from: None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    let string_paths = |value: Option<&Value>, kind: &'static str| {
        value
            .and_then(Value::as_array)
            .map(|paths| {
                paths
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|path| Touch {
                        path: path.to_string(),
                        kind,
                        from: None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    match call.function_id.as_str() {
        "shell::fs::write" => one("modified", str_at(args, "path"), None),
        "shell::fs::rm" => one("deleted", str_at(args, "path"), None),
        "shell::fs::mv" => one(
            "moved",
            str_at(args, "dst").or_else(|| str_at(args, "to")),
            str_at(args, "src").or_else(|| str_at(args, "from")),
        ),
        "shell::fs::sed" => {
            let mut out: Vec<Touch> = string_paths(args.get("files"), "modified");
            if out.is_empty() {
                out = one("modified", str_at(args, "path"), None);
            }
            out
        }
        "coder::create-file" => file_paths("created"),
        "coder::update-file" => file_paths("modified"),
        "coder::delete-file" => string_paths(args.get("paths"), "deleted"),
        "coder::move" => args
            .get("files")
            .and_then(Value::as_array)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|f| {
                        str_at(f, "to").map(|path| Touch {
                            path,
                            kind: "moved",
                            from: str_at(f, "from"),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub fn session_root(metadata: Option<&Value>) -> Option<String> {
    metadata?
        .get("fs_scope")?
        .get("root")?
        .as_str()
        .map(str::to_string)
}

/// Absolute path for a call argument, relative ones resolved against the
/// session root (or left alone when there is none).
pub fn absolute_path(path: &str, root: Option<&str>) -> String {
    if Path::new(path).is_absolute() {
        return path.to_string();
    }
    match root {
        Some(root) if !root.is_empty() && root != "." => PathBuf::from(root)
            .join(path)
            .to_string_lossy()
            .into_owned(),
        _ => path.to_string(),
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

fn system_time_ms(value: SystemTime) -> Option<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .ok()
        .map(|duration| duration.as_millis() as u64)
}

fn recent_enough(value: Option<u64>, started_at: u64) -> bool {
    value.is_some_and(|value| value.saturating_add(2_000) >= started_at)
}

fn adoptable_project_path(root: &Path, path: &Path) -> bool {
    let Ok(relative) = path.strip_prefix(root) else {
        return false;
    };
    let ignored = [
        ".git",
        "node_modules",
        "target",
        "dist",
        "build",
        "out",
        "vendor",
        "__pycache__",
    ];
    if relative
        .components()
        .any(|part| ignored.iter().any(|name| part.as_os_str() == *name))
    {
        return false;
    }
    !matches!(
        path.extension().and_then(|extension| extension.to_str()),
        Some(
            "o" | "a"
                | "d"
                | "rlib"
                | "rmeta"
                | "so"
                | "dylib"
                | "dll"
                | "class"
                | "pyc"
                | "wasm"
                | "map"
                | "log"
                | "tmp"
                | "swp"
                | "part"
                | "pid"
                | "sock"
        )
    )
}

fn scan_adopted_project(root: &Path, started_at: u64) -> Vec<(String, String)> {
    struct Candidate {
        path: PathBuf,
        created_at: Option<u64>,
        modified_at: Option<u64>,
    }

    let root_created_at = std::fs::metadata(root)
        .ok()
        .and_then(|metadata| metadata.created().ok())
        .and_then(system_time_ms);
    let mut stack = vec![root.to_path_buf()];
    let mut candidates = Vec::new();
    while let Some(directory) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(directory) else {
            continue;
        };
        let mut entries = entries.flatten().collect::<Vec<_>>();
        entries.sort_by_key(std::fs::DirEntry::path);
        for entry in entries {
            let path = entry.path();
            if !adoptable_project_path(root, &path) {
                continue;
            }
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_dir() {
                stack.push(path);
                continue;
            }
            if !metadata.is_file() || candidates.len() >= MAX_FILES_PER_TURN {
                continue;
            }
            candidates.push(Candidate {
                path,
                created_at: metadata.created().ok().and_then(system_time_ms),
                modified_at: metadata.modified().ok().and_then(system_time_ms),
            });
        }
    }
    let whole_project_is_new = recent_enough(root_created_at, started_at)
        || (!candidates.is_empty()
            && candidates
                .iter()
                .all(|candidate| recent_enough(candidate.modified_at, started_at)));
    candidates
        .into_iter()
        .filter(|candidate| whole_project_is_new || recent_enough(candidate.created_at, started_at))
        .filter_map(|candidate| {
            std::fs::read(&candidate.path).ok().map(|bytes| {
                (
                    candidate.path.to_string_lossy().into_owned(),
                    content_revision(&bytes),
                )
            })
        })
        .collect()
}

struct ReadImage {
    image: PreImage,
    body: Option<Vec<u8>>,
}

async fn read_pre_image(path: &str) -> ReadImage {
    let blank = PreImage {
        revision: None,
        content: None,
        truncated: false,
        stored: false,
        missing: false,
        binary: false,
    };
    match tokio::fs::read(path).await {
        Ok(bytes) => {
            let revision = Some(content_revision(&bytes));
            if std::str::from_utf8(&bytes).is_err() {
                return ReadImage {
                    image: PreImage {
                        revision,
                        binary: true,
                        ..blank
                    },
                    body: None,
                };
            }
            if bytes.len() > MAX_PRE_IMAGE_BYTES {
                return ReadImage {
                    image: PreImage {
                        revision,
                        truncated: true,
                        ..blank
                    },
                    body: None,
                };
            }
            ReadImage {
                image: PreImage { revision, ..blank },
                body: Some(bytes),
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => ReadImage {
            image: PreImage {
                missing: true,
                ..blank
            },
            body: None,
        },
        Err(_) => ReadImage {
            image: blank,
            body: None,
        },
    }
}

async fn read_revision(path: &str) -> Option<String> {
    tokio::fs::read(path)
        .await
        .ok()
        .map(|bytes| content_revision(&bytes))
}

/// Fold a change into the turn: one record per path, the first pre-image
/// wins (it is the true "before" for the turn), kinds compose (`created` then
/// `modified` stays `created`; anything then `deleted` is `deleted`).
pub fn record_file(turn: &mut TurnRecord, change: FileRecord) {
    if let Some(existing) = turn.files.iter_mut().find(|f| f.path == change.path) {
        existing.last_seen = change.last_seen;
        existing.cause = change.cause;
        existing.after_revision = change.after_revision;
        existing.kind = match (existing.kind.as_str(), change.kind.as_str()) {
            (_, "deleted") => "deleted".to_string(),
            ("created", _) => "created".to_string(),
            ("deleted", "created") | ("deleted", "modified") => "modified".to_string(),
            (_, next) => next.to_string(),
        };
        if existing.before.is_none() {
            existing.before = change.before;
        }
        if change.from.is_some() {
            existing.from = change.from;
        }
        return;
    }
    if turn.files.len() >= MAX_FILES_PER_TURN {
        return;
    }
    turn.files.push(change);
}

/// Fold a change the workspace watch saw into the turn. Unlike a hooked
/// write there is no pre-image and no function id; when the path is already
/// recorded from a hook, only the freshness and kind move — the hook's
/// cause, pre-image and after-revision are the better record.
pub fn record_observed(turn: &mut TurnRecord, path: String, root: &str, kind: &str, at: u64) {
    if let Some(existing) = turn.files.iter_mut().find(|f| f.path == path) {
        existing.last_seen = at;
        existing.kind = match (existing.kind.as_str(), kind) {
            (_, "deleted") => "deleted".to_string(),
            ("created", _) => "created".to_string(),
            ("deleted", "created") | ("deleted", "modified") => "modified".to_string(),
            (_, next) => next.to_string(),
        };
        return;
    }
    if turn.files.len() >= MAX_FILES_PER_TURN {
        return;
    }
    turn.files.push(FileRecord {
        path,
        root: Some(root.to_string()),
        kind: kind.to_string(),
        cause: "observed".to_string(),
        first_seen: at,
        last_seen: at,
        from: None,
        before: None,
        after_revision: None,
    });
}

/// Newest turns win once a session passes its cap.
pub fn enforce_budgets(record: &mut SessionRecord) {
    if record.turns.len() > MAX_TURNS_PER_SESSION {
        let excess = record.turns.len() - MAX_TURNS_PER_SESSION;
        record.turns.drain(0..excess);
    }
}

fn turn_mut<'a>(record: &'a mut SessionRecord, turn_id: &str, at: u64) -> &'a mut TurnRecord {
    if let Some(index) = record.turns.iter().position(|t| t.turn_id == turn_id) {
        return &mut record.turns[index];
    }
    record.turns.push(TurnRecord {
        turn_id: turn_id.to_string(),
        started_at: at,
        ended_at: None,
        files: Vec::new(),
    });
    record.turns.last_mut().expect("just pushed")
}

#[derive(Hash, PartialEq, Eq, Clone)]
struct PendingKey {
    session_id: String,
    turn_id: String,
    path: String,
}

/// The on-disk layout: `sessions/<stem>.json` records and `objects/<hex>`
/// pre-image blobs keyed by the content digest.
pub struct TurnStore {
    dir: PathBuf,
    max_blob_bytes: u64,
    blob_bytes: Mutex<Option<u64>>,
}

impl TurnStore {
    pub fn new(dir: PathBuf, max_blob_bytes: u64) -> Self {
        Self {
            dir,
            max_blob_bytes,
            blob_bytes: Mutex::new(None),
        }
    }

    fn session_path(&self, session_id: &str) -> PathBuf {
        self.dir
            .join("sessions")
            .join(format!("{}.json", session_file_stem(session_id)))
    }

    fn blob_path(&self, revision: &str) -> Option<PathBuf> {
        let hex = revision.strip_prefix("sha256:")?;
        if hex.len() < 4 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(self.dir.join("objects").join(&hex[..2]).join(hex))
    }

    pub async fn load(&self, session_id: &str) -> Result<SessionRecord, Error> {
        match tokio::fs::read(self.session_path(session_id)).await {
            Ok(bytes) => {
                let mut record: SessionRecord = serde_json::from_slice(&bytes).unwrap_or_default();
                record.session_id = session_id.to_string();
                Ok(record)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(SessionRecord {
                session_id: session_id.to_string(),
                turns: Vec::new(),
            }),
            Err(e) => Err(Error::Handler(format!("turn history read failed: {e}"))),
        }
    }

    pub async fn store(&self, record: &SessionRecord) -> Result<(), Error> {
        let path = self.session_path(&record.session_id);
        let io = |e: std::io::Error| Error::Handler(format!("turn history write failed: {e}"));
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(io)?;
        }
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec(record).map_err(|e| Error::Handler(e.to_string()))?;
        tokio::fs::write(&tmp, bytes).await.map_err(io)?;
        tokio::fs::rename(&tmp, &path).await.map_err(io)
    }

    /// Keep a body under its digest; a body already present is not rewritten.
    pub async fn put_blob(&self, revision: &str, body: &[u8]) -> bool {
        let Some(path) = self.blob_path(revision) else {
            return false;
        };
        if tokio::fs::metadata(&path).await.is_ok() {
            return true;
        }
        let Some(parent) = path.parent() else {
            return false;
        };
        if tokio::fs::create_dir_all(parent).await.is_err() {
            return false;
        }
        let tmp = parent.join(format!(
            ".{}.tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("blob")
        ));
        if tokio::fs::write(&tmp, body).await.is_err() {
            return false;
        }
        if tokio::fs::rename(&tmp, &path).await.is_err() {
            let _ = tokio::fs::remove_file(&tmp).await;
            return false;
        }
        self.account_and_prune(body.len() as u64).await;
        true
    }

    pub async fn get_blob(&self, revision: &str) -> Option<Vec<u8>> {
        let path = self.blob_path(revision)?;
        tokio::fs::read(path).await.ok()
    }

    async fn blob_files(&self) -> Vec<(PathBuf, u64, SystemTime)> {
        let mut out = Vec::new();
        let root = self.dir.join("objects");
        let Ok(mut shards) = tokio::fs::read_dir(&root).await else {
            return out;
        };
        while let Ok(Some(shard)) = shards.next_entry().await {
            let Ok(mut files) = tokio::fs::read_dir(shard.path()).await else {
                continue;
            };
            while let Ok(Some(file)) = files.next_entry().await {
                let Ok(meta) = file.metadata().await else {
                    continue;
                };
                if !meta.is_file() {
                    continue;
                }
                let modified = meta.modified().unwrap_or(UNIX_EPOCH);
                out.push((file.path(), meta.len(), modified));
            }
        }
        out
    }

    /// Track the store size without rescanning on every write; rescan once
    /// at first use, then drop the oldest blobs while over the cap.
    async fn account_and_prune(&self, added: u64) {
        let mut total = self.blob_bytes.lock().await;
        let current = match *total {
            Some(bytes) => bytes + added,
            None => self
                .blob_files()
                .await
                .iter()
                .map(|(_, size, _)| size)
                .sum(),
        };
        if current <= self.max_blob_bytes {
            *total = Some(current);
            return;
        }
        let mut files = self.blob_files().await;
        files.sort_by_key(|(_, _, modified)| *modified);
        let mut remaining: u64 = files.iter().map(|(_, size, _)| size).sum();
        let target = self.max_blob_bytes.saturating_mul(9) / 10;
        for (path, size, _) in files {
            if remaining <= target {
                break;
            }
            if tokio::fs::remove_file(&path).await.is_ok() {
                remaining = remaining.saturating_sub(size);
            }
        }
        *total = Some(remaining);
    }
}

pub struct TurnLog {
    store: TurnStore,
    pending: Mutex<HashMap<PendingKey, ReadImage>>,
    locks: Mutex<HashMap<String, Arc<Mutex<()>>>>,
}

impl TurnLog {
    pub fn new(store: TurnStore) -> Self {
        Self {
            store,
            pending: Mutex::new(HashMap::new()),
            locks: Mutex::new(HashMap::new()),
        }
    }

    async fn session_lock(&self, session_id: &str) -> Arc<Mutex<()>> {
        let mut locks = self.locks.lock().await;
        locks
            .entry(session_id.to_string())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    }

    pub async fn load(&self, session_id: &str) -> Result<SessionRecord, Error> {
        self.store.load(session_id).await
    }

    /// A turn with its pre-image bodies read back from the blob store, within
    /// the response budget.
    pub async fn inflate(&self, mut turn: TurnRecord) -> TurnRecord {
        let mut budget = MAX_RESPONSE_BODY_BYTES;
        for file in &mut turn.files {
            let Some(before) = &mut file.before else {
                continue;
            };
            if !before.stored {
                continue;
            }
            let Some(revision) = before.revision.as_deref() else {
                continue;
            };
            match self.store.get_blob(revision).await {
                Some(bytes) if bytes.len() <= budget => match String::from_utf8(bytes) {
                    Ok(text) => {
                        budget -= text.len();
                        before.content = Some(text);
                    }
                    Err(_) => before.binary = true,
                },
                Some(_) => before.truncated = true,
                None => before.stored = false,
            }
        }
        turn
    }

    async fn update<F>(&self, session_id: &str, apply: F) -> Result<(), Error>
    where
        F: FnOnce(&mut SessionRecord),
    {
        let lock = self.session_lock(session_id).await;
        let _guard = lock.lock().await;
        let mut record = self.store.load(session_id).await?;
        apply(&mut record);
        enforce_budgets(&mut record);
        self.store.store(&record).await
    }

    pub async fn on_turn_started(&self, session_id: &str, turn_id: &str) -> Result<(), Error> {
        let at = now_ms();
        self.update(session_id, |record| {
            turn_mut(record, turn_id, at);
        })
        .await
    }

    /// Record a batch of watch-observed changes under a turn.
    pub async fn fold_observed(
        &self,
        session_id: &str,
        turn_id: &str,
        root: &str,
        changes: Vec<(String, &'static str)>,
    ) {
        let at = now_ms();
        let result = self
            .update(session_id, |record| {
                let turn = turn_mut(record, turn_id, at);
                for (path, kind) in changes {
                    record_observed(turn, path, root, kind, at);
                }
            })
            .await;
        if let Err(e) = result {
            tracing::warn!(error = %e, session_id, "turn observe: fold not recorded");
        }
    }

    pub async fn on_turn_completed(&self, session_id: &str, turn_id: &str) -> Result<(), Error> {
        let at = now_ms();
        self.update(session_id, |record| {
            turn_mut(record, turn_id, at).ended_at = Some(at);
        })
        .await
    }

    pub async fn adopt_root(
        &self,
        session_id: &str,
        turn_id: &str,
        root: &str,
    ) -> Result<usize, Error> {
        let root = tokio::fs::canonicalize(root)
            .await
            .map_err(|error| Error::Handler(format!("project root is not accessible: {error}")))?;
        if !root.is_dir() {
            return Err(Error::Handler("project root is not a directory".into()));
        }
        let record = self.load(session_id).await?;
        let started_at = record
            .turns
            .iter()
            .find(|turn| turn.turn_id == turn_id)
            .map(|turn| turn.started_at)
            .ok_or_else(|| Error::Handler("turn is not present in session history".into()))?;
        let scan_root = root.clone();
        let files =
            tokio::task::spawn_blocking(move || scan_adopted_project(&scan_root, started_at))
                .await
                .map_err(|error| Error::Handler(format!("project scan failed: {error}")))?;
        let count = files.len();
        let root = root.to_string_lossy().into_owned();
        let at = now_ms();
        self.update(session_id, |record| {
            let turn = turn_mut(record, turn_id, at);
            for (path, after_revision) in files {
                record_file(
                    turn,
                    FileRecord {
                        path,
                        root: Some(root.clone()),
                        kind: "created".into(),
                        cause: "console::working-directory::propose".into(),
                        first_seen: at,
                        last_seen: at,
                        from: None,
                        before: Some(PreImage {
                            revision: None,
                            content: None,
                            truncated: false,
                            stored: false,
                            missing: true,
                            binary: false,
                        }),
                        after_revision: Some(after_revision),
                    },
                );
            }
        })
        .await?;
        Ok(count)
    }

    pub async fn on_pre_trigger(&self, input: HookInput) {
        let (Some(session_id), Some(turn_id), Some(call)) =
            (input.session_id, input.turn_id, input.call)
        else {
            return;
        };
        let touches = touches(&call);
        if touches.is_empty() {
            return;
        }
        let root = session_root(input.metadata.as_ref());
        let mut pending = self.pending.lock().await;
        for touch in touches {
            let path = absolute_path(&touch.path, root.as_deref());
            let read = read_pre_image(&path).await;
            pending.insert(
                PendingKey {
                    session_id: session_id.clone(),
                    turn_id: turn_id.clone(),
                    path,
                },
                read,
            );
        }
    }

    pub async fn on_post_trigger(&self, input: HookInput) {
        let (Some(session_id), Some(turn_id), Some(call)) =
            (input.session_id, input.turn_id, input.call)
        else {
            return;
        };
        let touches = touches(&call);
        if touches.is_empty() {
            return;
        }
        let failed = input.result.as_ref().is_some_and(|r| r.is_error);
        let root = session_root(input.metadata.as_ref());
        let at = now_ms();
        let mut changes: Vec<FileRecord> = Vec::new();
        for touch in touches {
            let path = absolute_path(&touch.path, root.as_deref());
            let key = PendingKey {
                session_id: session_id.clone(),
                turn_id: turn_id.clone(),
                path: path.clone(),
            };
            let pending = self.pending.lock().await.remove(&key);
            if failed {
                continue;
            }
            let before = match pending {
                Some(ReadImage {
                    mut image,
                    body: Some(body),
                }) => {
                    if let Some(revision) = image.revision.as_deref() {
                        image.stored = self.store.put_blob(revision, &body).await;
                    }
                    Some(image)
                }
                Some(ReadImage { image, body: None }) => Some(image),
                None => None,
            };
            let after_revision = if touch.kind == "deleted" {
                None
            } else {
                read_revision(&path).await
            };
            changes.push(FileRecord {
                path,
                root: root.clone(),
                kind: touch.kind.to_string(),
                cause: call.function_id.clone(),
                first_seen: at,
                last_seen: at,
                from: touch.from.map(|from| absolute_path(&from, root.as_deref())),
                before,
                after_revision,
            });
        }
        if changes.is_empty() {
            return;
        }
        if let Err(e) = self
            .update(&session_id, |record| {
                let turn = turn_mut(record, &turn_id, at);
                for change in changes {
                    record_file(turn, change);
                }
            })
            .await
        {
            tracing::warn!(error = %e, "turn log: recording a change failed");
        }
    }
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListInput {
    pub session_id: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct TurnSummary {
    pub turn_id: String,
    pub started_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<u64>,
    pub file_count: usize,
    /// Paths with their change kind, without bodies.
    pub files: Vec<FileHead>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct FileHead {
    pub path: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub root: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct ListOutput {
    pub session_id: String,
    /// Newest first.
    pub turns: Vec<TurnSummary>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetInput {
    pub session_id: String,
    /// Omit for the newest turn.
    #[serde(default)]
    pub turn_id: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct GetOutput {
    pub session_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub turn: Option<TurnRecord>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AdoptRootInput {
    pub session_id: String,
    pub turn_id: String,
    pub root: String,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct AdoptRootOutput {
    pub recorded: usize,
}

pub fn register(iii: &IIIClient, config: &TurnsConfig) -> Arc<TurnLog> {
    let data_dir = config.resolved_data_dir();
    let log = Arc::new(TurnLog::new(TurnStore::new(
        data_dir.clone(),
        config.max_blob_bytes,
    )));
    let observers = crate::turn_observe::TurnObservers::new(log.clone(), data_dir);

    {
        let log = log.clone();
        let observers = observers.clone();
        iii.register_function(
            PRE_HOOK_FN_ID,
            RegisterFunction::new_async(move |input: HookInput| {
                let log = log.clone();
                let observers = observers.clone();
                async move {
                    if let (Some(session_id), Some(turn_id)) =
                        (input.session_id.as_deref(), input.turn_id.as_deref())
                    {
                        let root = session_root(input.metadata.as_ref());
                        observers.ensure(session_id, turn_id, root.as_deref());
                    }
                    log.on_pre_trigger(input).await;
                    Ok::<HookOutput, Error>(HookOutput::default())
                }
            })
            .description(
                "Internal: keeps the pre-image of a file a shell or coder call is about to \
                 change. Observes only; always continues.",
            )
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let log = log.clone();
        iii.register_function(
            POST_HOOK_FN_ID,
            RegisterFunction::new_async(move |input: HookInput| {
                let log = log.clone();
                async move {
                    log.on_post_trigger(input).await;
                    Ok::<HookOutput, Error>(HookOutput::default())
                }
            })
            .description(
                "Internal: records a file change made through a shell or coder call into \
                 the session's durable change history. Observes only; always continues.",
            )
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let log = log.clone();
        iii.register_function(
            ADOPT_ROOT_FN_ID,
            RegisterFunction::new_async(move |input: AdoptRootInput| {
                let log = log.clone();
                async move {
                    let recorded = log
                        .adopt_root(&input.session_id, &input.turn_id, &input.root)
                        .await?;
                    Ok::<AdoptRootOutput, Error>(AdoptRootOutput { recorded })
                }
            })
            .description(
                "Internal: add a newly created project root to the active Harness turn after a confirmed workspace proposal.",
            )
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
    for (point, function_id) in [
        ("harness::hook::pre-trigger", PRE_HOOK_FN_ID),
        ("harness::hook::post-trigger", POST_HOOK_FN_ID),
    ] {
        if let Err(e) = iii.register_trigger(RegisterTriggerInput::new(
            point.to_string(),
            function_id.to_string(),
            json!({
                "functions": HOOK_FUNCTIONS,
                "timeout_ms": HOOK_TIMEOUT_MS,
                "on_error": "fail_open",
            }),
        )) {
            tracing::warn!(error = %e, point, "turn log: hook binding failed; session history will miss writes");
        }
    }

    {
        let log = log.clone();
        iii.register_function(
            STARTED_FN_ID,
            RegisterFunction::new_async(move |event: TurnEvent| {
                let log = log.clone();
                async move {
                    if let (Some(session_id), Some(turn_id)) = (event.session_id, event.turn_id) {
                        if let Err(e) = log.on_turn_started(&session_id, &turn_id).await {
                            tracing::warn!(error = %e, "turn log: turn start not recorded");
                        }
                    }
                    Ok::<Ack, Error>(Ack { ok: true })
                }
            })
            .description("Internal: opens a turn in the session's change history.")
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
    {
        let log = log.clone();
        let observers = observers.clone();
        iii.register_function(
            COMPLETED_FN_ID,
            RegisterFunction::new_async(move |event: TurnEvent| {
                let log = log.clone();
                let observers = observers.clone();
                async move {
                    if let (Some(session_id), Some(turn_id)) = (event.session_id, event.turn_id) {
                        if let Err(e) = log.on_turn_completed(&session_id, &turn_id).await {
                            tracing::warn!(error = %e, "turn log: turn end not recorded");
                        }
                        observers.complete(&session_id, &turn_id);
                    }
                    Ok::<Ack, Error>(Ack { ok: true })
                }
            })
            .description("Internal: closes a turn in the session's change history.")
            .metadata(json!({ "internal": true, "trace_hidden": true })),
        );
    }
    for (event, function_id) in [
        ("harness::turn-started", STARTED_FN_ID),
        ("harness::turn-completed", COMPLETED_FN_ID),
    ] {
        if let Err(e) = iii.register_trigger(RegisterTriggerInput::new(
            event.to_string(),
            function_id.to_string(),
            json!({}),
        )) {
            tracing::warn!(error = %e, event, "turn log: turn event binding failed");
        }
    }

    {
        let log = log.clone();
        iii.register_function(
            "shell::turns::list",
            RegisterFunction::new_async(move |input: ListInput| {
                let log = log.clone();
                async move {
                    let record = log.load(&input.session_id).await?;
                    let turns = record
                        .turns
                        .iter()
                        .rev()
                        .map(|turn| TurnSummary {
                            turn_id: turn.turn_id.clone(),
                            started_at: turn.started_at,
                            ended_at: turn.ended_at,
                            file_count: turn.files.len(),
                            files: turn
                                .files
                                .iter()
                                .map(|f| FileHead {
                                    path: f.path.clone(),
                                    kind: f.kind.clone(),
                                    root: f.root.clone(),
                                })
                                .collect(),
                        })
                        .collect();
                    Ok::<ListOutput, Error>(ListOutput {
                        session_id: record.session_id,
                        turns,
                    })
                }
            })
            .description(
                "List the turns of a harness session with the files each one changed through \
                 shell or coder functions, newest first. Paths and kinds only; use \
                 shell::turns::get for the pre-images.",
            ),
        );
    }
    {
        let log = log.clone();
        iii.register_function(
            "shell::turns::get",
            RegisterFunction::new_async(move |input: GetInput| {
                let log = log.clone();
                async move {
                    let record = log.load(&input.session_id).await?;
                    let turn = match input.turn_id {
                        Some(turn_id) => record.turns.into_iter().find(|t| t.turn_id == turn_id),
                        None => record.turns.into_iter().last(),
                    };
                    let turn = match turn {
                        Some(turn) => Some(log.inflate(turn).await),
                        None => None,
                    };
                    Ok::<GetOutput, Error>(GetOutput {
                        session_id: input.session_id,
                        turn,
                    })
                }
            })
            .description(
                "Fetch one turn of a session's change history: every file it changed, the \
                 change kind, the function that made it, and the file's pre-image (revision \
                 and body up to 64 KiB each, 1 MiB per response) so a diff can be shown. \
                 Omit turn_id for the newest turn.",
            ),
        );
    }
    tracing::info!("registered shell::turns::list, shell::turns::get, shell::turns::adopt-root and the turn-history hooks");
    log
}

#[cfg(test)]
mod tests {
    use super::*;

    fn log_in(dir: &Path) -> Arc<TurnLog> {
        Arc::new(TurnLog::new(TurnStore::new(dir.join("turns"), 1024 * 1024)))
    }

    fn call(function_id: &str, arguments: Value) -> HookCall {
        HookCall {
            function_id: function_id.to_string(),
            arguments,
        }
    }

    fn hook(session: &str, turn: &str, c: HookCall, root: &str, failed: bool) -> HookInput {
        HookInput {
            metadata: Some(json!({ "fs_scope": { "root": root } })),
            call: Some(c),
            session_id: Some(session.to_string()),
            turn_id: Some(turn.to_string()),
            result: Some(HookResult { is_error: failed }),
        }
    }

    #[test]
    fn observed_never_overwrites_a_hook_record() {
        let mut turn = TurnRecord {
            turn_id: "t1".into(),
            started_at: 1,
            ended_at: None,
            files: vec![FileRecord {
                path: "/w/a.txt".into(),
                root: Some("/w".into()),
                kind: "created".into(),
                cause: "coder::create-file".into(),
                first_seen: 1,
                last_seen: 1,
                from: None,
                before: Some(PreImage {
                    revision: Some("sha256:aa".into()),
                    content: None,
                    truncated: false,
                    missing: false,
                    binary: false,
                    stored: true,
                }),
                after_revision: Some("sha256:bb".into()),
            }],
        };
        record_observed(&mut turn, "/w/a.txt".into(), "/w", "modified", 9);
        let file = &turn.files[0];
        assert_eq!(file.cause, "coder::create-file");
        assert_eq!(file.kind, "created");
        assert_eq!(file.after_revision.as_deref(), Some("sha256:bb"));
        assert!(file.before.is_some());
        assert_eq!(file.last_seen, 9);
    }

    #[test]
    fn observed_records_a_new_path_without_pre_image() {
        let mut turn = TurnRecord {
            turn_id: "t1".into(),
            started_at: 1,
            ended_at: None,
            files: Vec::new(),
        };
        record_observed(&mut turn, "/w/gen.txt".into(), "/w", "created", 5);
        record_observed(&mut turn, "/w/gen.txt".into(), "/w", "deleted", 8);
        let file = &turn.files[0];
        assert_eq!(file.cause, "observed");
        assert_eq!(file.kind, "deleted");
        assert!(file.before.is_none());
        assert_eq!(file.first_seen, 5);
        assert_eq!(file.last_seen, 8);
    }

    #[test]
    fn touches_covers_single_and_batch_verbs() {
        let kinds = |c: HookCall| {
            touches(&c)
                .into_iter()
                .map(|t| (t.path, t.kind, t.from))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            kinds(call("shell::fs::write", json!({ "path": "a.txt" }))),
            vec![("a.txt".into(), "modified", None)]
        );
        assert_eq!(
            kinds(call(
                "coder::create-file",
                json!({ "files": [{ "path": "n.rs" }, { "path": "m.rs" }] })
            )),
            vec![
                ("n.rs".into(), "created", None),
                ("m.rs".into(), "created", None)
            ]
        );
        assert_eq!(
            kinds(call("coder::delete-file", json!({ "paths": ["a", "b"] }))),
            vec![("a".into(), "deleted", None), ("b".into(), "deleted", None)]
        );
        assert_eq!(
            kinds(call(
                "coder::move",
                json!({ "files": [{ "from": "a", "to": "b" }] })
            )),
            vec![("b".into(), "moved", Some("a".into()))]
        );
        assert_eq!(
            kinds(call("shell::fs::mv", json!({ "src": "a", "dst": "b" }))),
            vec![("b".into(), "moved", Some("a".into()))]
        );
        assert_eq!(
            kinds(call(
                "shell::fs::sed",
                json!({ "files": ["a", "b"], "pattern": "x", "replacement": "y" })
            )),
            vec![
                ("a".into(), "modified", None),
                ("b".into(), "modified", None)
            ]
        );
        assert!(touches(&call("shell::fs::read", json!({ "path": "a" }))).is_empty());
        assert!(touches(&call(
            "shell::fs::chmod",
            json!({ "path": "a", "mode": "0644" })
        ))
        .is_empty());
        assert!(touches(&call("shell::exec", json!({ "command": "sed" }))).is_empty());
        assert!(touches(&call("shell::fs::write", json!({}))).is_empty());
    }

    #[test]
    fn absolute_path_joins_relative_to_the_session_root() {
        assert_eq!(absolute_path("/x/y", Some("/r")), "/x/y");
        assert_eq!(absolute_path("y", Some("/r")), "/r/y");
        assert_eq!(absolute_path("y", Some(".")), "y");
        assert_eq!(absolute_path("y", None), "y");
    }

    #[test]
    fn session_ids_become_safe_file_stems() {
        assert_eq!(session_file_stem("s_abc-123.x"), "s_abc-123.x");
        assert!(session_file_stem("../etc/passwd").starts_with("h-"));
        assert!(session_file_stem("").starts_with("h-"));
        assert!(session_file_stem(".hidden").starts_with("h-"));
        assert!(session_file_stem("a/b").starts_with("h-"));
    }

    #[test]
    fn record_file_folds_kinds_and_keeps_the_first_pre_image() {
        let mut turn = TurnRecord {
            turn_id: "t".into(),
            started_at: 1,
            ended_at: None,
            files: Vec::new(),
        };
        let before = PreImage {
            revision: Some("sha256:old".into()),
            content: None,
            truncated: false,
            stored: true,
            missing: false,
            binary: false,
        };
        let change = |kind: &str, before: Option<PreImage>, at: u64| FileRecord {
            path: "/r/a".into(),
            root: Some("/r".into()),
            kind: kind.into(),
            cause: "shell::fs::write".into(),
            first_seen: at,
            last_seen: at,
            from: None,
            before,
            after_revision: Some(format!("sha256:{at}")),
        };
        record_file(&mut turn, change("created", None, 1));
        record_file(&mut turn, change("modified", Some(before.clone()), 2));
        assert_eq!(turn.files.len(), 1);
        assert_eq!(turn.files[0].kind, "created");
        assert_eq!(turn.files[0].before, Some(before));
        assert_eq!(turn.files[0].after_revision.as_deref(), Some("sha256:2"));
        record_file(&mut turn, change("deleted", None, 3));
        assert_eq!(turn.files[0].kind, "deleted");
        record_file(&mut turn, change("created", None, 4));
        assert_eq!(turn.files[0].kind, "modified");
    }

    #[test]
    fn budgets_drop_oldest_turns() {
        let mut record = SessionRecord::default();
        for i in 0..(MAX_TURNS_PER_SESSION + 3) {
            record.turns.push(TurnRecord {
                turn_id: format!("t{i}"),
                started_at: i as u64,
                ended_at: None,
                files: Vec::new(),
            });
        }
        enforce_budgets(&mut record);
        assert_eq!(record.turns.len(), MAX_TURNS_PER_SESSION);
        assert_eq!(record.turns[0].turn_id, "t3");
    }

    #[tokio::test]
    async fn pre_then_post_records_the_exact_pre_image_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        std::fs::create_dir_all(&root).unwrap();
        let root_s = root.to_string_lossy().into_owned();
        let file = root.join("note.md");
        std::fs::write(&file, "before\n").unwrap();
        let log = log_in(dir.path());

        log.on_turn_started("s1", "t1").await.unwrap();
        log.on_pre_trigger(hook(
            "s1",
            "t1",
            call("shell::fs::write", json!({ "path": "note.md" })),
            &root_s,
            false,
        ))
        .await;
        std::fs::write(&file, "after\n").unwrap();
        log.on_post_trigger(hook(
            "s1",
            "t1",
            call("shell::fs::write", json!({ "path": "note.md" })),
            &root_s,
            false,
        ))
        .await;
        log.on_turn_completed("s1", "t1").await.unwrap();

        let record = log.load("s1").await.unwrap();
        assert_eq!(record.turns.len(), 1);
        let stored = &record.turns[0];
        assert!(stored.ended_at.is_some());
        assert_eq!(stored.files.len(), 1);
        let f = &stored.files[0];
        assert_eq!(f.path, file.to_string_lossy());
        assert_eq!(f.kind, "modified");
        assert_eq!(f.cause, "shell::fs::write");
        let before = f.before.as_ref().unwrap();
        assert!(before.stored);
        assert!(before.content.is_none(), "bodies never live in the record");
        assert_eq!(
            before.revision.as_deref(),
            Some(content_revision(b"before\n").as_str())
        );
        assert_eq!(
            f.after_revision.as_deref(),
            Some(content_revision(b"after\n").as_str())
        );

        let inflated = log.inflate(stored.clone()).await;
        assert_eq!(
            inflated.files[0]
                .before
                .as_ref()
                .unwrap()
                .content
                .as_deref(),
            Some("before\n")
        );
        assert!(dir.path().join("turns/sessions/s1.json").is_file());
        assert!(dir.path().join("turns/objects").is_dir());
    }

    #[tokio::test]
    async fn failed_calls_and_missing_files_are_handled() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("work");
        std::fs::create_dir_all(&root).unwrap();
        let root_s = root.to_string_lossy().into_owned();
        let log = log_in(dir.path());
        let create = || {
            call(
                "coder::create-file",
                json!({ "files": [{ "path": "new.txt" }] }),
            )
        };
        log.on_pre_trigger(hook("s", "t", create(), &root_s, false))
            .await;
        log.on_post_trigger(hook("s", "t", create(), &root_s, true))
            .await;
        assert!(log.load("s").await.unwrap().turns.is_empty());

        log.on_pre_trigger(hook("s", "t", create(), &root_s, false))
            .await;
        std::fs::write(root.join("new.txt"), "x").unwrap();
        log.on_post_trigger(hook("s", "t", create(), &root_s, false))
            .await;
        let record = log.load("s").await.unwrap();
        let f = &record.turns[0].files[0];
        assert_eq!(f.kind, "created");
        assert!(f.before.as_ref().unwrap().missing);
        assert!(!f.before.as_ref().unwrap().stored);
        assert!(f.after_revision.is_some());
    }

    #[tokio::test]
    async fn blob_store_dedupes_and_prunes_oldest_past_the_cap() {
        let dir = tempfile::tempdir().unwrap();
        let store = TurnStore::new(dir.path().join("turns"), 1000);
        let a = vec![b'a'; 400];
        let b = vec![b'b'; 400];
        let c = vec![b'c'; 400];
        let ra = content_revision(&a);
        let rb = content_revision(&b);
        let rc = content_revision(&c);
        assert!(store.put_blob(&ra, &a).await);
        assert!(store.put_blob(&ra, &a).await);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(store.put_blob(&rb, &b).await);
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        assert!(store.put_blob(&rc, &c).await);
        assert!(store.get_blob(&ra).await.is_none(), "oldest blob pruned");
        assert!(store.get_blob(&rc).await.is_some());
        assert!(store.get_blob("sha256:zz").await.is_none());
        assert!(!store.put_blob("bogus", &a).await);
    }

    #[tokio::test]
    async fn hooks_without_a_turn_are_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let log = log_in(dir.path());
        log.on_post_trigger(HookInput {
            call: Some(call("shell::fs::write", json!({ "path": "/tmp/x" }))),
            ..HookInput::default()
        })
        .await;
        assert!(!dir.path().join("turns").exists());
    }

    #[tokio::test]
    async fn adopted_new_project_files_join_the_existing_turn() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("project");
        let log = log_in(dir.path());
        log.on_turn_started("session", "turn").await.unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("test")).unwrap();
        std::fs::write(root.join("package.json"), "{}\n").unwrap();
        std::fs::write(root.join("src/index.js"), "export const value = 1\n").unwrap();
        std::fs::write(root.join("test/index.test.js"), "// test\n").unwrap();
        std::fs::create_dir_all(root.join("node_modules/pkg")).unwrap();
        std::fs::write(root.join("node_modules/pkg/index.js"), "noise\n").unwrap();

        let recorded = log
            .adopt_root("session", "turn", root.to_str().unwrap())
            .await
            .unwrap();

        assert_eq!(recorded, 3);
        let record = log.load("session").await.unwrap();
        let files = &record.turns[0].files;
        assert_eq!(files.len(), 3);
        assert!(files.iter().all(|file| file.kind == "created"));
        assert!(files
            .iter()
            .all(|file| file.before.as_ref().is_some_and(|before| before.missing)));
        assert!(files
            .iter()
            .all(|file| file.cause == "console::working-directory::propose"));
        assert!(!files.iter().any(|file| file.path.contains("node_modules")));
    }
}
