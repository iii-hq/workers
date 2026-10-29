//! The compose project watch behind `console::compose::on-changed`: one
//! filesystem watch of the daemon's state dir and the compose file's dir,
//! started with the function's first observer and closed with its last.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::{
    AddedHook, CallAction, CallRequest, IIIClient, ObserveRejection, Observer, RemovedHook,
};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

use super::ON_CHANGED_FN;

pub const STATE_FILE: &str = "state.json";
const COALESCE: Duration = Duration::from_millis(200);

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusContainer {
    pub container: String,
    pub pid: Option<u32>,
    pub state: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct ComposeStatus {
    pub file: Option<String>,
    pub namespace: Option<String>,
    pub state_dir: Option<String>,
    pub daemon_pid: Option<u32>,
    #[serde(default)]
    pub containers: Vec<StatusContainer>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectLocation {
    pub file: PathBuf,
    pub namespace: String,
    pub state_dir: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    State,
    File,
}

/// The event `console::compose::on-changed` is called with: what changed
/// (`state`: the daemon wrote state.json; `file`: the compose file), in
/// which project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedEvent {
    pub kind: ChangeKind,
    pub file: String,
    pub namespace: String,
    pub state_dir: String,
    pub path: String,
    pub captured_at: u64,
}

pub async fn compose_status(iii: &IIIClient, file: Option<&str>) -> Result<ComposeStatus, Error> {
    let value = iii
        .call(CallRequest {
            function_id: "compose::status".to_string(),
            payload: file
                .map(|file| serde_json::json!({ "file": file }))
                .unwrap_or_else(|| serde_json::json!({})),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await?;
    serde_json::from_value(value)
        .map_err(|error| Error::Handler(format!("invalid compose::status response: {error}")))
}

pub async fn locate(iii: &IIIClient, file: Option<&str>) -> Result<Option<ProjectLocation>, Error> {
    let status = compose_status(iii, file).await?;
    let (Some(file), Some(namespace), Some(state_dir)) =
        (status.file, status.namespace, status.state_dir)
    else {
        return Ok(None);
    };
    Ok(Some(ProjectLocation {
        file: PathBuf::from(file),
        namespace,
        state_dir: PathBuf::from(state_dir),
    }))
}

struct WatchHandle {
    location: ProjectLocation,
    healthy: Arc<AtomicBool>,
    _watcher: RecommendedWatcher,
    task: tokio::task::JoinHandle<()>,
}

impl Drop for WatchHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// The watch `console::compose::on-changed` runs while it is observed: the
/// added hook starts it with the first observer (and re-arms one that
/// dropped), the removed hook closes it with the last.
#[derive(Clone)]
pub struct StateWatcher {
    iii: Arc<IIIClient>,
    observers: Arc<std::sync::Mutex<HashSet<String>>>,
    active: Arc<Mutex<Option<WatchHandle>>>,
}

impl StateWatcher {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self {
            iii,
            observers: Arc::default(),
            active: Arc::new(Mutex::new(None)),
        }
    }

    pub async fn ensure(&self) -> Result<Option<ProjectLocation>, Error> {
        let mut active = self.active.lock().await;
        if let Some(watch) = active.as_ref() {
            if watch.healthy.load(Ordering::Acquire) {
                return Ok(Some(watch.location.clone()));
            }
            active.take();
        }
        let Some(location) = locate(&self.iii, None).await? else {
            return Ok(None);
        };
        let watch = start_watch(self.iii.clone(), location.clone())
            .map_err(|error| Error::Handler(format!("compose watch failed: {error}")))?;
        *active = Some(watch);
        Ok(Some(location))
    }

    pub async fn close(&self) {
        self.active.lock().await.take();
    }

    fn observers(&self) -> std::sync::MutexGuard<'_, HashSet<String>> {
        self.observers
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

#[async_trait]
impl AddedHook for StateWatcher {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        if self.ensure().await?.is_none() {
            tracing::warn!(
                "compose daemon not reachable yet; watching starts with the next observer"
            );
        }
        self.observers().insert(obs.id().to_string());
        Ok(())
    }
}

#[async_trait]
impl RemovedHook for StateWatcher {
    async fn on_removed(&self, obs: Observer) {
        let last = {
            let mut observers = self.observers();
            observers.remove(obs.id());
            observers.is_empty()
        };
        if last {
            self.close().await;
            tracing::info!("compose state watch closed with its last observer");
        }
    }
}

fn start_watch(iii: Arc<IIIClient>, location: ProjectLocation) -> notify::Result<WatchHandle> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })?;
    let state_dir = watch_dirs(&mut watcher, &location)?;

    let healthy = Arc::new(AtomicBool::new(true));
    let task_location = location.clone();
    let task_health = healthy.clone();
    let task = tokio::spawn(async move {
        run_watch(iii, task_location, state_dir, rx, task_health).await;
    });
    tracing::info!(
        state_dir = %location.state_dir.display(),
        file = %location.file.display(),
        "compose state watch armed"
    );
    Ok(WatchHandle {
        location,
        healthy,
        _watcher: watcher,
        task,
    })
}

