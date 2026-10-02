//! `shell::changed` — a system-level workspace watch, owned by this worker.
//!
//! A subscriber binds the trigger type with `config: { path }` and the
//! worker puts an OS file watcher (FSEvents on macOS, inotify on Linux)
//! on that directory tree. Every change fans out as one event — whoever
//! made it: a harness agent calling `coder::*`, a `shell::exec` command's
//! side effects, or an editor outside the engine entirely. No harness
//! coupling, no polling.
//!
//! One watcher per root (and `include_ignored`), shared by its bindings and
//! torn down when the last unregisters (the console page binds per open
//! pane; the binding is GC'd with the pane). Raw OS events storm, so each
//! watcher coalesces per path in a short window before emitting. Emission
//! is best-effort: a slow or absent subscriber must never delay anything.
//!
//! The watch is bounded (`DirWatch`): set up off the runtime, one
//! non-recursive OS watch per directory, `.git`, symlinks and — unless the
//! binding sets `include_ignored` — gitignored trees left out. A root with
//! more than `CHANGED_WATCH_BUDGET` such directories is watched
//! breadth-first up to it, never refused.

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
use tokio::sync::mpsc::Sender;

use crate::code::path::PathResolver;
use crate::code::state::ResolverCell;

/// The trigger type a surface binds to watch a directory change.
pub const CHANGED: &str = "shell::changed";

/// How long a watcher batches raw OS events before fanning out — long
/// enough to fold an editor's write-rename dance into one event, short
/// enough to read as live.
const COALESCE_MS: u64 = 200;
/// Directories one `shell::changed` watch may hold, one inotify watch each,
/// out of a per-user budget shared with editors and every other watcher:
/// about 6% of the 524288 most distributions default to. A tree with more
/// is watched breadth-first from the root up to it (VS Code and JetBrains
/// degrade the same way), and the log says so once.
const CHANGED_WATCH_BUDGET: usize = 32_768;

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

/// The bindings one watch serves: binding id → its function and delivery.
type Subscribers = Arc<Mutex<HashMap<String, (String, Delivery)>>>;

/// One watch, shared by every binding on its root and `include_ignored`.
struct WatchEntry {
    subscribers: Subscribers,
    /// `Some(started)` once the setup is over.
    ready: tokio::sync::watch::Receiver<Option<bool>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for WatchEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The `shell::changed` trigger type: the first binding on a root (and
/// `include_ignored`) starts its watch, the last to unregister stops it.
struct ChangedTriggerHandler {
    iii: IIIClient,
    watches: Arc<Mutex<HashMap<(PathBuf, bool), WatchEntry>>>,
    /// The coder surface's path policy — a watch is a read of every
    /// filename under a tree, so it obeys the SAME jail and denylist as
    /// `coder::*`/`shell::fs::*` (hot-reload aware through the cell).
    resolver: ResolverCell,
    /// Directories one watch may hold: `CHANGED_WATCH_BUDGET`.
    budget: usize,
}

/// The map is plain data: a panic elsewhere must not silently drop a
/// registration (an entry's Drop aborts its pump) while we still return Ok.
fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    if in_repo(root) {
        return git_ignored(root, paths).await;
    }
    let Some(matcher) = plain_ignores(root) else {
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

/// A watch root inside a repository (any ancestor owns a `.git`) is git's
/// domain too: `git -C root check-ignore` resolves the containing
/// repository and its parent .gitignore rules from a subdirectory.
fn in_repo(root: &Path) -> bool {
    root.ancestors().any(|dir| dir.join(".git").exists())
}

/// What a root outside any repository ignores: its `.gitignore` plus the
/// built-in engine-owned directories.
fn plain_ignores(root: &Path) -> Option<ignore::gitignore::Gitignore> {
    let mut builder = ignore::gitignore::GitignoreBuilder::new(root);
    for line in ["data/", "config/", ".iii/", "node_modules/", ".git/"] {
        let _ = builder.add_line(None, line);
    }
    let gitignore = root.join(".gitignore");
    if gitignore.is_file() {
        let _ = builder.add(&gitignore);
    }
    builder.build().ok()
}

/// A bounded workspace watch: one non-recursive OS watch per directory
/// kept. notify's recursive inotify watch walks every directory under the
/// root, symlinks followed and gitignored trees included; on a monorepo
/// root that held the worker for 44 s and nearly every inotify watch the
/// user had.
pub(crate) struct DirWatch {
    watcher: notify::RecommendedWatcher,
    root: PathBuf,
    /// What is under watch. A directory deleted or moved away leaves
    /// notify's map at once and this one when the budget is reached, so a
    /// tree replaced or renamed does not count twice.
    pub(crate) watched: HashSet<PathBuf>,
    /// How many directories `watched` may hold.
    budget: usize,
    /// Gitignored trees are watched too (a binding's `include_ignored`).
    all: bool,
    /// The budget ran out once, and the log said so.
    spent: bool,
}

impl DirWatch {
    /// Blocking: watch `root` and the directories the walk keeps under it
    /// (gitignored ones too with `all`), up to `budget` of them. `None` when
    /// the watcher will not start.
    pub(crate) fn open(
        root: &Path,
        tx: Sender<notify::Event>,
        budget: usize,
        all: bool,
    ) -> Option<Self> {
        let watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                // A full channel means the pump already has a backlog to
                // coalesce; dropping here loses nothing distinct.
                let _ = tx.try_send(event);
            }
        });
        let mut watch = match watcher {
            Ok(watcher) => Self {
                watcher,
                root: root.to_path_buf(),
                watched: HashSet::new(),
                budget,
                all,
                spent: false,
            },
            Err(e) => {
                tracing::warn!(error = %e, "watcher start failed");
                return None;
            }
        };
        watch.add_tree(root, false, &mut false);
        Some(watch)
    }

