//! `shell::git-changed` — a repository's own state, watched.
//!
//! `shell::changed` leaves `.git` out on purpose: git internals churn on
//! every operation and mean nothing to a file tree. A surface that shows the
//! repository itself (the branch chip, the Git window's log) still has to
//! learn that HEAD moved, the index changed or a branch appeared — a
//! `git switch` in a terminal fires nothing else. Without this type the only
//! way to learn it was to ask git again on every focus change.
//!
//! A subscriber binds with `config: { path }` naming any directory inside a
//! worktree (jail-checked like every `coder::*` path). The worker finds the
//! repository the way git does (the nearest `.git`, a directory or a
//! `gitdir:` file, and its `commondir`) and watches only what git state lives
//! in: the worktree's git dir (`HEAD`, `index`), the common dir
//! (`packed-refs`), `refs/**` and the other worktrees' `HEAD`s, plus the
//! worktree top for `.git` itself coming or going. Raw events are batched
//! for `COALESCE_MS` and fan out as ONE event per batch naming what moved.
//!
//! A `path` in no repository is watched for a `.git` to appear (`git init`,
//! a clone into it): the subscriber hears `repository` once, and from then
//! on the repository's own changes. Removing the repository says
//! `repository` again and goes back to waiting. No polling anywhere.
//!
//! One watch per path, shared by its bindings and torn down with the last.
//! Delivery mirrors [`crate::events`]: `Void` routing, each fire on its own
//! task, the binding's `metadata` and `namespace` passed through.

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, RegisterTriggerType, TriggerAction};
use notify::{RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;

use crate::code::path::PathResolver;
use crate::code::state::ResolverCell;
use crate::events::is_noise_kind;

/// The trigger type a surface binds to follow a repository's state.
pub const GIT_CHANGED: &str = "shell::git-changed";

/// Batching window, as `shell::changed`'s: a `git commit` writes the index,
/// a ref, a reflog and HEAD's log in a few milliseconds — one event.
const COALESCE_MS: u64 = 200;

/// The current worktree's HEAD moved: a switch, a checkout, a detach.
pub const HEAD: &str = "head";
/// The worktree's index was rewritten: a stage, a commit, a reset.
pub const INDEX: &str = "index";
/// A ref changed: a commit on a branch, a branch made or deleted, a fetch,
/// a tag, a stash, `packed-refs` rewritten.
pub const REFS: &str = "refs";
/// Another worktree's HEAD moved, or a worktree was added or removed.
pub const WORKTREES: &str = "worktrees";
/// The repository appeared, disappeared or moved: re-read everything.
pub const REPOSITORY: &str = "repository";

/// Bind with `{ path }`: any directory inside a worktree, or one that may
/// become a repository.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitChangedConfig {
    /// The directory whose repository to follow (jail-checked).
    pub path: String,
}

/// One batch of repository changes. Lean like [`crate::events::ChangedEvent`]:
/// a subscriber that wants the new state asks git.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq)]
pub struct GitChangedEvent {
    /// The watched directory: the binding's `path`, canonical.
    pub path: String,
    /// The worktree's top level; absent while `path` is in no repository.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub worktree: Option<String>,
    /// The worktree's own git dir (`.git`, or `<common>/worktrees/<name>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub git_dir: Option<String>,
    /// The repository's common dir (refs, packed-refs, worktrees).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub common_dir: Option<String>,
    /// What moved in this batch, sorted: `head`, `index`, `refs`,
    /// `worktrees`, `repository`.
    pub changes: Vec<String>,
}

/// Where a worktree's git state lives.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Layout {
    /// The worktree's top: the directory holding `.git`.
    pub top: PathBuf,
    /// `HEAD` and `index` live here.
    pub git_dir: PathBuf,
    /// `refs/`, `packed-refs` and `worktrees/` live here; the git dir itself
    /// in a main checkout.
    pub common_dir: PathBuf,
}

