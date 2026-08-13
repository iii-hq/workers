//! `shell::changed` — a system-level workspace watch, owned by this worker.
//!
//! A subscriber binds the trigger type with `config: { path }` and the
//! worker puts an OS file watcher (FSEvents on macOS, inotify on Linux)
//! on that directory tree. Every change fans out as one event — whoever
//! made it: a harness agent calling `coder::*`, a `shell::exec` command's
//! side effects, or an editor outside the engine entirely. No harness
//! coupling, no polling.
//!
//! One watcher per subscription, torn down when the binding unregisters
//! (the console page binds per open tab; the binding is GC'd with the
//! tab). Raw OS events storm, so each watcher coalesces per path in a
//! short window before emitting. Emission is best-effort: a slow or
//! absent subscriber must never delay anything.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use notify::{RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::Value;

/// The trigger type a surface binds to watch a directory change.
pub const CHANGED: &str = "shell::changed";

/// How long a watcher batches raw OS events before fanning out — long
/// enough to fold an editor's write-rename dance into one event, short
/// enough to read as live.
const COALESCE_MS: u64 = 200;

/// What changed. Lean by design: a subscriber that wants content asks
/// `coder::read-file`; one that wants the diff asks git.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ChangedEvent {
    /// Path relative to `root`.
    pub path: String,
    /// `created`, `modified`, or `deleted`.
    pub kind: String,
    /// The watched directory this event is relative to.
    pub root: String,
    /// True when the path is a directory — a subscriber that opens
    /// files must skip these. Deleted paths can't be probed and report
    /// false.
    pub dir: bool,
}

struct WatchEntry {
    task: tokio::task::JoinHandle<()>,
}

impl Drop for WatchEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The `shell::changed` trigger type: each registration starts a watcher,
/// each unregistration stops one.
struct ChangedTriggerHandler {
    iii: IIIClient,
    watches: Arc<Mutex<HashMap<String, WatchEntry>>>,
}

/// Resolve and validate the watched directory out of a binding's config.
fn watch_root(config: &Value) -> Result<PathBuf, Error> {
    let Some(path) = config.get("path").and_then(Value::as_str) else {
        return Err(Error::Handler(
            "shell::changed needs config.path — the directory to watch".into(),
        ));
    };
    match std::fs::canonicalize(path) {
        Ok(p) if p.is_dir() => Ok(p),
        Ok(p) => Err(Error::Handler(format!(
            "shell::changed config.path is not a directory: {}",
            p.display()
        ))),
        Err(e) => Err(Error::Handler(format!(
            "shell::changed cannot watch {path}: {e}"
        ))),
    }
}

/// Map a notify event kind onto the wire vocabulary.
fn kind_of(kind: &notify::EventKind) -> &'static str {
    use notify::EventKind;
    match kind {
        EventKind::Create(_) => "created",
        EventKind::Remove(_) => "deleted",
        _ => "modified",
    }
}

/// Reads and bare metadata touches are not workspace changes — reporting
/// them would turn every `cat` and `chmod` into a phantom modification.
/// Content writes carry their own Data events regardless.
fn is_noise_kind(kind: &notify::EventKind) -> bool {
    use notify::event::ModifyKind;
    use notify::EventKind;
    matches!(
        kind,
        EventKind::Access(_) | EventKind::Modify(ModifyKind::Metadata(_))
    )
}

/// Git internals churn constantly during any git operation and mean
/// nothing to a workspace surface — the visible outcome arrives as
/// worktree events of its own.
fn is_git_internal(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_str() == Some(".git"))
}

/// Fold two kinds seen for one path in one window. A create followed by
/// the write that fills the file is still a creation; a deletion
/// supersedes what came before it; a creation after a deletion is a
/// creation again.
fn merge_kinds(prev: &'static str, next: &'static str) -> &'static str {
    match (prev, next) {
        ("created", "modified") => "created",
        _ => next,
    }
}

/// Relative to the watched root; `None` for the root itself or paths
/// outside it.
fn rel_to(root: &Path, path: &Path) -> Option<String> {
    let rel = path.strip_prefix(root).ok()?;
    let s = rel.to_string_lossy();
    if s.is_empty() {
        return None;
    }
    Some(s.into_owned())
}

/// Consume raw watcher events, coalesce per path, fan out to the bound
/// function. Owns the watcher: aborting this task tears the watch down.
async fn pump(
    iii: IIIClient,
    function_id: String,
    root: PathBuf,
    _watcher: notify::RecommendedWatcher,
    mut rx: tokio::sync::mpsc::Receiver<notify::Event>,
) {
    let root_str = root.to_string_lossy().into_owned();
    loop {
        let Some(first) = rx.recv().await else {
            return; // channel closed — the watch is gone
        };
        // Coalesce the storm: kinds for one path merge across the window
        // (macOS reports a create and the write that fills it separately).
        let mut batch: HashMap<String, &'static str> = HashMap::new();
        let mut fold = |event: notify::Event| {
            if is_noise_kind(&event.kind) {
                return;
            }
            let kind = kind_of(&event.kind);
            for p in &event.paths {
                if is_git_internal(p) {
                    continue;
                }
                if let Some(rel) = rel_to(&root, p) {
                    batch
                        .entry(rel)
                        .and_modify(|prev| *prev = merge_kinds(prev, kind))
                        .or_insert(kind);
                }
            }
        };
        fold(first);
        let window = tokio::time::sleep(Duration::from_millis(COALESCE_MS));
        tokio::pin!(window);
        loop {
            tokio::select! {
                more = rx.recv() => match more {
                    Some(event) => fold(event),
                    None => break,
                },
                () = &mut window => break,
            }
        }
        for (path, kind) in batch.drain() {
            let dir = kind != "deleted" && root.join(&path).is_dir();
            let event = ChangedEvent {
                path,
                kind: kind.to_string(),
                root: root_str.clone(),
                dir,
            };
            let payload = match serde_json::to_value(&event) {
                Ok(v) => v,
                Err(_) => continue,
            };
            if let Err(e) = iii
                .trigger(TriggerRequest {
                    function_id: function_id.clone(),
                    payload,
                    action: Some(TriggerAction::Void),
                    timeout_ms: None,
                })
                .await
            {
                tracing::warn!(function_id = %function_id, error = %e, "shell::changed fan-out failed");
            } else {
                tracing::info!(
                    function_id = %function_id,
                    path = %event.path,
                    kind = %event.kind,
                    "shell::changed delivered"
                );
            }
        }
    }
}

