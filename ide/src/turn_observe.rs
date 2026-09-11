//! Fold watched filesystem changes into the durable turn history.
//!
//! The turn log's harness hooks only see writes that go through a shell or
//! coder function; a `sed -i` inside `shell::exec`, a formatter, a build
//! step all bypass them, and a session reopened later showed "0 files" for
//! turns full of such work. This module puts an OS watch on the session's
//! workspace root for the duration of each turn and records what it sees as
//! `observed` changes: no pre-image (the watch fires after the write), so a
//! reopened review diffs them against the committed version when one exists
//! and says "nothing to diff" quietly when one does not.
//!
//! The watch starts on the first hooked call of a turn — the pre-trigger
//! hook is an awaited barrier, so the watcher is live before that call can
//! write — and stops a grace window after the turn completes, letting the
//! last coalesced burst land. Git internals, this worker's own temp files,
//! the turn store itself, and gitignored paths stay out of the record.
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

use crate::events::{
    ignored_under, is_git_internal, is_noise_kind, is_own_temp, kind_of, merge_kinds, resolve_kind,
};
use crate::turns::TurnLog;

/// Raw OS events batch this long before folding into the record.
const COALESCE_MS: u64 = 250;
/// How long a watch outlives its turn, so the final burst still lands.
const GRACE_MS: u64 = 1_200;

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
    /// from the awaited pre/post hooks, so by the time a hooked call runs
    /// the watcher is already live.
    pub fn ensure(self: &Arc<Self>, session_id: &str, turn_id: &str, root: Option<&str>) {
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

        let mut entries = self.lock();
        if let Some(entry) = entries.get_mut(session_id) {
            if entry.root == root {
                entry.turn_id = turn_id.to_string();
                return;
            }
            entries.remove(session_id);
        }

        let (tx, rx) = tokio::sync::mpsc::channel::<notify::Event>(1024);
        let mut watcher = match notify::recommended_watcher(move |res| {
            if let Ok(event) = res {
                let _ = tx.try_send(event);
            }
        }) {
            Ok(w) => w,
            Err(e) => {
                tracing::warn!(error = %e, "turn observe: watcher start failed");
                return;
            }
        };
        if let Err(e) = watcher.watch(&root, RecursiveMode::Recursive) {
            tracing::warn!(error = %e, root = %root.display(), "turn observe: watch failed");
            return;
        }
        tracing::info!(session_id, turn_id, root = %root.display(), "turn observe: watch started");
        let task = tokio::spawn(pump(
            Arc::clone(self),
            session_id.to_string(),
            root.clone(),
            watcher,
            rx,
        ));
        entries.insert(
            session_id.to_string(),
            ObserverEntry {
                turn_id: turn_id.to_string(),
                root,
                task,
            },
        );
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

/// Coalesce raw events and fold each batch into the session's current turn.
/// Owns the watcher: aborting this task tears the watch down.
async fn pump(
    observers: Arc<TurnObservers>,
    session_id: String,
    root: PathBuf,
    _watcher: notify::RecommendedWatcher,
    mut rx: tokio::sync::mpsc::Receiver<notify::Event>,
) {
    let store_dir = observers.store_dir.clone();
    loop {
        let Some(first) = rx.recv().await else {
            return;
        };
        let mut batch: HashMap<String, &'static str> = HashMap::new();
        let mut born: HashSet<String> = HashSet::new();
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
