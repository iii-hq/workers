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

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Stdio;
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

use crate::code::path::PathResolver;
use crate::code::state::ResolverCell;

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
    /// True when git ignores the path under `root` — build output, a
    /// dependency tree, a worker's own store. A root that is not a
    /// repository reports false for everything.
    pub ignored: bool,
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
    /// The coder surface's path policy — a watch is a read of every
    /// filename under a tree, so it obeys the SAME jail and denylist as
    /// `coder::*`/`shell::fs::*` (hot-reload aware through the cell).
    resolver: ResolverCell,
}

/// Resolve and validate the watched directory out of a binding's config —
/// through the shared path policy: jail containment, operator denylist,
/// canonicalization. Watching `/` or a denied tree fails exactly like
/// reading it would.
fn watch_root(config: &Value, resolver: &PathResolver) -> Result<PathBuf, Error> {
    let Some(path) = config.get("path").and_then(Value::as_str) else {
        return Err(Error::Handler(
            "shell::changed needs config.path — the directory to watch".into(),
        ));
    };
    let canon = resolver
        .resolve(path)
        .map_err(|e| Error::Handler(format!("shell::changed cannot watch {path}: {e}")))?;
    if !canon.is_dir() {
        return Err(Error::Handler(format!(
            "shell::changed config.path is not a directory: {}",
            canon.display()
        )));
    }
    Ok(canon)
}

/// Map a notify event kind onto the wire vocabulary.
pub(crate) fn kind_of(kind: &notify::EventKind) -> &'static str {
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
pub(crate) fn is_noise_kind(kind: &notify::EventKind) -> bool {
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
pub(crate) fn is_git_internal(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str().to_str() == Some(".git"))
}

/// This worker's own atomic-write machinery: sibling temp files that
/// exist for a moment between write and rename (`coder::update-file`'s
/// `.coder-tmp-`, the fs backend's `.iii-tmp-` and `.tmp.<uuid>`).
/// Reporting them would hand every write a phantom neighbor — and the
/// rename lands as an event on the REAL path regardless.
pub(crate) fn is_own_temp(path: &Path) -> bool {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.contains(".coder-tmp-")
        || name.contains(".iii-tmp-")
        || name
            .rsplit_once(".tmp.")
            .is_some_and(|(_, id)| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
}

/// Fold two kinds seen for one path in one window. A create followed by
/// the write that fills the file is still a creation; a deletion
/// supersedes what came before it; a creation after a deletion is a
/// creation again.
pub(crate) fn merge_kinds(prev: &'static str, next: &'static str) -> &'static str {
    match (prev, next) {
        ("created", "modified") => "created",
        _ => next,
    }
}

/// What a coalesced change amounts to once the window closes. `born` says the
/// path was created inside this window; a born path that is gone again never
/// existed for an observer (atomic-write temps arrive as create+rename or
/// create+delete), so it emits nothing regardless of the merged kind. A
/// pre-existing path that vanished mid-window is a real deletion.
pub(crate) fn resolve_kind(kind: &'static str, on_disk: bool, born: bool) -> Option<&'static str> {
    match (kind, on_disk) {
        (_, false) if born => None,
        ("created", false) => None,
        ("deleted", _) => Some("deleted"),
        (_, false) => Some("deleted"),
        (kind, true) => Some(kind),
    }
}

/// The subset of root-relative `paths` the watch treats as ignored. Inside a
/// git repository git itself decides. Outside one (compose template projects
/// are plain directories), fall back to the root `.gitignore` plus a small
/// built-in set for engine-owned directories — otherwise every internal
/// state/queue write floods the change feed as reviewable work.
pub(crate) async fn ignored_under<'a>(
    root: &Path,
    paths: impl Iterator<Item = &'a String> + Clone,
) -> HashSet<String> {
    // A watch root inside a repository (any ancestor owns a `.git`) is git's
    // domain too: `git -C root check-ignore` resolves the containing
    // repository and its parent .gitignore rules from a subdirectory.
    if root.ancestors().any(|dir| dir.join(".git").exists()) {
        return git_ignored(root, paths).await;
    }
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for line in ["data/", "config/", ".iii/", "node_modules/", ".git/"] {
        let _ = builder.add_line(None, line);
    }
    let gitignore = root.join(".gitignore");
    if gitignore.is_file() {
        let _ = builder.add(&gitignore);
    }
    let Ok(matcher) = builder.build() else {
        return HashSet::new();
    };
    paths
        .filter(|rel| {
            let is_dir = root.join(rel.as_str()).is_dir();
            matcher
                .matched_path_or_any_parents(rel.as_str(), is_dir)
                .is_ignore()
        })
        .cloned()
        .collect()
}