/// Watches the state dir and the compose file's dir; returns the state dir
/// in the form events name it.
fn watch_dirs(watcher: &mut impl Watcher, location: &ProjectLocation) -> notify::Result<PathBuf> {
    // FSEvents reports resolved paths (`/var/..` arrives as `/private/var/..`)
    // and `III_COMPOSE_STATE_DIR` reaches us unresolved, so state.json is
    // matched against the resolved dir; watching it makes inotify agree.
    let state_dir = location
        .state_dir
        .canonicalize()
        .unwrap_or_else(|_| location.state_dir.clone());
    watcher.watch(&state_dir, RecursiveMode::NonRecursive)?;
    let compose_dir = location.file.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(compose_dir, RecursiveMode::NonRecursive)?;
    Ok(state_dir)
}

#[derive(Debug)]
struct PendingEvent {
    path: String,
    due: tokio::time::Instant,
}

async fn run_watch(
    iii: Arc<IIIClient>,
    location: ProjectLocation,
    state_dir: PathBuf,
    mut rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    healthy: Arc<AtomicBool>,
) {
    let mut pending: HashMap<ChangeKind, PendingEvent> = HashMap::new();
    loop {
        // Sleep until the earliest coalesced change is due; with nothing
        // pending, wait for the filesystem alone (no idle wake-ups).
        let next_due = pending.values().map(|event| event.due).min();
        let due_timer = async move {
            match next_due {
                Some(due) => tokio::time::sleep_until(due).await,
                None => std::future::pending::<()>().await,
            }
        };
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else { break };
                match event {
                    Ok(event) => {
                        for (kind, path) in changes_for_event(&location, &state_dir, &event) {
                            pending.insert(kind, PendingEvent {
                                path,
                                due: tokio::time::Instant::now() + COALESCE,
                            });
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "compose filesystem watch dropped; it re-arms with the next observer");
                        healthy.store(false, Ordering::Release);
                        break;
                    }
                }
            }
            _ = due_timer => {
                let now = tokio::time::Instant::now();
                let due: Vec<ChangeKind> = pending
                    .iter()
                    .filter_map(|(kind, event)| (event.due <= now).then_some(*kind))
                    .collect();
                for kind in due {
                    if let Some(event) = pending.remove(&kind) {
                        emit_change(&iii, &location, kind, event.path);
                    }
                }
            }
        }
    }
    healthy.store(false, Ordering::Release);
}

fn changes_for_event(
    location: &ProjectLocation,
    state_dir: &Path,
    event: &Event,
) -> Vec<(ChangeKind, String)> {
    if matches!(event.kind, EventKind::Access(_)) {
        return Vec::new();
    }
    let mut changes = Vec::new();
    for path in &event.paths {
        let compose_parent = location.file.parent().unwrap_or_else(|| Path::new("."));
        if path.parent() == Some(compose_parent) && path.file_name() == location.file.file_name() {
            changes.push((
                ChangeKind::File,
                path.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            ));
        }
        if path.parent() == Some(state_dir)
            && path.file_name().and_then(|name| name.to_str()) == Some(STATE_FILE)
        {
            changes.push((ChangeKind::State, STATE_FILE.to_string()));
        }
    }
    changes.sort_by_key(|(kind, _)| match kind {
        ChangeKind::State => 0,
        ChangeKind::File => 1,
    });
    changes.dedup();
    changes
}