/// A path relative to `base`, absolute paths left as they are.
fn joined(base: &Path, rel: &str) -> PathBuf {
    let rel = Path::new(rel);
    let path = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        base.join(rel)
    };
    path.canonicalize().unwrap_or(path)
}

/// The repository `dir` sits in, found the way git finds it without
/// environment overrides: the nearest ancestor holding a `.git` directory,
/// or a `.git` file naming one (`gitdir: <path>`, a linked worktree or a
/// submodule). A `.git` without a HEAD is not a repository yet (a `git init`
/// mid-write) and the search goes on above it, as git's does.
pub(crate) fn discover(dir: &Path) -> Option<Layout> {
    for top in dir.ancestors() {
        let dot = top.join(".git");
        let Ok(meta) = std::fs::metadata(&dot) else {
            continue;
        };
        let git_dir = if meta.is_dir() {
            dot
        } else if meta.is_file() {
            let Ok(text) = std::fs::read_to_string(&dot) else {
                continue;
            };
            let Some(rel) = text
                .lines()
                .next()
                .and_then(|line| line.strip_prefix("gitdir:"))
            else {
                continue;
            };
            joined(top, rel.trim())
        } else {
            continue;
        };
        if !git_dir.join("HEAD").is_file() {
            continue;
        }
        let common_dir = match std::fs::read_to_string(git_dir.join("commondir")) {
            Ok(text) if !text.trim().is_empty() => joined(&git_dir, text.trim()),
            _ => git_dir.clone(),
        };
        return Some(Layout {
            top: top.to_path_buf(),
            git_dir,
            common_dir,
        });
    }
    None
}

/// What to put under watch for `path` and its repository. In none: `path`
/// itself, for a `.git` to appear there (and that `.git`, once it is a
/// directory still missing its HEAD). In one: the worktree top (`.git`
/// going), the git dir, the common dir, and `refs/` and `worktrees/`
/// recursively. A git dir inside the watched `worktrees/` is already covered.
fn targets(path: &Path, layout: Option<&Layout>) -> Vec<(PathBuf, RecursiveMode)> {
    let Some(layout) = layout else {
        let mut out = vec![(path.to_path_buf(), RecursiveMode::NonRecursive)];
        let dot = path.join(".git");
        if dot.is_dir() {
            out.push((dot, RecursiveMode::NonRecursive));
        }
        return out;
    };
    let mut out = Vec::new();
    let mut recursive = Vec::new();
    for sub in ["refs", "worktrees"] {
        let dir = layout.common_dir.join(sub);
        if dir.is_dir() {
            recursive.push(dir.clone());
            out.push((dir, RecursiveMode::Recursive));
        }
    }
    let mut plain = vec![layout.top.clone(), layout.git_dir.clone()];
    if layout.common_dir != layout.git_dir {
        plain.push(layout.common_dir.clone());
    }
    for dir in plain {
        if !recursive.iter().any(|covered| dir.starts_with(covered)) {
            out.push((dir, RecursiveMode::NonRecursive));
        }
    }
    out
}

/// What one changed path means for a subscriber, and whether the watch must
/// be set up again (the repository, or a directory it watches, came or went).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Classified {
    pub change: Option<&'static str>,
    pub rebuild: bool,
}

impl Classified {
    fn change(change: &'static str) -> Self {
        Self {
            change: Some(change),
            rebuild: false,
        }
    }
    fn rebuild(change: &'static str) -> Self {
        Self {
            change: Some(change),
            rebuild: true,
        }
    }
}