/// The subset of root-relative `paths` git ignores under `root`. A root
/// that is not a repository, or a host without git, ignores nothing.
pub(crate) async fn git_ignored<'a>(
    root: &Path,
    paths: impl Iterator<Item = &'a String>,
) -> HashSet<String> {
    let mut input = Vec::new();
    for rel in paths {
        input.extend_from_slice(rel.as_bytes());
        input.push(0);
    }
    if input.is_empty() {
        return HashSet::new();
    }
    let Ok(mut child) = tokio::process::Command::new("git")
        .arg("-C")
        .arg(root)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return HashSet::new();
    };
    let stdin = child.stdin.take();
    let writer = tokio::spawn(async move {
        if let Some(mut stdin) = stdin {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(&input).await;
            let _ = stdin.shutdown().await;
        }
    });
    let output = child.wait_with_output().await;
    let _ = writer.await;
    let Ok(out) = output else {
        return HashSet::new();
    };
    out.stdout
        .split(|b| *b == 0)
        .filter_map(|chunk| std::str::from_utf8(chunk).ok())
        .filter(|rel| !rel.is_empty())
        .map(str::to_owned)
        .collect()
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
        let mut born: HashSet<String> = HashSet::new();
        let mut fold = |event: notify::Event| {
            if is_noise_kind(&event.kind) {
                return;
            }
            let kind = kind_of(&event.kind);
            for p in &event.paths {
                if is_git_internal(p) || is_own_temp(p) {
                    continue;
                }
                if let Some(rel) = rel_to(&root, p) {
                    if kind == "created" {
                        born.insert(rel.clone());
                    }
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
        let ignored_paths = ignored_under(&root, batch.keys()).await;
        for (path, kind) in batch.drain() {
            let on_disk = root.join(&path).exists();
            let Some(kind) = resolve_kind(kind, on_disk, born.contains(&path)) else {
                continue;
            };
            let dir = kind != "deleted" && root.join(&path).is_dir();
            let ignored = ignored_paths.contains(&path);
            let event = ChangedEvent {
                path,
                kind: kind.to_string(),
                root: root_str.clone(),
                dir,
                ignored,
            };
            let payload = match serde_json::to_value(&event) {
                Ok(v) => v,
                Err(_) => continue,
            };
            // Deliveries ride their own tasks: awaiting them here would
            // let one hung send stop the pump from draining `rx`, fill
            // the channel, and silently drop distinct events — the exact
            // "slow subscriber never delays anything" violation.
            let iii = iii.clone();
            let function_id = function_id.clone();
            tokio::spawn(async move {
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
            });
        }
    }
}

#[async_trait]
impl TriggerHandler for ChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let resolver = self.resolver.read().await.clone();
        let root = watch_root(&config.config, &resolver)?;

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
        // Poison recovery: the map is plain data — a panic elsewhere must
        // not silently drop this registration (the entry's Drop aborts
        // the pump) while we still return Ok.
        self.watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(config.id, WatchEntry { task });
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = CHANGED, id = %config.id, "watch unregistered");
        self.watches
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(&config.id);
        Ok(())
    }
}