/// Call `console::compose::on-changed` once with the change, when anyone
/// observes it.
fn emit_change(iii: &Arc<IIIClient>, location: &ProjectLocation, kind: ChangeKind, path: String) {
    if !iii.is_observed(ON_CHANGED_FN) {
        return;
    }
    let captured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let event = ChangedEvent {
        kind,
        file: location.file.to_string_lossy().into_owned(),
        namespace: location.namespace.clone(),
        state_dir: location.state_dir.to_string_lossy().into_owned(),
        path,
        captured_at,
    };
    let iii = iii.clone();
    tokio::spawn(async move {
        let payload = serde_json::to_value(event).expect("ChangedEvent serializes");
        let request = CallRequest {
            function_id: ON_CHANGED_FN.to_string(),
            payload,
            action: Some(CallAction::Void),
            timeout_ms: None,
        };
        if let Err(error) = iii.call(request).await {
            tracing::warn!(error = %error, "{ON_CHANGED_FN} call failed");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::ModifyKind;

    fn location() -> ProjectLocation {
        ProjectLocation {
            file: PathBuf::from("/proj/worker-compose.yaml"),
            namespace: "app".to_string(),
            state_dir: PathBuf::from("/state/app"),
        }
    }

    #[test]
    fn classifies_only_the_compose_file_and_state_json() {
        let event = Event {
            kind: EventKind::Modify(ModifyKind::Any),
            paths: vec![
                PathBuf::from("/state/app/state.json"),
                PathBuf::from("/state/app/engine.log"),
                PathBuf::from("/proj/worker-compose.yaml"),
                PathBuf::from("/proj/README.md"),
            ],
            attrs: Default::default(),
        };
        let location = location();
        assert_eq!(
            changes_for_event(&location, &location.state_dir, &event),
            [
                (ChangeKind::State, "state.json".to_string()),
                (ChangeKind::File, "worker-compose.yaml".to_string()),
            ]
        );
    }

    #[test]
    fn ignores_access_events() {
        let event = Event {
            kind: EventKind::Access(notify::event::AccessKind::Any),
            paths: vec![PathBuf::from("/state/app/state.json")],
            attrs: Default::default(),
        };
        let location = location();
        assert!(changes_for_event(&location, &location.state_dir, &event).is_empty());
    }

    /// iii-compose uses `III_COMPOSE_STATE_DIR` as given, so the state dir can
    /// sit behind a symlink (macOS `/var`, a linked home): FSEvents reports the
    /// resolved path.
    #[cfg(unix)]
    #[test]
    fn state_dir_behind_a_symlink_reports_state_changes() {
        let tmp = tempfile::tempdir().unwrap();
        let real = tmp.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let location = ProjectLocation {
            file: tmp
                .path()
                .canonicalize()
                .unwrap()
                .join("worker-compose.yaml"),
            namespace: "app".to_string(),
            state_dir: link.clone(),
        };
        let (tx, rx) = std::sync::mpsc::channel();
        let mut watcher = notify::recommended_watcher(tx).unwrap();
        let state_dir = watch_dirs(&mut watcher, &location).unwrap();
        let state_change = (ChangeKind::State, STATE_FILE.to_string());

        std::fs::write(link.join(STATE_FILE), "{}").unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let event = rx
                .recv_timeout(deadline.saturating_duration_since(std::time::Instant::now()))
                .expect("state.json written through the link")
                .unwrap();
            if changes_for_event(&location, &state_dir, &event).contains(&state_change) {
                break;
            }
        }

        // The FSEvents form, checked on every platform.
        let resolved = Event {
            kind: EventKind::Modify(ModifyKind::Any),
            paths: vec![real.canonicalize().unwrap().join(STATE_FILE)],
            attrs: Default::default(),
        };
        assert_eq!(
            changes_for_event(&location, &state_dir, &resolved),
            [state_change]
        );
    }
}
