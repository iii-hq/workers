use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::{IIIClient, ObserveHandler, ObserveRejection, Observer};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

use crate::observe::{observer_key, EventObserver};

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

/// What changed: the daemon's `state.json` or the compose file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    State,
    File,
}

/// The event every `console::compose::changed` binding and every
/// `console::on-compose-changed` observer receives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChangedEvent {
    pub kind: ChangeKind,
    /// The compose file of the project.
    pub file: String,
    pub namespace: String,
    pub state_dir: String,
    /// The file that changed.
    pub path: String,
    /// Epoch milliseconds of the change.
    pub captured_at: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChangedTriggerSpec {}

#[derive(Debug, Clone)]
struct Binding {
    function_id: String,
    namespace: Option<String>,
}

/// One entry of the trigger type's registry: a `console::compose::changed`
/// binding or a `console::on-compose-changed` observer.
#[derive(Clone)]
enum Sink {
    Binding(Binding),
    Observer(Arc<dyn EventObserver>),
}

type Bindings = Arc<RwLock<HashMap<String, Sink>>>;

pub async fn compose_status(iii: &IIIClient, file: Option<&str>) -> Result<ComposeStatus, Error> {
    let value = iii
        .trigger(TriggerRequest {
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

#[derive(Clone)]
pub struct StateWatcher {
    iii: Arc<IIIClient>,
    bindings: Bindings,
    active: Arc<Mutex<Option<WatchHandle>>>,
}

impl StateWatcher {
    fn new(iii: Arc<IIIClient>, bindings: Bindings) -> Self {
        Self {
            iii,
            bindings,
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
        let watch = start_watch(self.iii.clone(), self.bindings.clone(), location.clone())
            .map_err(|error| Error::Handler(format!("compose watch failed: {error}")))?;
        *active = Some(watch);
        Ok(Some(location))
    }

    pub async fn close(&self) {
        self.active.lock().await.take();
    }
}

#[derive(Clone)]
pub struct ChangedTriggerHandler {
    bindings: Bindings,
    watcher: StateWatcher,
}

impl ChangedTriggerHandler {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        let bindings = Arc::new(RwLock::new(HashMap::new()));
        Self {
            watcher: StateWatcher::new(iii, bindings.clone()),
            bindings,
        }
    }

    pub fn watcher(&self) -> StateWatcher {
        self.watcher.clone()
    }

    /// The hooks of `console::on-compose-changed`: its observers join this
    /// trigger type's registry and watch.
    pub fn observe_handler(&self) -> OnChangedHandler {
        OnChangedHandler {
            bindings: self.bindings.clone(),
            watcher: self.watcher.clone(),
        }
    }
}

#[async_trait]
impl TriggerHandler for ChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        serde_json::from_value::<ChangedTriggerSpec>(config.config.clone()).map_err(|error| {
            Error::Handler(format!(
                "console::compose::changed config must be an empty object: {error}"
            ))
        })?;
        self.bindings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                config.id,
                Sink::Binding(Binding {
                    function_id: config.function_id,
                    namespace: config.namespace,
                }),
            );
        if self.watcher.ensure().await?.is_none() {
            tracing::warn!("compose daemon not reachable yet; watching starts on the next binding");
        }
        Ok(())
    }

    async fn unregister_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        self.bindings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&config.id);
        Ok(())
    }
}

/// Observers of `console::on-compose-changed`, filed in the registry the
/// `console::compose::changed` bindings live in: every change reaches both.
#[derive(Clone)]
pub struct OnChangedHandler {
    bindings: Bindings,
    watcher: StateWatcher,
}

impl OnChangedHandler {
    async fn add(&self, observer: Arc<dyn EventObserver>) -> Result<(), Error> {
        let id = observer.id().to_string();
        self.bindings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(observer_key(&id), Sink::Observer(observer));
        if self.watcher.ensure().await?.is_none() {
            tracing::warn!(
                "compose daemon not reachable yet; watching starts on the next observer"
            );
        }
        Ok(())
    }

    fn remove(&self, id: &str) {
        self.bindings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&observer_key(id));
    }
}

#[async_trait]
impl ObserveHandler for OnChangedHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format.
        let _: ChangedTriggerSpec = obs.args()?;
        self.add(Arc::new(obs.clone())).await?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), "console::on-compose-changed observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.remove(obs.id());
        tracing::info!(observer = %obs.id(), "console::on-compose-changed observer removed");
    }
}

