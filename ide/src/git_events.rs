//! `shell::on-git-changed` — a repository's own state, watched.
//!
//! `shell::on-changed` leaves `.git` out on purpose: git internals churn on
//! every operation and mean nothing to a file tree. A surface that shows the
//! repository itself (the branch chip, the Git window's log) still has to
//! learn that HEAD moved, the index changed or a branch appeared — a
//! `git switch` in a terminal says nothing else. Without this function the
//! only way to learn it is to ask git again on every focus change.
//!
//! An observer passes `args: { path }` naming any directory inside a
//! worktree (jail-checked like every `coder::*` path). The worker finds the
//! repository the way git does (the nearest `.git`, a directory or a
//! `gitdir:` file, and its `commondir`) and watches only what git state lives
//! in: the worktree's git dir (`HEAD`, `index`), the common dir
//! (`packed-refs`), `refs/**` and the other worktrees' `HEAD`s, plus the
//! worktree top for `.git` itself coming or going. Raw events are batched
//! for `COALESCE_MS` and become ONE call per batch naming what moved.
//!
//! A `path` in no repository is watched for a `.git` to appear (`git init`,
//! a clone into it): the observer hears `repository` once, and from then
//! on the repository's own changes. Removing the repository says
//! `repository` again and goes back to waiting. No polling anywhere.
//!
//! One watch per path, shared by its observers and torn down with the last.
//! The function's match hook files each observer under its canonical path,
//! the event's `path`, so a call reaches the observers of its own watch.
//! Calls mirror [`crate::events`]: `Void` routing, each on its own task.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    AddedHook, Call, CallAction, CallRequest, IIIClient, Keys, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use notify::{RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::Sender;

use crate::code::path::PathResolver;
use crate::code::state::ResolverCell;
use crate::events::{is_noise_kind, INVALID_PATH, WATCH_FAILED};

/// The function a surface observes to follow a repository's state.
pub const ON_GIT_CHANGED_FN: &str = "shell::on-git-changed";

/// Batching window, as `shell::on-changed`'s: a `git commit` writes the index,
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

/// An observer's args, `{ path }`: any directory inside a worktree, or one
/// that may become a repository.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct GitChangedArgs {
    /// The directory whose repository to follow (jail-checked).
    pub path: String,
}