    /// Blocking: watch `dir` and the directories `walk` keeps under it,
    /// breadth-first while the budget lasts, so a tree too big for it keeps
    /// the directories nearest its top (what a file tree shows first). Each
    /// is watched before the walk reads it, so what is made there meanwhile
    /// is walked or reported. With `files`, returns the files met: written
    /// before their directory had a watch, no event names them. `pruned`
    /// says gone directories were already dropped from the count.
    fn add_tree(&mut self, dir: &Path, files: bool, pruned: &mut bool) -> Vec<PathBuf> {
        let (mut level, mut found) = (vec![dir.to_path_buf()], Vec::new());
        while !level.is_empty() {
            let (mut read, mut full) = (Vec::new(), false);
            for path in level {
                if !self.watched.contains(&path) && self.watched.len() >= self.budget {
                    // Directories deleted or moved away still count: drop them.
                    if !*pruned {
                        self.watched.retain(|dir| dir.is_dir());
                        *pruned = true;
                    }
                    if self.watched.len() >= self.budget {
                        if !self.spent {
                            self.spent = true;
                            tracing::warn!(
                                root = %self.root.display(),
                                budget = self.budget,
                                "too many directories to watch; those deepest under the root go unwatched"
                            );
                        }
                        full = true;
                        break;
                    }
                }
                match self.watcher.watch(&path, RecursiveMode::NonRecursive) {
                    Ok(()) => {
                        self.watched.insert(path.clone());
                        read.push(path);
                    }
                    // Out of inotify watches: the rest would fail too.
                    Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                        tracing::warn!(error = %e, root = %self.root.display(), "watch failed");
                        full = true;
                        break;
                    }
                    // Gone since the walk, or unreadable: nothing to watch.
                    Err(_) => {}
                }
            }
            if read.is_empty() {
                break;
            }
            let mut next = Vec::new();
            for entry in walk(&self.root, &read, files, self.all) {
                let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
                let path = entry.into_path();
                if is_dir {
                    next.push(path);
                } else {
                    found.push(path);
                }
            }
            if full {
                break;
            }
            level = next;
        }
        found
    }
}