fn start_watch(
    iii: Arc<IIIClient>,
    bindings: Bindings,
    location: ProjectLocation,
) -> notify::Result<WatchHandle> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })?;
    let state_dir = watch_dirs(&mut watcher, &location)?;

    let healthy = Arc::new(AtomicBool::new(true));
    let task_location = location.clone();
    let task_health = healthy.clone();
    let task = tokio::spawn(async move {
        run_watch(iii, bindings, task_location, state_dir, rx, task_health).await;
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
    bindings: Bindings,
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
                        tracing::warn!(error = %error, "compose filesystem watch dropped; it will re-arm on the next request or binding");
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
                        emit_change(&iii, &bindings, &location, kind, event.path);
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

fn emit_change(
    iii: &Arc<IIIClient>,
    bindings: &Bindings,
    location: &ProjectLocation,
    kind: ChangeKind,
    path: String,
) {
    let captured_at = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let payload = ChangedEvent {
        kind,
        file: location.file.to_string_lossy().into_owned(),
        namespace: location.namespace.clone(),
        state_dir: location.state_dir.to_string_lossy().into_owned(),
        path,
        captured_at,
    };
    let sinks: Vec<Sink> = bindings
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .cloned()
        .collect();
    let event = serde_json::to_value(payload).expect("ChangedEvent serializes");
    for sink in sinks {
        let iii = iii.clone();
        let event = event.clone();
        tokio::spawn(async move {
            match sink {
                Sink::Binding(binding) => {
                    let request = TriggerRequest {
                        function_id: binding.function_id.clone(),
                        payload: event,
                        action: None,
                        timeout_ms: Some(10_000),
                    };
                    let request: TriggerRequestWithMetadata = match binding.namespace {
                        Some(namespace) => request.namespace(namespace),
                        None => request.into(),
                    };
                    if let Err(error) = iii.trigger(request).await {
                        tracing::warn!(
                            function_id = %binding.function_id,
                            error = %error,
                            "compose change subscriber rejected an event"
                        );
                    }
                }
                Sink::Observer(observer) => {
                    if let Err(error) = observer.deliver(event).await {
                        tracing::warn!(
                            observer = %observer.id(),
                            error = %error,
                            "compose change observer rejected an event"
                        );
                    }
                }
            }
        });
    }
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

    /// A `console::on-compose-changed` observer that records what it is handed.
    struct Recorder {
        id: String,
        events: mpsc::UnboundedSender<serde_json::Value>,
    }

    #[async_trait]
    impl EventObserver for Recorder {
        fn id(&self) -> &str {
            &self.id
        }

        async fn deliver(&self, event: serde_json::Value) -> Result<(), String> {
            self.events.send(event).map_err(|error| error.to_string())
        }
    }

    fn offline_client() -> Arc<IIIClient> {
        // Never connects: the binding's trigger fails in its own task, the
        // observer's delivery is in-process.
        Arc::new(iii_sdk::register_worker(
            "ws://127.0.0.1:9",
            iii_sdk::InitOptions::default(),
        ))
    }

    #[tokio::test]
    async fn an_observer_receives_the_event_a_binding_receives() {
        let iii = offline_client();
        let (events, mut received) = mpsc::unbounded_channel();
        let bindings: Bindings = Arc::new(RwLock::new(HashMap::new()));
        bindings.write().unwrap().insert(
            "binding-1".to_string(),
            Sink::Binding(Binding {
                function_id: "iii::console::compose_changed::binding-1".to_string(),
                namespace: None,
            }),
        );
        bindings.write().unwrap().insert(
            observer_key("obs-1"),
            Sink::Observer(Arc::new(Recorder {
                id: "obs-1".to_string(),
                events,
            })),
        );

        emit_change(
            &iii,
            &bindings,
            &location(),
            ChangeKind::State,
            STATE_FILE.to_string(),
        );

        let event = tokio::time::timeout(Duration::from_secs(5), received.recv())
            .await
            .expect("the observer is handed the change")
            .expect("one event");
        let captured_at = event["captured_at"].as_u64().expect("captured_at");
        // The ChangedEvent a console::compose::changed binding is triggered with.
        assert_eq!(
            event,
            serde_json::to_value(ChangedEvent {
                kind: ChangeKind::State,
                file: "/proj/worker-compose.yaml".to_string(),
                namespace: "app".to_string(),
                state_dir: "/state/app".to_string(),
                path: STATE_FILE.to_string(),
                captured_at,
            })
            .unwrap()
        );
        assert!(received.try_recv().is_err(), "exactly one event");
    }

    #[tokio::test]
    async fn removing_an_observer_keeps_the_bindings() {
        let handler = ChangedTriggerHandler::new(offline_client());
        let (events, _received) = mpsc::unbounded_channel();
        handler.bindings.write().unwrap().insert(
            "binding-1".to_string(),
            Sink::Binding(Binding {
                function_id: "iii::console::compose_changed::binding-1".to_string(),
                namespace: None,
            }),
        );
        handler.bindings.write().unwrap().insert(
            observer_key("obs-1"),
            Sink::Observer(Arc::new(Recorder {
                id: "obs-1".to_string(),
                events,
            })),
        );

        handler.observe_handler().remove("obs-1");

        let bindings = handler.bindings.read().unwrap();
        assert!(!bindings.contains_key(&observer_key("obs-1")));
        assert!(bindings.contains_key("binding-1"));
    }
}
