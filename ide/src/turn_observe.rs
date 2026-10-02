//! Fold watched filesystem changes into the durable turn history.
//!
//! The turn log's harness hooks only see writes that go through a shell or
//! coder function; a `sed -i` inside `shell::exec`, a formatter, a build
//! step all bypass them, and a session reopened later showed "0 files" for
//! turns full of such work. This module puts an OS watch on the session's
//! workspace root for the duration of each turn and records what it sees as
//! `observed` changes: no pre-image of its own (the watch fires after the
//! write). Their sides come from the root's tree snapshots (`turn_snapshot`)
//! when the turn has them; a turn without falls back to the committed
//! version when one exists and says "nothing to diff" quietly otherwise.
//!
//! The watch starts on the first hooked call of a turn — the pre-trigger
//! hook is an awaited barrier, so on a root that walks in time the watcher
//! is live before that call can write — and stops a grace window after the
//! turn completes, letting the last coalesced burst land. Git internals,
//! this worker's own temp files, the turn store itself, and gitignored
//! paths stay out of the record; gitignored trees and symlinks are not
//! watched at all, and a root with too many directories goes unobserved.
//!
//! Two chats working in one workspace watch the same files, and a watch
//! cannot tell who wrote. So a write is recorded under a session only when
//! it can be its own: a path a hooked call of another session just touched
//! belongs to that call (`TurnLog::claimed_by_other`), and where another
//! session's watch covers the same path, only a session with a hooked call
//! running or just finished takes the write (`TurnLog::is_working`).

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use notify::{RecursiveMode, Watcher};
use tokio::sync::mpsc::Sender;

use crate::events::{
    ignored_under, in_repo, is_git_internal, is_noise_kind, is_own_temp, kind_of, merge_kinds,
    plain_ignores, resolve_kind,
};
use crate::turns::TurnLog;

/// Raw OS events batch this long before folding into the record.
const COALESCE_MS: u64 = 250;
/// How long a watch outlives its turn, so the final burst still lands.
const GRACE_MS: u64 = 1_200;
/// How long the pre-trigger hook waits for a new watch to go live — well
/// under the harness hook timeout. A root slower to walk lets the call
/// through unobserved (hook pre-images and the turn's snapshots still
/// cover it) while the walk finishes off the runtime.
const SETUP_WAIT_MS: u64 = 1_000;
/// Directories one root may put under watch, one inotify watch each, out
/// of a per-user budget (8192 by default on older kernels) shared with
/// editors and every other watcher. A root with more outside its
/// .gitignore goes unobserved; a new tree that would cross it, unwatched.
const MAX_WATCHED_DIRS: usize = 4_096;

struct ObserverEntry {
    turn_id: String,
    root: PathBuf,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for ObserverEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One live workspace watch per session with an active turn.
pub struct TurnObservers {
    log: Arc<TurnLog>,
    entries: StdMutex<HashMap<String, ObserverEntry>>,
    /// The turn store's own directory: folding a record writes here, and
    /// watching those writes back would loop forever.
    store_dir: PathBuf,
}

impl TurnObservers {
    pub fn new(log: Arc<TurnLog>, store_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            log,
            entries: StdMutex::new(HashMap::new()),
            store_dir,
        })
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, ObserverEntry>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// The turn currently observed for a session, read at fold time so a
    /// burst that spans a turn boundary lands under the right id.
    fn current_turn(&self, session_id: &str) -> Option<(String, PathBuf)> {
        self.lock()
            .get(session_id)
            .map(|entry| (entry.turn_id.clone(), entry.root.clone()))
    }

    /// Make sure a watch covers this session's root for this turn. Called
    /// from the awaited pre-trigger hook, so by the time a hooked call runs
    /// the watcher is live — unless the root takes over `SETUP_WAIT_MS` to
    /// walk.
    pub async fn ensure(self: &Arc<Self>, session_id: &str, turn_id: &str, root: Option<&str>) {
        let Some(root) = root else { return };
        let root = Path::new(root);
        if !root.is_absolute() {
            return;
        }
        // FSEvents reports resolved paths; an unresolved root would make
        // every strip_prefix miss (macOS /tmp vs /private/tmp).
        let Ok(root) = std::fs::canonicalize(root) else {
            return;
        };
        if !root.is_dir() {
            return;
        }
        self.ensure_with(session_id, turn_id, root, DirWatch::open)
            .await;
    }