/// One level of the walk a watch takes: the entries right under `dirs`
/// (not empty), sorted by name. By `root`'s ignore rules unless `all`:
/// gitignored trees (outside a repository, what `ignored_under` ignores)
/// stay out. `.git` and symlinks always do; regular files come along with
/// `files`.
fn walk(
    root: &Path,
    dirs: &[PathBuf],
    files: bool,
    all: bool,
) -> impl Iterator<Item = ignore::DirEntry> {
    let plain = if all || in_repo(root) {
        None
    } else {
        plain_ignores(root)
    };
    let mut walker = ignore::WalkBuilder::new(&dirs[0]);
    for dir in &dirs[1..] {
        walker.add(dir);
    }
    walker
        .max_depth(Some(1))
        .sort_by_file_name(|a, b| a.cmp(b))
        .follow_links(false)
        .standard_filters(!all)
        .hidden(false)
        .ignore(false)
        .filter_entry(move |entry| {
            let is_dir = entry.file_type().is_some_and(|t| t.is_dir());
            (is_dir || (files && entry.file_type().is_some_and(|t| t.is_file())))
                && entry.file_name() != ".git"
                && !plain
                    .as_ref()
                    .is_some_and(|m| m.matched(entry.path(), is_dir).is_ignore())
        });
    walker.build().flatten().filter(|entry| entry.depth() == 1)
}

/// Watch the trees of directories that appeared under a watch — `true`
/// for one made, `false` for one moved in — gitignored ones left out
/// unless the watch takes them, off the runtime: a tree copied in can be
/// big. Returns the files already inside the ones made: a tool writes into
/// a directory it just made before a watch can exist.
pub(crate) async fn watch_new_dirs(
    root: &Path,
    mut watch: DirWatch,
    mut dirs: HashMap<PathBuf, bool>,
) -> Option<(DirWatch, Vec<PathBuf>)> {
    if !watch.all {
        let rels: Vec<String> = dirs.keys().filter_map(|dir| rel_to(root, dir)).collect();
        let ignored = ignored_under(root, rels.iter()).await;
        dirs.retain(|dir, _| rel_to(root, dir).is_some_and(|rel| !ignored.contains(&rel)));
    }
    tokio::task::spawn_blocking(move || {
        let (mut found, mut pruned) = (Vec::new(), false);
        for (dir, made) in dirs {
            found.extend(watch.add_tree(&dir, made, &mut pruned));
        }
        (watch, found)
    })
    .await
    .ok()
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

/// How one binding's events reach its function: the registration's stored
/// `metadata` and `namespace`, echoed on every fire exactly like the engine
/// does for its own trigger types, plus whether ignored paths are delivered.
///
/// The metadata is not optional plumbing. The harness binds every agent wake
/// as `harness::trigger::deliver` with `metadata: { "__binding": <id> }` and
/// DROPS any fire that arrives without it. Delivering the bare payload made
/// every `shell::changed` wake in a live run expire "unfired" while this
/// worker logged 34 000 successful deliveries (MOT-4719).
#[derive(Debug, Clone, Default)]
struct Delivery {
    metadata: Option<Value>,
    namespace: Option<String>,
    /// Deliver paths the root ignores (git-ignored, or the built-in
    /// engine-owned set outside a repository). Off by default: a wake armed on
    /// a project root would otherwise fire on the engine's own
    /// `data/observability` and `data/session-manager` writes — the
    /// subscriber's own transcript. The workspace UI opts in to count them.
    include_ignored: bool,
}

impl Delivery {
    fn from_config(config: &TriggerConfig) -> Self {
        Self {
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
            include_ignored: config
                .config
                .get("include_ignored")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        }
    }

    /// The fire for one event: the bound function, the event, `Void` routing
    /// (fire-and-forget), and the binding's metadata/namespace passed through.
    fn request(
        &self,
        function_id: &str,
        payload: Value,
    ) -> iii_sdk::protocol::TriggerRequestWithMetadata {
        let mut request: iii_sdk::protocol::TriggerRequestWithMetadata = TriggerRequest {
            function_id: function_id.to_string(),
            payload,
            action: Some(TriggerAction::Void),
            timeout_ms: None,
        }
        .into();
        if let Some(metadata) = &self.metadata {
            request = request.metadata(metadata.clone());
        }
        if let Some(namespace) = &self.namespace {
            request = request.namespace(namespace.clone());
        }
        request
    }
}

/// Consume raw watcher events, coalesce per path, fan out to each bound
/// function. Owns the watcher: aborting this task tears the watch down.
async fn pump(
    iii: IIIClient,
    root: PathBuf,
    mut watch: DirWatch,
    mut rx: tokio::sync::mpsc::Receiver<notify::Event>,
    subscribers: Subscribers,
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
        let mut new_dirs: HashMap<PathBuf, bool> = HashMap::new();
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
                    if kind != "deleted" && p.is_dir() && !p.is_symlink() {
                        // Made or moved in under a watched directory: a
                        // recursive watch would cover it, so this one does.
                        *new_dirs.entry(p.clone()).or_default() |= kind == "created";
                    }
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
        if !new_dirs.is_empty() {
            let Some((grown, found)) = watch_new_dirs(&root, watch, new_dirs).await else {
                return;
            };
            watch = grown;
            for rel in found
                .iter()
                .filter(|p| !is_own_temp(p))
                .filter_map(|p| rel_to(&root, p))
            {
                born.insert(rel.clone());
                batch.insert(rel, "created");
            }
        }
        let ignored_paths = ignored_under(&root, batch.keys()).await;
        let bound: Vec<(String, Delivery)> = lock(&subscribers).values().cloned().collect();
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
            for (function_id, delivery) in &bound {
                if ignored && !delivery.include_ignored {
                    tracing::debug!(
                        function_id = %function_id,
                        path = %event.path,
                        "shell::changed skipped an ignored path (bind with include_ignored: true to receive it)"
                    );
                    continue;
                }
                // Deliveries ride their own tasks: awaiting them here would
                // let one hung send stop the pump from draining `rx`, fill
                // the channel, and silently drop distinct events — the exact
                // "slow subscriber never delays anything" violation.
                let iii = iii.clone();
                let function_id = function_id.clone();
                let request = delivery.request(&function_id, payload.clone());
                let (path, kind) = (event.path.clone(), event.kind.clone());
                tokio::spawn(async move {
                    if let Err(e) = iii.trigger(request).await {
                        tracing::warn!(function_id = %function_id, error = %e, "shell::changed fan-out failed");
                    } else {
                        tracing::info!(
                            function_id = %function_id,
                            path = %path,
                            kind = %kind,
                            "shell::changed delivered"
                        );
                    }
                });
            }
        }
    }
}

