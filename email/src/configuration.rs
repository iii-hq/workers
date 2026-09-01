use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};
use tokio::task::JoinHandle;

use crate::config::{Provider, WorkerConfig};
use crate::provider::imap::ImapPool;
use crate::triggers::dispatcher::EventDispatcher;

pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const DEFAULT_CONFIG_ID: &str = "email";

pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.to_string())
    })
    .as_str()
}

const CONFIG_FN_ID: &str = "email::on-config-change";
const CONFIG_STATUS_FN_ID: &str = "email::config-status";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

#[derive(Clone)]
pub struct AppState {
    pub cell: ConfigCell,
    pub pool: Arc<ImapPool>,
    pub dispatcher: Arc<dyn EventDispatcher>,
    pub idle: Arc<Mutex<Vec<JoinHandle<()>>>>,
    pub reload_lock: Arc<Mutex<()>>,
    pub reload_status: Arc<RwLock<ReloadStatus>>,
}

impl AppState {
    pub fn new(cfg: WorkerConfig, dispatcher: Arc<dyn EventDispatcher>) -> Self {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg)));
        Self {
            pool: Arc::new(ImapPool::new(cell.clone())),
            cell,
            dispatcher,
            idle: Arc::new(Mutex::new(Vec::new())),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        }
    }

    pub async fn snapshot(&self) -> Arc<WorkerConfig> {
        self.cell.read().await.clone()
    }

    pub async fn start_idle(&self) -> usize {
        let handles = spawn_idle(&self.snapshot().await, &self.dispatcher);
        let count = handles.len();
        *self.idle.lock().await = handles;
        count
    }

    pub async fn stop_idle(&self) {
        for handle in self.idle.lock().await.drain(..) {
            handle.abort();
        }
    }
}

pub fn spawn_idle(
    cfg: &Arc<WorkerConfig>,
    dispatcher: &Arc<dyn EventDispatcher>,
) -> Vec<JoinHandle<()>> {
    let mut handles = Vec::new();
    for (name, account) in cfg.accounts.iter() {
        if account.provider != Provider::Imap {
            continue;
        }
        let Some(imap) = account.imap.as_ref() else {
            tracing::warn!(account = %name, "provider=imap but imap config missing, skipping");
            continue;
        };
        for folder in imap.folders.iter().cloned() {
            let name = name.clone();
            let cfg = cfg.clone();
            let dispatcher = dispatcher.clone();
            handles.push(tokio::spawn(async move {
                crate::provider::imap::connection::run_until_shutdown(name, folder, cfg, dispatcher)
                    .await
            }));
        }
    }
    handles
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReloadOutcome {
    Applied,
    Rejected,
}

#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ReloadStatus {
    pub last_outcome: ReloadOutcome,
    pub last_error: Option<String>,
    pub rejected_reloads: u64,
}

impl Default for ReloadStatus {
    fn default() -> Self {
        Self {
            last_outcome: ReloadOutcome::Applied,
            last_error: None,
            rejected_reloads: 0,
        }
    }
}

impl ReloadStatus {
    fn record_applied(&mut self) {
        self.last_outcome = ReloadOutcome::Applied;
        self.last_error = None;
    }

    fn record_rejected(&mut self, err: String) {
        self.last_outcome = ReloadOutcome::Rejected;
        self.last_error = Some(err);
        self.rejected_reloads = self.rejected_reloads.saturating_add(1);
    }
}

pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Email",
        "description": "Email worker settings: the SMTP / IMAP accounts (host, port, TLS, \
                        sender, optional login) and the send / attachment / recipient limits.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
    cfg.validate()?;
    let structural = state.snapshot().await.boot_signature() != cfg.boot_signature();
    let next = Arc::new(cfg);
    if !structural {
        *state.cell.write().await = next;
        tracing::info!("email limits reloaded (accounts unchanged)");
        return Ok(());
    }
    let handles = spawn_idle(&next, &state.dispatcher);
    let supervised = handles.len();
    *state.cell.write().await = next;
    state.pool.reset();
    let previous = std::mem::replace(&mut *state.idle.lock().await, handles);
    for handle in previous {
        handle.abort();
    }
    tracing::info!(
        imap_connections = supervised,
        "email accounts reloaded; IMAP supervisors respawned and pooled sessions dropped"
    );
    Ok(())
}

async fn reload_serialized<F, Fut>(state: &AppState, fetch: F) -> Result<ReloadOutcome, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<WorkerConfig, String>>,
{
    let _reload = state.reload_lock.lock().await;
    let cfg = match fetch().await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous runtime, signaling retry"
            );
            return Err(e);
        }
    };
    match apply_config(state, cfg).await {
        Ok(()) => {
            state.reload_status.write().await.record_applied();
            Ok(ReloadOutcome::Applied)
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                "rejected configuration change; keeping previous runtime (config could not be applied)"
            );
            state.reload_status.write().await.record_rejected(e);
            Ok(ReloadOutcome::Rejected)
        }
    }
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigStatusRequest {}