    /// `ensure` with the watch setup handed in. The setup walks the root on
    /// a blocking thread: the SDK runs every handler on one runtime thread,
    /// and a walk there, or under `entries`, froze the whole worker. The
    /// entry goes in first, so a later call of the turn finds the setup in
    /// flight instead of walking again.
    async fn ensure_with(
        self: &Arc<Self>,
        session_id: &str,
        turn_id: &str,
        root: PathBuf,
        open: impl FnOnce(&Path, Sender<notify::Event>) -> Option<DirWatch> + Send + 'static,
    ) {
        let (live, ready) = tokio::sync::oneshot::channel::<()>();
        {
            let mut entries = self.lock();
            if let Some(entry) = entries.get_mut(session_id) {
                if entry.root == root {
                    entry.turn_id = turn_id.to_string();
                    return;
                }
                entries.remove(session_id);
            }
            let this = Arc::clone(self);
            let (session, turn, watch_root) =
                (session_id.to_string(), turn_id.to_string(), root.clone());
            let task = tokio::spawn(async move {
                let (tx, rx) = tokio::sync::mpsc::channel::<notify::Event>(1024);
                let walk_root = watch_root.clone();
                let Ok(Some(watch)) =
                    tokio::task::spawn_blocking(move || open(&walk_root, tx)).await
                else {
                    return;
                };
                tracing::info!(
                    session_id = %session,
                    turn_id = %turn,
                    root = %watch_root.display(),
                    dirs = watch.dirs,
                    "turn observe: watch started"
                );
                let _ = live.send(());
                pump(this, session, watch_root, watch, rx).await;
            });
            entries.insert(
                session_id.to_string(),
                ObserverEntry {
                    turn_id: turn_id.to_string(),
                    root,
                    task,
                },
            );
        }
        if tokio::time::timeout(Duration::from_millis(SETUP_WAIT_MS), ready)
            .await
            .is_err()
        {
            tracing::info!(
                session_id,
                turn_id,
                "turn observe: watch still starting; the call goes ahead"
            );
        }
    }

    /// A turn ended: keep its watch alive for the grace window, then tear
    /// it down — unless a newer turn took the session over meanwhile.
    pub fn complete(self: &Arc<Self>, session_id: &str, turn_id: &str) {
        let this = Arc::clone(self);
        let session_id = session_id.to_string();
        let turn_id = turn_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(GRACE_MS)).await;
            let mut entries = this.lock();
            if entries
                .get(&session_id)
                .is_some_and(|entry| entry.turn_id == turn_id)
            {
                entries.remove(&session_id);
            }
        });
    }

    /// The changes of a batch this session may call its own. A write only
    /// this session's watch covers is the session's. When another
    /// top-level session's watch covers the same path — two chats working
    /// in one workspace — the write is recorded here only while this
    /// session, a sub-agent of it included, has a hooked call running or
    /// just finished: an idle chat did not make it. A path a hooked call
    /// of another session just touched is dropped later, by the log.
    fn own_changes(
        &self,
        session_id: &str,
        turn_id: &str,
        changes: Vec<(String, &'static str)>,
    ) -> Vec<(String, &'static str)> {
        let target = self.log.resolve_root(session_id, turn_id);
        if self.log.is_working(&target.session_id) {
            return changes;
        }
        let other_roots: Vec<PathBuf> = {
            let entries = self.lock();
            entries
                .iter()
                .filter(|(other, entry)| {
                    other.as_str() != session_id
                        && self.log.resolve_root(other, &entry.turn_id).session_id
                            != target.session_id
                })
                .map(|(_, entry)| entry.root.clone())
                .collect()
        };
        if other_roots.is_empty() {
            return changes;
        }
        let before = changes.len();
        let kept: Vec<(String, &'static str)> = changes
            .into_iter()
            .filter(|(path, _)| {
                !other_roots
                    .iter()
                    .any(|root| Path::new(path).starts_with(root))
            })
            .collect();
        if kept.len() < before {
            tracing::debug!(
                session_id,
                dropped = before - kept.len(),
                "turn observe: shared-workspace writes left to the session at work"
            );
        }
        kept
    }
}

/// A bounded workspace watch: one non-recursive OS watch per directory
/// kept. notify's recursive inotify watch walks every directory under the
/// root, symlinks followed and gitignored trees included; on a monorepo
/// root that held the worker for 44 s and nearly every inotify watch the
/// user had.
struct DirWatch {
    watcher: notify::RecommendedWatcher,
    dirs: usize,
}

impl DirWatch {
    /// Blocking. `None` when the watcher will not start or the root has
    /// too many directories to observe.
    fn open(root: &Path, tx: Sender<notify::Event>) -> Option<Self> {
        let watcher = notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = tx.try_send(event);
            }
        });
        let mut watch = match watcher {
            Ok(watcher) => Self { watcher, dirs: 0 },
            Err(e) => {
                tracing::warn!(error = %e, "turn observe: watcher start failed");
                return None;
            }
        };
        watch.add_tree(root, root).then_some(watch)
    }

    /// Blocking: watch `dir` and the directories `watch_dirs` keeps under
    /// it. False, with nothing watched, when that would cross
    /// `MAX_WATCHED_DIRS`.
    fn add_tree(&mut self, root: &Path, dir: &Path) -> bool {
        let Some(dirs) = watch_dirs(root, dir, MAX_WATCHED_DIRS.saturating_sub(self.dirs)) else {
            tracing::warn!(
                root = %root.display(),
                dir = %dir.display(),
                cap = MAX_WATCHED_DIRS,
                "turn observe: too many directories to watch; this tree goes unobserved \
                 (hook pre-images and snapshots still cover the turn)"
            );
            return false;
        };
        for dir in dirs {
            match self.watcher.watch(&dir, RecursiveMode::NonRecursive) {
                Ok(()) => self.dirs += 1,
                // Out of inotify watches: the rest would fail too.
                Err(e) if matches!(e.kind, notify::ErrorKind::MaxFilesWatch) => {
                    tracing::warn!(error = %e, root = %root.display(), "turn observe: watch failed");
                    break;
                }
                // Gone since the walk, or unreadable: nothing to watch.
                Err(_) => {}
            }
        }
        true
    }
}