/// Start the watch a first binding on `root` needs. The walk and the
/// watches go on a blocking thread: the SDK runs every handler on one
/// runtime thread. The caller puts the entry in the map before the walk
/// ends, so an unregister landing meanwhile drops the watch too.
fn start_watch(
    iii: IIIClient,
    root: PathBuf,
    open: impl FnOnce(&Path, Sender<notify::Event>) -> Option<DirWatch> + Send + 'static,
) -> WatchEntry {
    let subscribers = Subscribers::default();
    let (started, ready) = tokio::sync::watch::channel(None);
    let bound = subscribers.clone();
    let task = tokio::spawn(async move {
        let (tx, rx) = tokio::sync::mpsc::channel::<notify::Event>(1024);
        let walk_root = root.clone();
        let Ok(Some(watch)) = tokio::task::spawn_blocking(move || open(&walk_root, tx)).await
        else {
            started.send_replace(Some(false));
            return;
        };
        started.send_replace(Some(true));
        pump(iii, root, watch, rx, bound).await;
    });
    WatchEntry {
        subscribers,
        ready,
        task,
    }
}

#[async_trait]
impl TriggerHandler for ChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let (budget, all) = (self.budget, Delivery::from_config(&config).include_ignored);
        self.subscribe(config, move |root, tx| {
            DirWatch::open(root, tx, budget, all)
        })
        .await
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = CHANGED, id = %config.id, "watch unregistered");
        self.unbind(&config.id);
        Ok(())
    }
}

impl ChangedTriggerHandler {
    /// Take a binding off its watch, and the watch down with its last.
    fn unbind(&self, id: &str) {
        lock(&self.watches).retain(|_, entry| {
            let mut subscribers = lock(&entry.subscribers);
            subscribers.remove(id);
            !subscribers.is_empty()
        });
    }