fn name_of(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

/// Classify one path a watch reported. Everything else under the watched
/// directories (reflogs, objects, lock files, the worktree's own files) is
/// nothing to a subscriber.
pub(crate) fn classify(path: &Path, watched: &Path, layout: Option<&Layout>) -> Classified {
    let Some(layout) = layout else {
        let dot = watched.join(".git");
        return if path.starts_with(&dot) {
            Classified::rebuild(REPOSITORY)
        } else {
            Classified::default()
        };
    };
    if path == layout.top.join(".git") || path == layout.git_dir || path == layout.common_dir {
        return Classified::rebuild(REPOSITORY);
    }
    let parent = path.parent();
    if parent == Some(layout.git_dir.as_path()) {
        match name_of(path) {
            "HEAD" => return Classified::change(HEAD),
            "index" => return Classified::change(INDEX),
            "packed-refs" => return Classified::change(REFS),
            "commondir" | "gitdir" => return Classified::rebuild(REPOSITORY),
            _ => {}
        }
    }
    if parent == Some(layout.common_dir.as_path()) {
        match name_of(path) {
            "packed-refs" => return Classified::change(REFS),
            // The main checkout's HEAD, seen from a linked worktree.
            "HEAD" if layout.git_dir != layout.common_dir => return Classified::change(WORKTREES),
            // A directory the watch covers recursively came or went.
            "refs" => return Classified::rebuild(REFS),
            "worktrees" => return Classified::rebuild(WORKTREES),
            _ => {}
        }
    }
    if path.starts_with(layout.common_dir.join("refs")) {
        return if name_of(path).ends_with(".lock") {
            Classified::default()
        } else {
            Classified::change(REFS)
        };
    }
    if let Ok(rel) = path.strip_prefix(layout.common_dir.join("worktrees")) {
        let parts: Vec<_> = rel.components().collect();
        return match parts.as_slice() {
            // A worktree's admin directory made or removed.
            [_] => Classified::change(WORKTREES),
            [_, head] if head.as_os_str() == "HEAD" => Classified::change(WORKTREES),
            _ => Classified::default(),
        };
    }
    Classified::default()
}

/// The open watch: the OS watcher and the layout it was set up for.
pub(crate) struct GitWatch {
    _watcher: notify::RecommendedWatcher,
    pub(crate) layout: Option<Layout>,
}

impl GitWatch {
    /// Blocking: find the repository and watch it. `missed` is raised when
    /// the channel was full or the kernel queue overflowed: the pump then
    /// treats the batch as everything having moved.
    pub(crate) fn open(
        path: &Path,
        tx: Sender<notify::Event>,
        missed: Arc<AtomicBool>,
    ) -> Option<Self> {
        let layout = discover(path);
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            let Ok(event) = res else { return };
            if is_noise_kind(&event.kind) {
                return;
            }
            if event.need_rescan() {
                missed.store(true, Ordering::Relaxed);
            }
            if let Err(TrySendError::Full(_)) = tx.try_send(event) {
                missed.store(true, Ordering::Relaxed);
            }
        });
        let mut watcher = match watcher {
            Ok(watcher) => watcher,
            Err(e) => {
                tracing::warn!(error = %e, "git watcher start failed");
                return None;
            }
        };
        for (dir, mode) in targets(path, layout.as_ref()) {
            // Gone since discovery, or unreadable: the rest still holds.
            if let Err(e) = watcher.watch(&dir, mode) {
                tracing::debug!(error = %e, dir = %dir.display(), "git watch skipped a directory");
            }
        }
        Some(Self {
            _watcher: watcher,
            layout,
        })
    }

    fn event(&self, path: &Path, changes: &BTreeSet<&'static str>) -> GitChangedEvent {
        let shown = |p: &Path| p.to_string_lossy().into_owned();
        GitChangedEvent {
            path: shown(path),
            worktree: self.layout.as_ref().map(|l| shown(&l.top)),
            git_dir: self.layout.as_ref().map(|l| shown(&l.git_dir)),
            common_dir: self.layout.as_ref().map(|l| shown(&l.common_dir)),
            changes: changes.iter().map(|c| c.to_string()).collect(),
        }
    }
}