/// Register the trigger type. The SDK queues the registration and cannot
/// report failure; a dead type surfaces as bindings that never fire.
pub fn register_changed_trigger(iii: &IIIClient, resolver: ResolverCell) {
    let _handle = iii.register_trigger_type(RegisterTriggerType::new(
        CHANGED,
        "Fires when anything under the watched directory changes, whoever changed \
         it — bind with config: { path } naming the directory (jail-checked like \
         every coder::* path).",
        ChangedTriggerHandler {
            iii: iii.clone(),
            watches: Arc::new(Mutex::new(HashMap::new())),
            resolver,
        },
    ));
    tracing::info!(
        trigger_type = CHANGED,
        "sent the trigger type registration; delivery is confirmed by the first subscription"
    );
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn fallback_defers_to_git_from_a_subdirectory_watch() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        std::fs::create_dir_all(root.join("sub")).unwrap();
        let paths = [
            "build.log".to_string(),
            "data/keep.txt".to_string(),
            "src/main.rs".to_string(),
        ];
        let ignored = super::ignored_under(&root.join("sub"), paths.iter()).await;
        assert!(ignored.contains("build.log"));
        assert!(!ignored.contains("data/keep.txt"));
        assert!(!ignored.contains("src/main.rs"));
    }

    #[test]
    fn resolve_kind_folds_transient_writes_and_real_deletions() {
        assert_eq!(super::resolve_kind("created", false, true), None);
        assert_eq!(super::resolve_kind("deleted", false, true), None);
        assert_eq!(super::resolve_kind("created", false, false), None);
        assert_eq!(
            super::resolve_kind("deleted", false, false),
            Some("deleted")
        );
        assert_eq!(
            super::resolve_kind("modified", false, false),
            Some("deleted")
        );
        assert_eq!(
            super::resolve_kind("modified", true, false),
            Some("modified")
        );
        assert_eq!(super::resolve_kind("created", true, true), Some("created"));
    }

    #[tokio::test]
    async fn fallback_ignores_engine_dirs_and_root_gitignore_outside_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::write(root.join(".gitignore"), "*.log\n").unwrap();
        let paths = [
            "data/state/a.bin.tmp".to_string(),
            "config/shell-ui.yaml.tmp".to_string(),
            "todo/node_modules/x/index.js".to_string(),
            "build.log".to_string(),
            "src/main.rs".to_string(),
        ];
        let ignored = super::ignored_under(root, paths.iter()).await;
        assert!(ignored.contains("data/state/a.bin.tmp"));
        assert!(ignored.contains("config/shell-ui.yaml.tmp"));
        assert!(ignored.contains("todo/node_modules/x/index.js"));
        assert!(ignored.contains("build.log"));
        assert!(!ignored.contains("src/main.rs"));
    }

    #[tokio::test]
    async fn fallback_defers_to_git_inside_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        let paths = ["data/a.jsonl".to_string()];
        let ignored = super::ignored_under(root, paths.iter()).await;
        assert!(ignored.is_empty());
    }

    #[tokio::test]
    async fn git_ignored_reports_only_the_ignored_paths() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let git = |args: &[&str]| {
            let status = std::process::Command::new("git")
                .arg("-C")
                .arg(root)
                .args(args)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .unwrap();
            assert!(status.success(), "git {args:?}");
        };
        git(&["init", "-q"]);
        std::fs::write(
            root.join(".gitignore"),
            "data/
*.log
",
        )
        .unwrap();
        let paths = [
            "data/session-manager/a.jsonl".to_string(),
            "build.log".to_string(),
            "src/main.rs".to_string(),
        ];
        let ignored = super::git_ignored(root, paths.iter()).await;
        assert!(ignored.contains("data/session-manager/a.jsonl"));
        assert!(ignored.contains("build.log"));
        assert!(!ignored.contains("src/main.rs"));
    }

    #[tokio::test]
    async fn git_ignored_is_empty_outside_a_repository() {
        let dir = tempfile::tempdir().unwrap();
        let paths = ["data/a.jsonl".to_string()];
        let ignored = super::git_ignored(dir.path(), paths.iter()).await;
        assert!(ignored.is_empty());
    }

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
    fn own_atomic_write_temps_are_filtered() {
        assert!(is_own_temp(Path::new(
            "/w/index.html.coder-tmp-58416-681434000"
        )));
        assert!(is_own_temp(Path::new(
            "/w/a.txt.iii-tmp-1f2e3d4c-0000-0000-0000-000000000000"
        )));
        assert!(is_own_temp(Path::new(
            "/w/a.txt.tmp.0123456789abcdef0123456789abcdef"
        )));
        assert!(!is_own_temp(Path::new("/w/index.html")));
        assert!(!is_own_temp(Path::new("/w/notes.tmp.md")));
        assert!(!is_own_temp(Path::new("/w/archive.tmp.backup")));
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

    fn resolver_rooted_at(root: &Path) -> PathResolver {
        let cfg = crate::code::config::CoderConfig {
            base_paths: vec![root.to_path_buf()],
            ..Default::default()
        };
        PathResolver::new(&cfg).unwrap()
    }

    #[test]
    fn watch_root_validates_shape_existence_and_jail() {
        let tmp = tempfile::tempdir().unwrap();
        let jail = tmp.path().canonicalize().unwrap();
        let resolver = resolver_rooted_at(&jail);

        let err = watch_root(&json!({}), &resolver).unwrap_err();
        assert!(err.to_string().contains("config.path"), "{err}");
        let err =
            watch_root(&json!({ "path": "/definitely/not/here-xyz" }), &resolver).unwrap_err();
        assert!(err.to_string().contains("cannot watch"), "{err}");

        let sub = jail.join("watched");
        std::fs::create_dir(&sub).unwrap();
        let ok = watch_root(&json!({ "path": sub.to_string_lossy() }), &resolver).unwrap();
        assert!(ok.is_dir());

        let file = jail.join("f.txt");
        std::fs::write(&file, "x").unwrap();
        let err = watch_root(&json!({ "path": file.to_string_lossy() }), &resolver).unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");

        // The jail holds: an existing directory OUTSIDE the allowed root
        // is refused exactly like reading it would be.
        let outside = tempfile::tempdir().unwrap();
        let err = watch_root(
            &json!({ "path": outside.path().to_string_lossy() }),
            &resolver,
        )
        .unwrap_err();
        assert!(err.to_string().contains("cannot watch"), "{err}");
    }
}
