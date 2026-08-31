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