/// Fold one batch of raw events into what moved, and whether to set the
/// watch up again.
pub(crate) fn fold(
    events: &[notify::Event],
    watched: &Path,
    layout: Option<&Layout>,
) -> (BTreeSet<&'static str>, bool) {
    let mut changes = BTreeSet::new();
    let mut rebuild = false;
    for event in events {
        for path in &event.paths {
            let found = classify(path, watched, layout);
            if let Some(change) = found.change {
                changes.insert(change);
            }
            rebuild |= found.rebuild;
        }
    }
    (changes, rebuild)
}

/// How one binding's events reach its function.
#[derive(Debug, Clone)]
struct Delivery {
    function_id: String,
    metadata: Option<Value>,
    namespace: Option<String>,
}

impl Delivery {
    fn from_config(config: &TriggerConfig) -> Self {
        Self {
            function_id: config.function_id.clone(),
            metadata: config.metadata.clone(),
            namespace: config.namespace.clone(),
        }
    }

    fn request(&self, payload: Value) -> iii_sdk::protocol::TriggerRequestWithMetadata {
        let mut request: iii_sdk::protocol::TriggerRequestWithMetadata = TriggerRequest {
            function_id: self.function_id.clone(),
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

type Subscribers = Arc<Mutex<HashMap<String, Delivery>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Batch raw events, classify, fan one event per batch out to every
/// binding. Owns the watch: aborting this task tears it down.
async fn pump(
    iii: IIIClient,
    path: PathBuf,
    mut watch: GitWatch,
    tx: Sender<notify::Event>,
    mut rx: tokio::sync::mpsc::Receiver<notify::Event>,
    missed: Arc<AtomicBool>,
    subscribers: Subscribers,
) {
    loop {
        let Some(first) = rx.recv().await else {
            return;
        };
        let mut batch = vec![first];
        let window = tokio::time::sleep(Duration::from_millis(COALESCE_MS));
        tokio::pin!(window);
        loop {
            tokio::select! {
                more = rx.recv() => match more {
                    Some(event) => batch.push(event),
                    None => break,
                },
                () = &mut window => break,
            }
        }
        let (mut changes, mut rebuild) = fold(&batch, &path, watch.layout.as_ref());
        if missed.swap(false, Ordering::Relaxed) {
            // Events were dropped: whatever they said, say everything.
            changes.extend([HEAD, INDEX, REFS, WORKTREES]);
            rebuild = true;
        }
        if rebuild {
            let before = watch.layout.clone();
            let (open_path, open_tx, open_missed) = (path.clone(), tx.clone(), missed.clone());
            let Ok(Some(next)) = tokio::task::spawn_blocking(move || {
                GitWatch::open(&open_path, open_tx, open_missed)
            })
            .await
            else {
                tracing::warn!(path = %path.display(), "git watch could not be set up again");
                return;
            };
            watch = next;
            if watch.layout != before {
                changes.insert(REPOSITORY);
            } else {
                changes.remove(REPOSITORY);
            }
        }
        if changes.is_empty() {
            continue;
        }
        let event = watch.event(&path, &changes);
        let Ok(payload) = serde_json::to_value(&event) else {
            continue;
        };
        let bound: Vec<Delivery> = lock(&subscribers).values().cloned().collect();
        for delivery in bound {
            let iii = iii.clone();
            let request = delivery.request(payload.clone());
            let function_id = delivery.function_id.clone();
            let changes = event.changes.join(",");
            tokio::spawn(async move {
                if let Err(e) = iii.trigger(request).await {
                    tracing::warn!(function_id = %function_id, error = %e, "shell::git-changed fan-out failed");
                } else {
                    tracing::debug!(function_id = %function_id, changes = %changes, "shell::git-changed delivered");
                }
            });
        }
    }
}

/// One watch, shared by every binding on its path.
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

fn start_watch(iii: IIIClient, path: PathBuf, subscribers: Subscribers) -> WatchEntry {
    let (started, ready) = tokio::sync::watch::channel(None);
    let bound = subscribers.clone();
    let task = tokio::spawn(async move {
        let (tx, rx) = tokio::sync::mpsc::channel::<notify::Event>(1024);
        let missed = Arc::new(AtomicBool::new(false));
        let (open_path, open_tx, open_missed) = (path.clone(), tx.clone(), missed.clone());
        let Ok(Some(watch)) =
            tokio::task::spawn_blocking(move || GitWatch::open(&open_path, open_tx, open_missed))
                .await
        else {
            started.send_replace(Some(false));
            return;
        };
        started.send_replace(Some(true));
        pump(iii, path, watch, tx, rx, missed, bound).await;
    });
    WatchEntry {
        subscribers,
        ready,
        task,
    }
}

/// The directory a binding follows, through the coder surface's path policy.
fn watch_path(config: &Value, resolver: &PathResolver) -> Result<PathBuf, Error> {
    let config: GitChangedConfig = serde_json::from_value(config.clone()).map_err(|e| {
        Error::Handler(format!(
            "shell::git-changed needs config.path — a directory in the repository: {e}"
        ))
    })?;
    let canon = resolver.resolve(&config.path).map_err(|e| {
        Error::Handler(format!(
            "shell::git-changed cannot watch {}: {e}",
            config.path
        ))
    })?;
    if !canon.is_dir() {
        return Err(Error::Handler(format!(
            "shell::git-changed config.path is not a directory: {}",
            canon.display()
        )));
    }
    Ok(canon)
}

struct GitChangedTriggerHandler {
    iii: IIIClient,
    watches: Arc<Mutex<HashMap<PathBuf, WatchEntry>>>,
    resolver: ResolverCell,
}

impl GitChangedTriggerHandler {
    fn unbind(&self, id: &str) {
        lock(&self.watches).retain(|_, entry| {
            let mut subscribers = lock(&entry.subscribers);
            subscribers.remove(id);
            !subscribers.is_empty()
        });
    }
}

#[async_trait]
impl TriggerHandler for GitChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        let resolver = self.resolver.read().await.clone();
        let path = watch_path(&config.config, &resolver)?;
        let shown = path.display().to_string();
        // A binding registered again (a reconnect) may name another path.
        self.unbind(&config.id);
        let mut ready = {
            let mut watches = lock(&self.watches);
            // A pump that died must not swallow every later binding.
            let orphans = match watches.get(&path) {
                Some(entry) if entry.task.is_finished() => {
                    watches.remove(&path).map(|e| e.subscribers.clone())
                }
                _ => None,
            };
            let entry = watches.entry(path.clone()).or_insert_with(|| {
                start_watch(self.iii.clone(), path, orphans.unwrap_or_default())
            });
            lock(&entry.subscribers).insert(config.id.clone(), Delivery::from_config(&config));
            entry.ready.clone()
        };
        let failed = ready
            .wait_for(Option::is_some)
            .await
            .is_ok_and(|started| *started == Some(false));
        if failed {
            self.unbind(&config.id);
            return Err(Error::Handler(format!(
                "shell::git-changed cannot watch {shown}: the watcher did not start"
            )));
        }
        tracing::info!(
            trigger_type = GIT_CHANGED,
            id = %config.id,
            function_id = %config.function_id,
            path = %shown,
            "git watch registered"
        );
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        tracing::info!(trigger_type = GIT_CHANGED, id = %config.id, "git watch unregistered");
        self.unbind(&config.id);
        Ok(())
    }
}

/// Register the trigger type. The SDK queues the registration and cannot
/// report failure; a dead type surfaces as bindings that never fire.
pub fn register_git_changed_trigger(iii: &IIIClient, resolver: ResolverCell) {
    let _handle = iii.register_trigger_type(
        RegisterTriggerType::new(
            GIT_CHANGED,
            "Fires when the git repository a directory sits in changes state — bind with \
             config: { path } naming any directory in the worktree (jail-checked like every \
             coder::* path). One event per ~200 ms batch, listing what moved in `changes`: \
             head (this worktree's HEAD: a switch, a checkout), index (staged changes, a \
             commit), refs (branches, tags, remotes, packed-refs), worktrees (another \
             worktree's HEAD, a worktree added or removed), repository (the repository \
             appeared, disappeared or moved — re-read everything). A path in no repository \
             fires repository once a .git appears in it. Worktree file edits are not \
             reported here: bind shell::changed for those.",
            GitChangedTriggerHandler {
                iii: iii.clone(),
                watches: Arc::new(Mutex::new(HashMap::new())),
                resolver,
            },
        )
        .trigger_request_format::<GitChangedConfig>()
        .call_request_format::<GitChangedEvent>(),
    );
    tracing::info!(
        trigger_type = GIT_CHANGED,
        "sent the trigger type registration; delivery is confirmed by the first subscription"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn git(dir: &Path, args: &[&str]) {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args([
                "-c",
                "init.defaultBranch=main",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "git {args:?}");
    }

    /// A repository with one commit on `main`.
    fn repo() -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap().join("repo");
        std::fs::create_dir_all(root.join("src")).unwrap();
        git(&root, &["init", "-q"]);
        std::fs::write(root.join("a.txt"), "a").unwrap();
        git(&root, &["add", "a.txt"]);
        git(&root, &["commit", "-q", "-m", "one"]);
        (tmp, root)
    }

    #[test]
    fn discover_finds_main_and_linked_worktrees_and_nothing_outside() {
        let (tmp, root) = repo();
        let main = discover(&root.join("src")).unwrap();
        assert_eq!(main.top, root);
        assert_eq!(main.git_dir, root.join(".git"));
        assert_eq!(main.common_dir, root.join(".git"));

        let linked_path = tmp.path().canonicalize().unwrap().join("linked");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                linked_path.to_str().unwrap(),
            ],
        );
        let linked = discover(&linked_path).unwrap();
        assert_eq!(linked.top, linked_path);
        assert_eq!(linked.common_dir, root.join(".git"));
        assert_eq!(linked.git_dir, root.join(".git/worktrees/linked"));

        let plain = tmp.path().canonicalize().unwrap().join("plain");
        std::fs::create_dir(&plain).unwrap();
        assert_eq!(discover(&plain), None);
        // A `.git` still missing its HEAD is not a repository yet.
        std::fs::create_dir(plain.join(".git")).unwrap();
        assert_eq!(discover(&plain), None);
    }

