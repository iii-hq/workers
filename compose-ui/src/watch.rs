use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::protocol::{TriggerRequest, TriggerRequestWithMetadata};
use iii_sdk::trigger::{TriggerConfig, TriggerHandler};
use iii_sdk::IIIClient;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, Mutex};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    State,
    File,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ChangedEvent {
    pub kind: ChangeKind,
    pub file: String,
    pub namespace: String,
    pub state_dir: String,
    pub path: String,
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

type Bindings = Arc<RwLock<HashMap<String, Binding>>>;

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
}

#[async_trait]
impl TriggerHandler for ChangedTriggerHandler {
    async fn register_trigger(&self, config: TriggerConfig) -> Result<(), Error> {
        serde_json::from_value::<ChangedTriggerSpec>(config.config.clone()).map_err(|error| {
            Error::Handler(format!(
                "compose-ui::changed config must be an empty object: {error}"
            ))
        })?;
        self.bindings
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                config.id,
                Binding {
                    function_id: config.function_id,
                    namespace: config.namespace,
                },
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

fn start_watch(
    iii: Arc<IIIClient>,
    bindings: Bindings,
    location: ProjectLocation,
) -> notify::Result<WatchHandle> {
    let (tx, rx) = mpsc::unbounded_channel();
    let mut watcher = notify::recommended_watcher(move |event| {
        let _ = tx.send(event);
    })?;
    watcher.watch(&location.state_dir, RecursiveMode::NonRecursive)?;
    let compose_dir = location.file.parent().unwrap_or_else(|| Path::new("."));
    watcher.watch(compose_dir, RecursiveMode::NonRecursive)?;

    let healthy = Arc::new(AtomicBool::new(true));
    let task_location = location.clone();
    let task_health = healthy.clone();
    let task = tokio::spawn(async move {
        run_watch(iii, bindings, task_location, rx, task_health).await;
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

#[derive(Debug)]
struct PendingEvent {
    path: String,
    due: tokio::time::Instant,
}

async fn run_watch(
    iii: Arc<IIIClient>,
    bindings: Bindings,
    location: ProjectLocation,
    mut rx: mpsc::UnboundedReceiver<notify::Result<Event>>,
    healthy: Arc<AtomicBool>,
) {
    let mut pending: HashMap<ChangeKind, PendingEvent> = HashMap::new();
    let mut tick = tokio::time::interval(Duration::from_millis(25));
    loop {
        tokio::select! {
            event = rx.recv() => {
                let Some(event) = event else { break };
                match event {
                    Ok(event) => {
                        for (kind, path) in changes_for_event(&location, &event) {
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
            _ = tick.tick() => {
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

fn changes_for_event(location: &ProjectLocation, event: &Event) -> Vec<(ChangeKind, String)> {
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
        if path.parent() == Some(location.state_dir.as_path())
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
    let bindings: Vec<Binding> = bindings
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .values()
        .cloned()
        .collect();
    for binding in bindings {
        let iii = iii.clone();
        let payload = payload.clone();
        tokio::spawn(async move {
            let request = TriggerRequest {
                function_id: binding.function_id.clone(),
                payload: serde_json::to_value(payload).expect("ChangedEvent serializes"),
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
        assert_eq!(
            changes_for_event(&location(), &event),
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
        assert!(changes_for_event(&location(), &event).is_empty());
    }
}