    /// `register_trigger` with the watch setup handed in.
    async fn subscribe(
        &self,
        config: TriggerConfig,
        open: impl FnOnce(&Path, Sender<notify::Event>) -> Option<DirWatch> + Send + 'static,
    ) -> Result<(), Error> {
        let resolver = self.resolver.read().await.clone();
        let root = watch_root(&config.config, &resolver)?;
        let shown = root.display().to_string();
        let delivery = Delivery::from_config(&config);
        // A binding registered again (a reconnect) may name another root.
        self.unbind(&config.id);
        let mut ready = {
            let mut watches = lock(&self.watches);
            let key = (root.clone(), delivery.include_ignored);
            // A pump that died (a panic) must not swallow every later binding.
            if watches
                .get(&key)
                .is_some_and(|entry| entry.task.is_finished())
            {
                watches.remove(&key);
            }
            let entry = watches
                .entry(key)
                .or_insert_with(|| start_watch(self.iii.clone(), root, open));
            lock(&entry.subscribers)
                .insert(config.id.clone(), (config.function_id.clone(), delivery));
            entry.ready.clone()
        };
        // An unregister landing mid-setup drops the sender: not a failure.
        let failed = ready
            .wait_for(Option::is_some)
            .await
            .is_ok_and(|started| *started == Some(false));
        if failed {
            self.unbind(&config.id);
            return Err(Error::Handler(format!(
                "shell::changed cannot watch {shown}: the watcher did not start"
            )));
        }
        tracing::info!(
            trigger_type = CHANGED,
            id = %config.id,
            function_id = %config.function_id,
            root = %shown,
            "watch registered"
        );
        Ok(())
    }
}