pub fn register_config_trigger(iii: &Arc<IIIClient>, state: AppState) -> Result<(), Error> {
    let iii_inner = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let state = state.clone();
            let iii = iii_inner.clone();
            async move {
                reload_serialized(&state, || fetch_config(&iii))
                    .await
                    .map_err(Error::Handler)?;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the email worker from the authoritative configuration when it \
             changes. Limit changes swap the snapshot; account changes respawn the IMAP \
             supervisors and drop pooled sessions. Rejected values keep the previous runtime.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_FN_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
    ))?;
    Ok(())
}

pub fn register_config_status(iii: &IIIClient, state: AppState) {
    iii.register_function(
        CONFIG_STATUS_FN_ID,
        RegisterFunction::new_async(move |_req: ConfigStatusRequest| {
            let state = state.clone();
            async move {
                let status = { state.reload_status.read().await.clone() };
                Ok::<ReloadStatus, Error>(status)
            }
        })
        .description(
            "Report the last configuration hot-reload outcome: last_outcome (applied|rejected), \
             last_error, and rejected_reloads (count since boot). A rejected outcome means a \
             stored config was refused and the live accounts diverged from the central store. \
             Takes no arguments.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}

async fn trigger_configuration_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(
                TriggerRequest {
                    function_id: function_id.to_string(),
                    payload: payload.clone(),
                    action: None,
                    timeout_ms: Some(CONFIG_TIMEOUT_MS),
                }
                .namespace("default"),
            )
            .await
        {
            Ok(v) => return Ok(v),
            Err(e) => {
                last_err = e.to_string();
                if attempt < CONFIG_RETRIES {
                    tracing::warn!(
                        function_id,
                        attempt,
                        error = %last_err,
                        "configuration RPC failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * u64::from(attempt),
                    ))
                    .await;
                }
            }
        }
    }
    Err(format!(
        "{function_id} failed after {CONFIG_RETRIES} attempts: {last_err}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::triggers::Event;
    use async_trait::async_trait;

    struct NoopDispatcher;

    #[async_trait]
    impl EventDispatcher for NoopDispatcher {
        async fn dispatch(&self, _event: Event) {}
    }

    fn state_with(yaml: &str) -> AppState {
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        AppState::new(cfg, Arc::new(NoopDispatcher))
    }

    const ONE_ACCOUNT: &str = r#"
accounts:
  notifications:
    provider: smtp
    from: "ReachAI <noreply@example.com>"
    smtp:
      host: smtp.example.com
      port: 587
"#;

    #[tokio::test]
    async fn tuning_change_swaps_the_snapshot_only() {
        let state = state_with(ONE_ACCOUNT);
        let mut next = (*state.snapshot().await).clone();
        next.limits.max_recipients = 3;
        apply_config(&state, next).await.unwrap();
        assert_eq!(state.snapshot().await.limits.max_recipients, 3);
        assert!(state.idle.lock().await.is_empty());
    }

    #[tokio::test]
    async fn account_change_replaces_the_snapshot_and_spawns_only_imap_supervisors() {
        let state = state_with(ONE_ACCOUNT);
        let mut next = (*state.snapshot().await).clone();
        next.accounts.get_mut("notifications").unwrap().from =
            "Other <other@example.com>".to_string();
        apply_config(&state, next).await.unwrap();
        assert_eq!(
            state.snapshot().await.accounts["notifications"].from,
            "Other <other@example.com>"
        );
        assert!(state.idle.lock().await.is_empty());
    }

    #[tokio::test]
    async fn invalid_config_is_rejected_and_the_previous_snapshot_survives() {
        let state = state_with(ONE_ACCOUNT);
        let mut next = (*state.snapshot().await).clone();
        next.limits.max_recipients = 0;
        let outcome = reload_serialized(&state, || async { Ok(next) })
            .await
            .unwrap();
        assert_eq!(outcome, ReloadOutcome::Rejected);
        assert_eq!(state.snapshot().await.limits.max_recipients, 100);
        let status = state.reload_status.read().await.clone();
        assert_eq!(status.rejected_reloads, 1);
        assert!(status.last_error.unwrap().contains("max_recipients"));
    }

    #[tokio::test]
    async fn fetch_failure_keeps_the_runtime_and_reports_retry() {
        let state = state_with(ONE_ACCOUNT);
        let err = reload_serialized(&state, || async { Err("offline".to_string()) })
            .await
            .unwrap_err();
        assert_eq!(err, "offline");
        assert_eq!(state.reload_status.read().await.rejected_reloads, 0);
    }
}