    #[test]
    fn classify_names_what_moved() {
        let layout = Layout {
            top: PathBuf::from("/w/linked"),
            git_dir: PathBuf::from("/r/.git/worktrees/linked"),
            common_dir: PathBuf::from("/r/.git"),
        };
        let at = |p: &str| classify(Path::new(p), Path::new("/w/linked"), Some(&layout));
        assert_eq!(
            at("/r/.git/worktrees/linked/HEAD"),
            Classified::change(HEAD)
        );
        assert_eq!(
            at("/r/.git/worktrees/linked/index"),
            Classified::change(INDEX)
        );
        assert_eq!(
            at("/r/.git/worktrees/linked/index.lock"),
            Classified::default()
        );
        assert_eq!(
            at("/r/.git/worktrees/other/HEAD"),
            Classified::change(WORKTREES)
        );
        assert_eq!(at("/r/.git/worktrees/other/index"), Classified::default());
        assert_eq!(at("/r/.git/worktrees/gone"), Classified::change(WORKTREES));
        assert_eq!(at("/r/.git/refs/heads/feat/x"), Classified::change(REFS));
        assert_eq!(at("/r/.git/refs/heads/main.lock"), Classified::default());
        assert_eq!(at("/r/.git/packed-refs"), Classified::change(REFS));
        assert_eq!(
            at("/r/.git/HEAD"),
            Classified::change(WORKTREES),
            "the main checkout's HEAD"
        );
        assert_eq!(at("/r/.git/objects/ab/cd"), Classified::default());
        assert_eq!(at("/r/.git/worktrees"), Classified::rebuild(WORKTREES));
        assert_eq!(at("/w/linked/.git"), Classified::rebuild(REPOSITORY));
        assert_eq!(at("/w/linked/README.md"), Classified::default());

        let none = |p: &str| classify(Path::new(p), Path::new("/plain"), None);
        assert_eq!(none("/plain/.git"), Classified::rebuild(REPOSITORY));
        assert_eq!(none("/plain/.git/HEAD"), Classified::rebuild(REPOSITORY));
        assert_eq!(none("/plain/notes.txt"), Classified::default());
    }