/// Register the trigger type. The SDK queues the registration and cannot
/// report failure; a dead type surfaces as bindings that never fire.
pub fn register_changed_trigger(iii: &IIIClient, resolver: ResolverCell) {
    let _handle = iii.register_trigger_type(RegisterTriggerType::new(
        CHANGED,
        format!(
            "Fires when anything under the watched directory changes, whoever changed \
             it — bind with config: {{ path }} naming the directory (jail-checked like \
             every coder::* path). Ignored paths (git-ignored; outside a repository \
             data/, config/, .iii/, node_modules/) are skipped unless config also sets \
             include_ignored: true, which also watches inside ignored trees. Symlinks \
             are not followed. A tree holding more than {CHANGED_WATCH_BUDGET} \
             directories is watched breadth-first from the top for that many: changes \
             deeper down go unreported."
        ),
        ChangedTriggerHandler {
            iii: iii.clone(),
            watches: Arc::new(Mutex::new(HashMap::new())),
            resolver,
            budget: CHANGED_WATCH_BUDGET,
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

    /// Prevents: the wake that never fires — the harness binds
    /// `harness::trigger::deliver` with `metadata: { "__binding": id }` and
    /// drops a fire without it; the pump used to send the bare payload
    /// (MOT-4719, 34k deliveries logged here, 0 fires counted there).
    #[test]
    fn deliveries_echo_the_binding_metadata_and_namespace() {
        let config = TriggerConfig {
            id: "b1".into(),
            function_id: "harness::trigger::deliver".into(),
            config: json!({ "path": "/tmp/x" }),
            metadata: Some(json!({ "__binding": "sub_abc" })),
            namespace: Some("project".into()),
        };
        let delivery = Delivery::from_config(&config);
        assert!(!delivery.include_ignored, "ignored paths are opt-in");
        let request = delivery.request("harness::trigger::deliver", json!({ "path": "a.rs" }));
        let debug = format!("{request:?}");
        assert!(debug.contains("__binding"), "{debug}");
        assert!(debug.contains("sub_abc"), "{debug}");
        assert!(debug.contains("project"), "{debug}");
        assert!(debug.contains("Void"), "{debug}");

        // A legacy binding without metadata still fires, without inventing any.
        let bare = Delivery::from_config(&TriggerConfig {
            id: "b2".into(),
            function_id: "probe::on_change".into(),
            config: json!({ "path": "/tmp/x", "include_ignored": true }),
            metadata: None,
            namespace: None,
        });
        assert!(bare.include_ignored);
        let debug = format!("{:?}", bare.request("probe::on_change", json!({})));
        assert!(debug.contains("metadata: None"), "{debug}");
        assert!(debug.contains("namespace: None"), "{debug}");
    }

    fn resolver_rooted_at(root: &Path) -> PathResolver {
        let cfg = crate::code::config::CoderConfig {
            base_paths: vec![root.to_path_buf()],
            ..Default::default()
        };
        PathResolver::new(&cfg).unwrap()
    }

    fn open_watch(
        root: &Path,
        budget: usize,
        all: bool,
    ) -> (DirWatch, tokio::sync::mpsc::Receiver<notify::Event>) {
        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        (DirWatch::open(root, tx, budget, all).unwrap(), rx)
    }

    /// The directories a watch holds, relative to `root`, sorted.
    fn watched_dirs(watch: &DirWatch, root: &Path) -> Vec<String> {
        let mut names: Vec<String> = watch
            .watched
            .iter()
            .map(|dir| {
                dir.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    /// Whether `rx` names `path` within half a second.
    async fn saw(rx: &mut tokio::sync::mpsc::Receiver<notify::Event>, path: &Path) -> bool {
        let seen = async {
            while let Some(event) = rx.recv().await {
                if event.paths.iter().any(|p| p == path) {
                    return true;
                }
            }
            false
        };
        tokio::time::timeout(Duration::from_millis(500), seen)
            .await
            .unwrap_or(false)
    }

    #[tokio::test]
    async fn the_watch_walk_leaves_out_ignored_trees_git_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().canonicalize().unwrap().join("repo");
        for sub in [".git/objects", ".github", "src/deep", "target/debug"] {
            std::fs::create_dir_all(repo.join(sub)).unwrap();
        }
        std::fs::write(repo.join(".gitignore"), "target/\n*.log\n").unwrap();
        std::fs::write(repo.join("src/a.rs"), "").unwrap();
        std::fs::write(repo.join("src/b.log"), "").unwrap();
        std::os::unix::fs::symlink(repo.join("src"), repo.join("link")).unwrap();
        let (watch, _rx) = open_watch(&repo, 64, false);
        assert_eq!(
            watched_dirs(&watch, &repo),
            ["", ".github", "src", "src/deep"]
        );
        let made = HashMap::from([(repo.join("src"), true)]);
        let (_, found) = watch_new_dirs(&repo, watch, made).await.unwrap();
        assert_eq!(found, [repo.join("src/a.rs")]);
        // Bound with include_ignored: ignored trees too, never .git or a symlink.
        let (watch, _rx) = open_watch(&repo, 64, true);
        assert_eq!(
            watched_dirs(&watch, &repo),
            ["", ".github", "src", "src/deep", "target", "target/debug"]
        );
        // Outside a repository: what `ignored_under` ignores there.
        let plain = dir.path().canonicalize().unwrap().join("plain");
        for sub in ["node_modules/pkg", ".iii/state", "app"] {
            std::fs::create_dir_all(plain.join(sub)).unwrap();
        }
        let (watch, _rx) = open_watch(&plain, 64, false);
        assert_eq!(watched_dirs(&watch, &plain), ["", "app"]);
        let (watch, _rx) = open_watch(&plain, 64, true);
        assert_eq!(
            watched_dirs(&watch, &plain),
            [
                "",
                ".iii",
                ".iii/state",
                "app",
                "node_modules",
                "node_modules/pkg"
            ]
        );
    }

    /// The IDE page binds with include_ignored to follow build output and
    /// dependency trees: its watch must reach inside them, at bind time and
    /// for directories made there later. A plain binding's must not.
    #[tokio::test]
    async fn only_an_include_ignored_watch_sees_inside_ignored_trees() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["init", "-q"])
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        for all in [false, true] {
            let (watch, mut rx) = open_watch(&root, 64, all);
            let file = root.join(format!("target/{all}.txt"));
            std::fs::write(&file, "x").unwrap();
            assert_eq!(saw(&mut rx, &file).await, all, "include_ignored: {all}");

            let made = root.join(format!("target/made-{all}"));
            std::fs::create_dir(&made).unwrap();
            std::fs::write(made.join("y.txt"), "y").unwrap();
            let new_dirs = HashMap::from([(made.clone(), true)]);
            let (watch, found) = watch_new_dirs(&root, watch, new_dirs).await.unwrap();
            assert_eq!(watch.watched.contains(&made), all, "include_ignored: {all}");
            assert_eq!(found == [made.join("y.txt")], all, "include_ignored: {all}");
        }
    }

    fn handler_at(root: &Path, budget: usize) -> ChangedTriggerHandler {
        ChangedTriggerHandler {
            iii: IIIClient::new("ws://127.0.0.1:1"),
            watches: Default::default(),
            resolver: Arc::new(tokio::sync::RwLock::new(Arc::new(resolver_rooted_at(root)))),
            budget,
        }
    }

    fn binding(id: &str, path: &Path, include_ignored: bool) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: format!("probe::{id}"),
            config: json!({ "path": path, "include_ignored": include_ignored }),
            metadata: None,
            namespace: None,
        }
    }

    /// A tree past the budget is watched from its top down as far as the
    /// budget goes — what a file tree shows first — and its binding is
    /// accepted: refusing it left the page with no live updates at all.
    #[tokio::test]
    async fn a_root_past_the_budget_keeps_its_top_and_is_still_bound() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        for sub in ["a/b/c", "e", "f"] {
            std::fs::create_dir_all(root.join(sub)).unwrap();
        }
        let (watch, _rx) = open_watch(&root, 4, false);
        assert_eq!(watched_dirs(&watch, &root), ["", "a", "e", "f"]);
        // A directory made once the budget is spent stays out.
        std::fs::create_dir(root.join("g")).unwrap();
        let made = HashMap::from([(root.join("g"), true)]);
        let (watch, _) = watch_new_dirs(&root, watch, made).await.unwrap();
        assert!(!watch.watched.contains(&root.join("g")));

        let handler = handler_at(&root, 4);
        handler
            .register_trigger(binding("b1", &root, false))
            .await
            .unwrap();
    }

    /// Every open pane binds its root: one watch per root (and
    /// include_ignored) serves them all, and lives until the last goes.
    #[tokio::test]
    async fn bindings_on_one_root_share_one_watch_until_the_last_goes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        std::fs::create_dir(root.join("src")).unwrap();
        let handler = handler_at(&root, 64);
        for (id, all) in [("b1", false), ("b2", false), ("b3", true)] {
            handler
                .register_trigger(binding(id, &root, all))
                .await
                .unwrap();
        }
        let live = || {
            let watches = handler.watches.lock().unwrap();
            watches.values().filter(|w| !w.task.is_finished()).count()
        };
        assert_eq!(live(), 2, "one watch per root and include_ignored");
        for (id, all, left) in [("b1", false, 2), ("b2", false, 1), ("b3", true, 0)] {
            handler
                .unregister_trigger(binding(id, &root, all))
                .await
                .unwrap();
            assert_eq!(live(), left, "after unregistering {id}");
        }
    }

    /// The SDK runs every handler on one current-thread runtime, as this
    /// test does: the walk must not hold it, and an unregister landing
    /// mid-walk drops the watch instead of leaking it.
    #[tokio::test]
    async fn a_binding_sets_its_watch_up_off_the_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let handler = handler_at(&root, 64);
        let (started, walking) = tokio::sync::oneshot::channel::<()>();
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let open = move |root: &Path, tx| {
            let _ = started.send(());
            let _ = gate.recv_timeout(Duration::from_secs(10));
            DirWatch::open(root, tx, 64, false)
        };
        let begun = std::time::Instant::now();
        let (registered, ()) = tokio::join!(
            handler.subscribe(binding("b1", &root, false), open),
            async {
                walking.await.unwrap();
                handler
                    .unregister_trigger(binding("b1", &root, false))
                    .await
                    .unwrap();
                release.send(()).unwrap();
            }
        );
        registered.unwrap();
        assert!(
            begun.elapsed() < Duration::from_secs(5),
            "the walk held the runtime"
        );
        assert!(handler.watches.lock().unwrap().is_empty());
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