/// The directories a watch keeps from `dir` down, by `root`'s ignore
/// rules: gitignored trees (outside a repository, what `ignored_under`
/// ignores), `.git` and symlinks stay out. `None` past `cap`; the walk
/// stops there.
fn watch_dirs(root: &Path, dir: &Path, cap: usize) -> Option<Vec<PathBuf>> {
    let plain = if in_repo(root) {
        None
    } else {
        plain_ignores(root)
    };
    let mut walker = ignore::WalkBuilder::new(dir);
    walker
        .follow_links(false)
        .hidden(false)
        .ignore(false)
        .filter_entry(move |entry| {
            entry.file_type().is_some_and(|t| t.is_dir())
                && entry.file_name() != ".git"
                && !plain
                    .as_ref()
                    .is_some_and(|m| m.matched(entry.path(), true).is_ignore())
        });
    let mut dirs = Vec::new();
    for entry in walker.build().flatten() {
        if dirs.len() == cap {
            return None;
        }
        dirs.push(entry.into_path());
    }
    Some(dirs)
}

/// Coalesce raw events and fold each batch into the session's current turn.
/// Owns the watch: aborting this task tears it down.
async fn pump(
    observers: Arc<TurnObservers>,
    session_id: String,
    root: PathBuf,
    mut watch: DirWatch,
    mut rx: tokio::sync::mpsc::Receiver<notify::Event>,
) {
    let store_dir = observers.store_dir.clone();
    loop {
        let Some(first) = rx.recv().await else {
            return;
        };
        let mut batch: HashMap<String, &'static str> = HashMap::new();
        let mut born: HashSet<String> = HashSet::new();
        let mut new_dirs: HashSet<String> = HashSet::new();
        let mut fold = |event: notify::Event| {
            if is_noise_kind(&event.kind) {
                return;
            }
            let kind = kind_of(&event.kind);
            for p in &event.paths {
                if is_git_internal(p) || is_own_temp(p) || p.starts_with(&store_dir) {
                    continue;
                }
                if p == &root || !p.starts_with(&root) {
                    continue;
                }
                if kind != "deleted" && p.is_dir() {
                    // Made or moved in under a watched directory: a
                    // recursive watch would cover it, so this one does.
                    if !p.is_symlink() {
                        new_dirs.insert(p.to_string_lossy().into_owned());
                    }
                    continue;
                }
                let key = p.to_string_lossy().into_owned();
                if kind == "created" {
                    born.insert(key.clone());
                }
                batch
                    .entry(key)
                    .and_modify(|prev| *prev = merge_kinds(prev, kind))
                    .or_insert(kind);
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
            let Some(grown) = watch_new_dirs(&root, watch, new_dirs).await else {
                return;
            };
            watch = grown;
        }
        if batch.is_empty() {
            continue;
        }
        let Some((turn_id, turn_root)) = observers.current_turn(&session_id) else {
            return;
        };
        let ignored = ignored_set(&root, batch.keys()).await;
        let changes: Vec<(String, &'static str)> = batch
            .drain()
            .filter(|(path, _)| !ignored.contains(path))
            .filter_map(|(path, kind)| {
                let on_disk = Path::new(&path).exists();
                let kind = resolve_kind(kind, on_disk, born.contains(&path))?;
                Some((path, kind))
            })
            .collect();
        let changes = observers.own_changes(&session_id, &turn_id, changes);
        if changes.is_empty() {
            continue;
        }
        let root_str = turn_root.to_string_lossy().into_owned();
        observers
            .log
            .fold_observed(&session_id, &turn_id, &root_str, changes)
            .await;
    }
}

/// Watch the trees of directories new under the root, gitignored ones
/// left out. Off the runtime: a tree copied in can be big.
async fn watch_new_dirs(
    root: &Path,
    mut watch: DirWatch,
    dirs: HashSet<String>,
) -> Option<DirWatch> {
    let ignored = ignored_set(root, dirs.iter()).await;
    let root = root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        for dir in dirs.iter().filter(|dir| !ignored.contains(*dir)) {
            watch.add_tree(&root, Path::new(dir));
        }
        watch
    })
    .await
    .ok()
}