    #[test]
    fn a_linked_git_dir_under_a_watched_worktrees_dir_is_not_watched_twice() {
        let (tmp, root) = repo();
        let linked_path = tmp.path().canonicalize().unwrap().join("linked");
        git(
            &root,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "feature",
                linked_path.to_str().unwrap(),
            ],
        );
        let layout = discover(&linked_path).unwrap();
        let dirs: Vec<PathBuf> = targets(&linked_path, Some(&layout))
            .into_iter()
            .map(|(dir, _)| dir)
            .collect();
        assert!(dirs.contains(&root.join(".git/worktrees")));
        assert!(dirs.contains(&root.join(".git/refs")));
        assert!(dirs.contains(&root.join(".git")));
        assert!(dirs.contains(&linked_path));
        assert!(!dirs.contains(&layout.git_dir), "{dirs:?}");
    }

    /// Everything a watch reports within `ms`.
    async fn drain(
        rx: &mut tokio::sync::mpsc::Receiver<notify::Event>,
        ms: u64,
    ) -> Vec<notify::Event> {
        let mut out = Vec::new();
        let until = tokio::time::Instant::now() + Duration::from_millis(ms);
        while let Ok(Some(event)) = tokio::time::timeout_at(until, rx.recv()).await {
            out.push(event);
        }
        out
    }

    fn open(path: &Path) -> (GitWatch, tokio::sync::mpsc::Receiver<notify::Event>) {
        let (tx, rx) = tokio::sync::mpsc::channel(1024);
        (
            GitWatch::open(path, tx, Arc::new(AtomicBool::new(false))).unwrap(),
            rx,
        )
    }

    /// A `git switch` in a terminal is what the branch chip used to re-read
    /// git on every focus change for: it must arrive as `head`.
    #[tokio::test]
    async fn a_branch_switch_stage_and_commit_are_reported() {
        let (_tmp, root) = repo();
        let (watch, mut rx) = open(&root.join("src"));
        tokio::time::sleep(Duration::from_millis(100)).await;

        git(&root, &["switch", "-q", "-c", "topic"]);
        let (changes, _) = fold(&drain(&mut rx, 500).await, &root, watch.layout.as_ref());
        assert!(changes.contains(HEAD), "{changes:?}");
        assert!(changes.contains(REFS), "{changes:?}");

        std::fs::write(root.join("b.txt"), "b").unwrap();
        let (changes, _) = fold(&drain(&mut rx, 300).await, &root, watch.layout.as_ref());
        assert!(
            changes.is_empty(),
            "worktree edits are shell::changed's: {changes:?}"
        );

        git(&root, &["add", "b.txt"]);
        let (changes, _) = fold(&drain(&mut rx, 500).await, &root, watch.layout.as_ref());
        assert!(changes.contains(INDEX), "{changes:?}");
    }

    /// A folder in no repository waits for one: `git init` there is a
    /// `repository` change and a rebuild, after which its HEAD is watched.
    #[tokio::test]
    async fn a_repository_made_in_a_plain_folder_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().canonicalize().unwrap();
        let (watch, mut rx) = open(&plain);
        assert!(watch.layout.is_none());
        tokio::time::sleep(Duration::from_millis(100)).await;
        git(&plain, &["init", "-q"]);
        let (changes, rebuild) = fold(&drain(&mut rx, 500).await, &plain, None);
        assert!(changes.contains(REPOSITORY), "{changes:?}");
        assert!(rebuild);
        let (watch, mut rx) = open(&plain);
        assert_eq!(
            watch.layout.as_ref().map(|l| l.top.clone()),
            Some(plain.clone())
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
        git(&plain, &["symbolic-ref", "HEAD", "refs/heads/other"]);
        let (changes, _) = fold(&drain(&mut rx, 500).await, &plain, watch.layout.as_ref());
        assert!(changes.contains(HEAD), "{changes:?}");
    }

    #[test]
    fn the_event_names_the_layout_and_sorted_changes() {
        let (_tmp, root) = repo();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let watch = GitWatch::open(&root, tx, Arc::new(AtomicBool::new(false))).unwrap();
        let changes = BTreeSet::from([REFS, HEAD]);
        let event = serde_json::to_value(watch.event(&root, &changes)).unwrap();
        assert_eq!(event["changes"], json!(["head", "refs"]));
        assert_eq!(event["worktree"], json!(root.to_string_lossy()));
        assert_eq!(event["git_dir"], json!(root.join(".git").to_string_lossy()));
    }

    fn resolver_rooted_at(root: &Path) -> PathResolver {
        let cfg = crate::code::config::CoderConfig {
            base_paths: vec![root.to_path_buf()],
            ..Default::default()
        };
        PathResolver::new(&cfg).unwrap()
    }

    fn handler_at(root: &Path) -> GitChangedTriggerHandler {
        GitChangedTriggerHandler {
            iii: IIIClient::new("ws://127.0.0.1:1"),
            watches: Default::default(),
            resolver: Arc::new(tokio::sync::RwLock::new(Arc::new(resolver_rooted_at(root)))),
        }
    }

    fn binding(id: &str, path: &Path) -> TriggerConfig {
        TriggerConfig {
            id: id.into(),
            function_id: format!("probe::{id}"),
            config: json!({ "path": path }),
            metadata: Some(json!({ "__binding": id })),
            namespace: None,
        }
    }

    /// The IDE header and the chat's composer follow the same folder: one
    /// watch serves both, and goes with the last of them.
    #[tokio::test]
    async fn bindings_on_one_path_share_one_watch_until_the_last_goes() {
        let (_tmp, root) = repo();
        let handler = handler_at(&root);
        for id in ["b1", "b2"] {
            handler.register_trigger(binding(id, &root)).await.unwrap();
        }
        assert_eq!(lock(&handler.watches).len(), 1);
        handler
            .unregister_trigger(binding("b1", &root))
            .await
            .unwrap();
        assert_eq!(lock(&handler.watches).len(), 1);
        handler
            .unregister_trigger(binding("b2", &root))
            .await
            .unwrap();
        assert!(lock(&handler.watches).is_empty());
    }

    #[tokio::test]
    async fn a_binding_needs_a_directory_inside_the_jail() {
        let (_tmp, root) = repo();
        let handler = handler_at(&root);
        let err = handler
            .register_trigger(TriggerConfig {
                config: json!({}),
                ..binding("b1", &root)
            })
            .await
            .unwrap_err();
        assert!(err.to_string().contains("config.path"), "{err}");
        let err = handler
            .register_trigger(binding("b2", &root.join("a.txt")))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not a directory"), "{err}");
        let outside = tempfile::tempdir().unwrap();
        let err = handler
            .register_trigger(binding("b3", outside.path()))
            .await
            .unwrap_err();
        assert!(err.to_string().contains("cannot watch"), "{err}");
        assert!(lock(&handler.watches).is_empty());
    }

    #[test]
    fn deliveries_echo_the_binding_metadata() {
        let (_tmp, root) = repo();
        let delivery = Delivery::from_config(&binding("b1", &root));
        let debug = format!("{:?}", delivery.request(json!({ "changes": ["head"] })));
        assert!(debug.contains("__binding"), "{debug}");
        assert!(debug.contains("Void"), "{debug}");
    }
}