#[async_trait]
impl TriggerHandler for ChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let root = watch_root(&config.config)?;

        let (tx, rx) = tokio::sync::mpsc::channel::<notify::Event>(1024);
        let mut watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                // A full channel means the pump already has a backlog to
                // coalesce; dropping here loses nothing distinct.
                let _ = tx.try_send(event);
            }
        })
        .map_err(|e| Error::Handler(format!("watcher start failed: {e}")))?;
        watcher
            .watch(&root, RecursiveMode::Recursive)
            .map_err(|e| Error::Handler(format!("watch {} failed: {e}", root.display())))?;

        tracing::info!(
            trigger_type = CHANGED,
            id = %config.id,
            function_id = %config.function_id,
            root = %root.display(),
            "watch registered"
        );
        let task = tokio::spawn(pump(
            self.iii.clone(),
            config.function_id,
            root,
            watcher,
            rx,
        ));
        if let Ok(mut watches) = self.watches.lock() {
            watches.insert(config.id, WatchEntry { task });
        }
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = CHANGED, id = %config.id, "watch unregistered");
        if let Ok(mut watches) = self.watches.lock() {
            watches.remove(&config.id);
        }
        Ok(())
    }
}

/// Register the trigger type. The SDK queues the registration and cannot
/// report failure; a dead type surfaces as bindings that never fire.
pub fn register_changed_trigger(iii: &IIIClient) {
    let _handle = iii.register_trigger_type(RegisterTriggerType::new(
        CHANGED,
        "Fires when anything under the watched directory changes, whoever changed \
         it — bind with config: { path } naming the directory.",
        ChangedTriggerHandler {
            iii: iii.clone(),
            watches: Arc::new(Mutex::new(HashMap::new())),
        },
    ));
    tracing::info!(
        trigger_type = CHANGED,
        "sent the trigger type registration; delivery is confirmed by the first subscription"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn notify_kinds_map_onto_the_wire_vocabulary() {
        use notify::event::{CreateKind, EventKind, ModifyKind, RemoveKind};
        assert_eq!(kind_of(&EventKind::Create(CreateKind::File)), "created");
        assert_eq!(kind_of(&EventKind::Remove(RemoveKind::File)), "deleted");
        assert_eq!(kind_of(&EventKind::Modify(ModifyKind::Any)), "modified");
    }

    #[test]
    fn reads_and_metadata_touches_are_noise() {
        use notify::event::{
            AccessKind, CreateKind, DataChange, EventKind, MetadataKind, ModifyKind,
        };
        assert!(is_noise_kind(&EventKind::Access(AccessKind::Any)));
        assert!(is_noise_kind(&EventKind::Modify(ModifyKind::Metadata(
            MetadataKind::Any
        ))));
        assert!(!is_noise_kind(&EventKind::Modify(ModifyKind::Data(
            DataChange::Any
        ))));
        assert!(!is_noise_kind(&EventKind::Create(CreateKind::File)));
    }

    #[test]
    fn kinds_merge_toward_the_visible_outcome() {
        assert_eq!(merge_kinds("created", "modified"), "created");
        assert_eq!(merge_kinds("created", "deleted"), "deleted");
        assert_eq!(merge_kinds("deleted", "created"), "created");
        assert_eq!(merge_kinds("modified", "modified"), "modified");
    }

    #[test]
    fn git_internals_are_filtered() {
        assert!(is_git_internal(Path::new("/repo/.git/objects/ab")));
        assert!(!is_git_internal(Path::new("/repo/src/.gitignore")));
        assert!(!is_git_internal(Path::new("/repo/src/main.rs")));
    }

    #[test]
    fn rel_to_strips_the_root_and_skips_the_root_itself() {
        let root = Path::new("/srv/app");
        assert_eq!(
            rel_to(root, Path::new("/srv/app/a.rs")).as_deref(),
            Some("a.rs")
        );
        assert_eq!(
            rel_to(root, Path::new("/srv/app/x/y.rs")).as_deref(),
            Some("x/y.rs")
        );
        assert!(rel_to(root, Path::new("/srv/app")).is_none());
        assert!(rel_to(root, Path::new("/elsewhere/b.rs")).is_none());
    }

    #[test]
    fn watch_root_validates_shape_and_existence() {
        let err = watch_root(&json!({})).unwrap_err();
        assert!(err.to_string().contains("config.path"), "{err}");
        let err = watch_root(&json!({ "path": "/definitely/not/here-xyz" })).unwrap_err();
        assert!(err.to_string().contains("cannot watch"), "{err}");
        let tmp = tempfile::tempdir().unwrap();
        let ok = watch_root(&json!({ "path": tmp.path().to_string_lossy() })).unwrap();
        assert!(ok.is_dir());
        let file = tmp.path().join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let err = watch_root(&json!({ "path": file.to_string_lossy() })).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
    }
}