/// The subset of `paths` that git ignores under `root`. A root that is not
/// a repository, or a host without git, ignores nothing.
async fn ignored_set<'a>(root: &Path, paths: impl Iterator<Item = &'a String>) -> HashSet<String> {
    let rels: Vec<(String, &'a String)> = paths
        .filter_map(|abs| {
            Path::new(abs)
                .strip_prefix(root)
                .ok()
                .map(|rel| (rel.to_string_lossy().into_owned(), abs))
        })
        .collect();
    let ignored = ignored_under(root, rels.iter().map(|(rel, _)| rel)).await;
    rels.into_iter()
        .filter(|(rel, _)| ignored.contains(rel))
        .map(|(_, abs)| abs.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::turns::TurnStore;

    fn observers_in(dir: &Path) -> Arc<TurnObservers> {
        let log = Arc::new(TurnLog::new(TurnStore::new(dir.join("turns"), 1024 * 1024)));
        TurnObservers::new(log, dir.join("turns"))
    }

    /// A live watch entry without the OS watcher behind it.
    fn watch(observers: &Arc<TurnObservers>, session_id: &str, turn_id: &str, root: &Path) {
        observers.lock().insert(
            session_id.to_string(),
            ObserverEntry {
                turn_id: turn_id.to_string(),
                root: root.to_path_buf(),
                task: tokio::spawn(async {}),
            },
        );
    }

    fn change(path: &str) -> Vec<(String, &'static str)> {
        vec![(path.to_string(), "modified")]
    }

    fn kept(root: &Path, cap: usize) -> Option<Vec<String>> {
        let mut names: Vec<String> = watch_dirs(root, root, cap)?
            .iter()
            .map(|dir| {
                dir.strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        Some(names)
    }

    #[test]
    fn the_watch_walk_leaves_out_ignored_trees_git_and_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().join("repo");
        for sub in [".git/objects", ".github", "src/deep", "target/debug"] {
            std::fs::create_dir_all(repo.join(sub)).unwrap();
        }
        std::fs::write(repo.join(".gitignore"), "target/\n").unwrap();
        std::os::unix::fs::symlink(repo.join("src"), repo.join("link")).unwrap();
        assert_eq!(
            kept(&repo, 100).unwrap(),
            ["", ".github", "src", "src/deep"]
        );
        // Outside a repository: what `ignored_under` ignores there.
        let plain = dir.path().join("plain");
        for sub in ["node_modules/pkg", ".iii/state", "app"] {
            std::fs::create_dir_all(plain.join(sub)).unwrap();
        }
        assert_eq!(kept(&plain, 100).unwrap(), ["", "app"]);
    }

    #[test]
    fn the_watch_walk_gives_up_past_its_cap() {
        let dir = tempfile::tempdir().unwrap();
        for sub in ["a", "b", "c"] {
            std::fs::create_dir_all(dir.path().join(sub)).unwrap();
        }
        assert_eq!(kept(dir.path(), 4).unwrap().len(), 4);
        assert!(kept(dir.path(), 3).is_none());
    }

    /// The SDK runs handlers on one current-thread runtime, as this test
    /// does: a setup that hangs must not hold the hook past its bound, and
    /// a second call of the turn must not start another.
    #[tokio::test]
    async fn a_slow_watch_setup_does_not_hold_the_hook() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Instant;
        let dir = tempfile::tempdir().unwrap();
        let observers = observers_in(dir.path());
        let setups = Arc::new(AtomicUsize::new(0));
        let (release, gate) = std::sync::mpsc::channel::<()>();
        let slow = {
            let setups = setups.clone();
            move |_: &Path, _| {
                setups.fetch_add(1, Ordering::SeqCst);
                let _ = gate.recv_timeout(Duration::from_secs(10));
                None
            }
        };
        let started = Instant::now();
        observers
            .ensure_with("s1", "t1", dir.path().to_path_buf(), slow)
            .await;
        assert!(started.elapsed() < Duration::from_millis(SETUP_WAIT_MS + 1_000));

        let again = {
            let setups = setups.clone();
            move |_: &Path, _| {
                setups.fetch_add(1, Ordering::SeqCst);
                std::thread::sleep(Duration::from_millis(SETUP_WAIT_MS * 2));
                None
            }
        };
        let started = Instant::now();
        observers
            .ensure_with("s1", "t1", dir.path().to_path_buf(), again)
            .await;
        assert!(started.elapsed() < Duration::from_millis(SETUP_WAIT_MS / 2));
        assert_eq!(setups.load(Ordering::SeqCst), 1);
        drop(release);
    }

    #[tokio::test]
    async fn a_directory_made_mid_turn_is_watched_too() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("w")).unwrap();
        let root = dir.path().join("w").canonicalize().unwrap();
        let observers = observers_in(dir.path());
        observers
            .ensure("s1", "t1", Some(&root.to_string_lossy()))
            .await;
        std::fs::create_dir_all(root.join("new/deeper")).unwrap();
        tokio::time::sleep(Duration::from_millis(COALESCE_MS * 4)).await;
        let file = root.join("new/deeper/a.txt");
        std::fs::write(&file, "a").unwrap();
        let file = file.to_string_lossy().into_owned();
        for _ in 0..50 {
            tokio::time::sleep(Duration::from_millis(100)).await;
            let record = observers.log.load("s1").await.unwrap();
            if record
                .turns
                .iter()
                .any(|turn| turn.files.iter().any(|f| f.path == file))
            {
                return;
            }
        }
        panic!("a write in a directory made mid-turn was never observed");
    }

    #[tokio::test]
    async fn a_lone_watch_keeps_what_it_sees() {
        let dir = tempfile::tempdir().unwrap();
        let observers = observers_in(dir.path());
        watch(&observers, "s1", "t1", Path::new("/w"));
        assert_eq!(
            observers.own_changes("s1", "t1", change("/w/a.rs")).len(),
            1
        );
    }

    #[tokio::test]
    async fn an_idle_session_leaves_a_shared_workspace_write_alone() {
        let dir = tempfile::tempdir().unwrap();
        let observers = observers_in(dir.path());
        watch(&observers, "s1", "t1", Path::new("/w"));
        watch(&observers, "s2", "t2", Path::new("/w"));
        assert!(observers
            .own_changes("s1", "t1", change("/w/a.rs"))
            .is_empty());
        // Only the paths the other watch covers are contested.
        watch(&observers, "s2", "t2", Path::new("/w/sub"));
        assert_eq!(
            observers.own_changes("s1", "t1", change("/w/a.rs")).len(),
            1
        );
        assert!(observers
            .own_changes("s1", "t1", change("/w/sub/b.rs"))
            .is_empty());
    }

    #[tokio::test]
    async fn a_session_at_work_keeps_a_shared_workspace_write() {
        let dir = tempfile::tempdir().unwrap();
        let observers = observers_in(dir.path());
        watch(&observers, "s1", "t1", Path::new("/w"));
        watch(&observers, "s2", "t2", Path::new("/w"));
        observers.log.begin_call("s1");
        assert_eq!(
            observers.own_changes("s1", "t1", change("/w/a.rs")).len(),
            1
        );
        assert!(observers
            .own_changes("s2", "t2", change("/w/a.rs"))
            .is_empty());
    }

    #[tokio::test]
    async fn a_parent_and_its_sub_agent_do_not_contest_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let observers = observers_in(dir.path());
        observers.log.link_parent("child", "parent", "pt");
        watch(&observers, "parent", "pt", Path::new("/w"));
        watch(&observers, "child", "ct", Path::new("/w"));
        assert_eq!(
            observers
                .own_changes("child", "ct", change("/w/a.rs"))
                .len(),
            1
        );
        assert_eq!(
            observers
                .own_changes("parent", "pt", change("/w/a.rs"))
                .len(),
            1
        );
        // The sub-agent's call is the parent's work; the other chat stays out.
        watch(&observers, "other", "ot", Path::new("/w"));
        observers.log.begin_call("parent");
        assert_eq!(
            observers
                .own_changes("child", "ct", change("/w/a.rs"))
                .len(),
            1
        );
        assert!(observers
            .own_changes("other", "ot", change("/w/a.rs"))
            .is_empty());
    }
}