/// One batch of repository changes. Lean like [`crate::events::ChangedEvent`]:
/// a consumer that wants the new state asks git.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
pub struct GitChangedEvent {
    /// The watched directory: the observer's `path`, canonical.
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
            // A `.git` symlinked elsewhere: FSEvents reports the target's paths.
            dot.canonicalize().unwrap_or(dot)
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

/// What one changed path means for an observer, and whether the watch must
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
/// nothing to an observer.
pub(crate) fn classify(path: &Path, watched: &Path, layout: Option<&Layout>) -> Classified {
    let Some(layout) = layout else {
        let dot = watched.join(".git");
        // A `.git` symlinked elsewhere: FSEvents reports the target's paths.
        return if path.starts_with(&dot)
            || dot.canonicalize().is_ok_and(|real| path.starts_with(real))
        {
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

/// The observers one watch serves, by id.
type Members = Arc<Mutex<HashSet<String>>>;

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// The match key of a call: the watched path its event names.
fn call_path(call: &Call) -> Option<&str> {
    call.input.get("path").and_then(Value::as_str)
}

/// Batch raw events, classify, and call the function once per batch. Owns
/// the watch: aborting this task tears it down.
async fn pump(
    iii: IIIClient,
    path: PathBuf,
    mut watch: GitWatch,
    tx: Sender<notify::Event>,
    mut rx: tokio::sync::mpsc::Receiver<notify::Event>,
    missed: Arc<AtomicBool>,
) {
    let probe = Call {
        input: json!({ "path": path.to_string_lossy() }),
        ..Call::default()
    };
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
        if changes.is_empty() || !iii.is_observed_for(ON_GIT_CHANGED_FN, &probe) {
            continue;
        }
        let event = watch.event(&path, &changes);
        let Ok(payload) = serde_json::to_value(&event) else {
            continue;
        };
        let iii = iii.clone();
        let request = CallRequest {
            function_id: ON_GIT_CHANGED_FN.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        let changes = event.changes.join(",");
        tokio::spawn(async move {
            if let Err(e) = iii.call(request).await {
                tracing::warn!(error = %e, "{ON_GIT_CHANGED_FN} call failed");
            } else {
                tracing::debug!(changes = %changes, "{ON_GIT_CHANGED_FN} called");
            }
        });
    }
}

/// One watch, shared by every observer of its path.
struct WatchEntry {
    observers: Members,
    /// `Some(started)` once the setup is over.
    ready: tokio::sync::watch::Receiver<Option<bool>>,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for WatchEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn start_watch(iii: IIIClient, path: PathBuf, observers: Members) -> WatchEntry {
    let (started, ready) = tokio::sync::watch::channel(None);
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
        pump(iii, path, watch, tx, rx, missed).await;
    });
    WatchEntry {
        observers,
        ready,
        task,
    }
}

/// The directory an observer follows, through the coder surface's path policy.
fn watch_path(path: &str, resolver: &PathResolver) -> Result<PathBuf, ObserveRejection> {
    let canon = resolver.resolve(path).map_err(|e| {
        ObserveRejection::new(
            INVALID_PATH,
            format!("{ON_GIT_CHANGED_FN} cannot watch {path}: {e}"),
        )
    })?;
    if !canon.is_dir() {
        return Err(ObserveRejection::new(
            INVALID_PATH,
            format!(
                "{ON_GIT_CHANGED_FN} args.path is not a directory: {}",
                canon.display()
            ),
        ));
    }
    Ok(canon)
}

/// `shell::on-git-changed`'s hooks: one watch per canonical path, started
/// by its first observer and stopped with its last.
#[derive(Clone)]
struct GitChangedSource {
    iii: IIIClient,
    watches: Arc<Mutex<HashMap<PathBuf, WatchEntry>>>,
    /// The canonical path of each accepted observer, by observer id: what
    /// the match hook files it under.
    keys: Arc<Mutex<HashMap<String, String>>>,
    resolver: ResolverCell,
}

impl GitChangedSource {
    fn release(&self, id: &str) {
        lock(&self.keys).remove(id);
        lock(&self.watches).retain(|_, entry| {
            let mut observers = lock(&entry.observers);
            observers.remove(id);
            !observers.is_empty()
        });
    }

    fn key_of(&self, id: &str) -> Option<String> {
        lock(&self.keys).get(id).cloned()
    }

    async fn add(&self, id: &str, args: &GitChangedArgs) -> Result<(), ObserveRejection> {
        let resolver = self.resolver.read().await.clone();
        let path = watch_path(&args.path, &resolver)?;
        let shown = path.display().to_string();
        // An observer added again (its args changed) may name another path.
        self.release(id);
        let mut ready = {
            let mut watches = lock(&self.watches);
            // A pump that died must not swallow every later observer.
            let orphans = match watches.get(&path) {
                Some(entry) if entry.task.is_finished() => {
                    watches.remove(&path).map(|e| e.observers.clone())
                }
                _ => None,
            };
            let entry = watches.entry(path.clone()).or_insert_with(|| {
                start_watch(self.iii.clone(), path.clone(), orphans.unwrap_or_default())
            });
            lock(&entry.observers).insert(id.to_string());
            entry.ready.clone()
        };
        let failed = ready
            .wait_for(Option::is_some)
            .await
            .is_ok_and(|started| *started == Some(false));
        if failed {
            self.release(id);
            return Err(ObserveRejection::new(
                WATCH_FAILED,
                format!("{ON_GIT_CHANGED_FN} cannot watch {shown}: the watcher did not start"),
            ));
        }
        lock(&self.keys).insert(id.to_string(), path.to_string_lossy().into_owned());
        tracing::info!(
            function_id = ON_GIT_CHANGED_FN,
            observer_id = %id,
            path = %shown,
            "git watch observed"
        );
        Ok(())
    }
}

#[async_trait]
impl AddedHook for GitChangedSource {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        let args: GitChangedArgs = obs.args()?;
        self.add(obs.id(), &args).await
    }
}

#[async_trait]
impl RemovedHook for GitChangedSource {
    async fn on_removed(&self, obs: Observer) {
        tracing::info!(function_id = ON_GIT_CHANGED_FN, observer_id = %obs.id(), "git watch released");
        self.release(obs.id());
    }
}

/// Register `shell::on-git-changed` and its hooks, set by id before the
/// function so they run for every observer.
pub fn register_git_changed(iii: &IIIClient, resolver: ResolverCell) {
    let source = GitChangedSource {
        iii: iii.clone(),
        watches: Arc::new(Mutex::new(HashMap::new())),
        keys: Arc::new(Mutex::new(HashMap::new())),
        resolver,
    };
    let keys = source.clone();
    iii.on_observer_added_for(ON_GIT_CHANGED_FN, source.clone())
        .on_observer_removed_for(ON_GIT_CHANGED_FN, source)
        .on_observer_match_for(
            ON_GIT_CHANGED_FN,
            Match::keys(
                move |obs| Keys::from(keys.key_of(obs.id())),
                |call| Keys::from(call_path(call)),
            ),
        );
    iii.register_function(
        ON_GIT_CHANGED_FN,
        RegisterFunction::new(|event: GitChangedEvent| Ok::<_, Error>(event))
            .description(
                "Called when the git repository a directory sits in changes state; observers \
                 receive the batch { path, worktree?, git_dir?, common_dir?, changes }. Observe \
                 with args { path } naming any directory in the worktree (jail-checked like \
                 every coder::* path; INVALID_PATH otherwise, WATCH_FAILED when the watcher \
                 does not start). One call per ~200 ms batch, listing what moved in `changes`: \
                 head (this worktree's HEAD: a switch, a checkout), index (staged changes, a \
                 commit), refs (branches, tags, remotes, packed-refs), worktrees (another \
                 worktree's HEAD, a worktree added or removed), repository (the repository \
                 appeared, disappeared or moved — re-read everything). A path in no \
                 repository reports repository once a .git appears in it. Worktree file edits \
                 are shell::on-changed's.",
            )
            .args_format::<GitChangedArgs>()
            .private(),
    );
    tracing::info!(
        function_id = ON_GIT_CHANGED_FN,
        "registered the repository state feed"
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
            "worktree edits are shell::on-changed's: {changes:?}"
        );

        git(&root, &["add", "b.txt"]);
        let (changes, _) = fold(&drain(&mut rx, 500).await, &root, watch.layout.as_ref());
        assert!(changes.contains(INDEX), "{changes:?}");
    }

    /// A `.git` symlinked to the real git dir: FSEvents reports the target's
    /// paths, so HEAD, index and ref changes must be matched there.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_symlinked_git_dir_reports_head_index_and_refs() {
        let (tmp, root) = repo();
        let real = tmp.path().canonicalize().unwrap().join("real.git");
        std::fs::rename(root.join(".git"), &real).unwrap();
        std::os::unix::fs::symlink(&real, root.join(".git")).unwrap();
        let (watch, mut rx) = open(&root);
        tokio::time::sleep(Duration::from_millis(100)).await;

        git(&root, &["switch", "-q", "-c", "topic"]);
        let (changes, _) = fold(&drain(&mut rx, 500).await, &root, watch.layout.as_ref());
        assert!(changes.contains(HEAD), "{changes:?}");
        assert!(changes.contains(REFS), "{changes:?}");

        std::fs::write(root.join("b.txt"), "b").unwrap();
        git(&root, &["add", "b.txt"]);
        let (changes, _) = fold(&drain(&mut rx, 500).await, &root, watch.layout.as_ref());
        assert!(changes.contains(INDEX), "{changes:?}");

        // inotify reports the watched form, so name the target everywhere.
        let event = watch.event(&root, &BTreeSet::from([HEAD]));
        assert_eq!(event.git_dir, Some(real.to_string_lossy().into_owned()));
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

    /// A `.git` symlinked to an empty directory before `git init`: FSEvents
    /// reports the target's paths, and the repository must still be seen.
    #[cfg(unix)]
    #[tokio::test]
    async fn a_repository_made_through_a_symlinked_git_dir_is_reported() {
        let tmp = tempfile::tempdir().unwrap();
        let plain = tmp.path().canonicalize().unwrap().join("plain");
        let real = tmp.path().canonicalize().unwrap().join("real.git");
        std::fs::create_dir(&plain).unwrap();
        std::fs::create_dir(&real).unwrap();
        std::os::unix::fs::symlink(&real, plain.join(".git")).unwrap();
        assert_eq!(
            classify(&real.join("HEAD"), &plain, None),
            Classified::rebuild(REPOSITORY)
        );
        let (watch, mut rx) = open(&plain);
        assert!(watch.layout.is_none());
        tokio::time::sleep(Duration::from_millis(100)).await;
        git(&plain, &["init", "-q"]);
        let (changes, rebuild) = fold(&drain(&mut rx, 500).await, &plain, None);
        assert!(changes.contains(REPOSITORY), "{changes:?}");
        assert!(rebuild);
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

    fn source_at(root: &Path) -> GitChangedSource {
        GitChangedSource {
            iii: IIIClient::new("ws://127.0.0.1:1"),
            watches: Default::default(),
            keys: Default::default(),
            resolver: Arc::new(tokio::sync::RwLock::new(Arc::new(resolver_rooted_at(root)))),
        }
    }

    fn args(path: &Path) -> GitChangedArgs {
        GitChangedArgs {
            path: path.to_string_lossy().into_owned(),
        }
    }

    /// The IDE header and the chat's composer follow the same folder: one
    /// watch serves both, and goes with the last of them. Each observer is
    /// filed under the canonical path its watch's events name.
    #[tokio::test]
    async fn observers_of_one_path_share_one_watch_until_the_last_goes() {
        let (_tmp, root) = repo();
        let source = source_at(&root);
        for id in ["o1", "o2"] {
            source.add(id, &args(&root)).await.unwrap();
        }
        assert_eq!(lock(&source.watches).len(), 1);
        let key = source.key_of("o1").unwrap();
        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        let watch = GitWatch::open(&root, tx, Arc::new(AtomicBool::new(false))).unwrap();
        let event = serde_json::to_value(watch.event(&root, &BTreeSet::from([HEAD]))).unwrap();
        let call = Call {
            input: event,
            ..Call::default()
        };
        assert_eq!(call_path(&call), Some(key.as_str()));
        source.release("o1");
        assert_eq!(lock(&source.watches).len(), 1);
        assert_eq!(source.key_of("o1"), None);
        source.release("o2");
        assert!(lock(&source.watches).is_empty());
    }

    #[tokio::test]
    async fn an_observer_needs_a_directory_inside_the_jail() {
        let (_tmp, root) = repo();
        let source = source_at(&root);
        assert!(serde_json::from_value::<GitChangedArgs>(json!({})).is_err());
        let err = source
            .add("o1", &args(&root.join("a.txt")))
            .await
            .unwrap_err();
        assert_eq!(err.code, INVALID_PATH);
        assert!(err.message.contains("not a directory"), "{err}");
        let outside = tempfile::tempdir().unwrap();
        let err = source.add("o2", &args(outside.path())).await.unwrap_err();
        assert_eq!(err.code, INVALID_PATH);
        assert!(err.message.contains("cannot watch"), "{err}");
        assert!(lock(&source.watches).is_empty());
        assert_eq!(source.key_of("o1"), None);
    }
}
