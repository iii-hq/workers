//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors [`context-manager`](../../context-manager/src/configuration.rs) /
//! [`shell`](../../shell/src/configuration.rs) /
//! [`storage`](../../storage/src/configuration.rs).
//!
//! Every field hot-reloads:
//!
//! - The `adapter` block builds the [`SessionStore`](crate::store::SessionStore)
//!   and event sink. A change to it (fs <-> bridge, a new `data_dir`, a bridge
//!   url/timeout) rebuilds a fresh [`SessionRuntime`] and swaps it in under a
//!   write lock; a failed build keeps the previous runtime (last-good) and is
//!   recorded for `session::config-status`. After a successful swap the new
//!   store's state is replayed through the trigger fan-out (see
//!   [`resync`](crate::resync)) so subscribers stay real-time.
//! - The two list limits (`default_list_limit` / `max_list_limit`) are per-call
//!   tuning knobs. When only they change, the shared [`ConfigCell`] snapshot is
//!   swapped without rebuilding the runtime; `SessionService` reads it per
//!   `list` / `messages` call.
//!
//! Reloads are serialized by [`AppState::reload_lock`] and re-fetch the
//! authoritative value inside the lock, so overlapping `configuration:updated`
//! events converge to the latest config and a slow build from an older event
//! can never clobber a newer one.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use crate::config::WorkerConfig;
use crate::runtime::{build_runtime, SessionBuildContext, SessionRuntime};

/// Hot-swappable config snapshot shared with `SessionService`. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a
/// `read().await` and `clone()` the inner `Arc` out (a cheap refcount bump)
/// without holding the lock across its work, while a reload whole-snapshot
/// replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "session-manager";
const CONFIG_FN_ID: &str = "session::on-config-change";
const CONFIG_STATUS_FN_ID: &str = "session::config-status";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the attempt
/// number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Shared worker state the function handlers and the config-change trigger read.
/// `runtime` is the live, swappable storage + event plumbing; the rest is the
/// machinery to rebuild it on a config change.
#[derive(Clone)]
pub struct AppState {
    /// The live runtime. Handlers clone the `service` / `sink` out per call; the
    /// reload path swaps a freshly-built runtime in under the write lock.
    pub runtime: Arc<RwLock<SessionRuntime>>,
    /// Boot-time wiring `build_runtime` reuses on every rebuild (engine handle,
    /// subscriber sets, bridge registry, shared config snapshot).
    pub ctx: Arc<SessionBuildContext>,
    /// Serializes reloads: held across the authoritative fetch + build + swap so
    /// an older event's slow build can never clobber a newer applied config.
    pub reload_lock: Arc<Mutex<()>>,
    /// Last hot-reload outcome, exposed via `session::config-status`.
    pub reload_status: Arc<RwLock<ReloadStatus>>,
}

impl AppState {
    /// Wrap a freshly-built runtime + its build context into shared state.
    pub fn new(runtime: SessionRuntime, ctx: Arc<SessionBuildContext>) -> Self {
        Self {
            runtime: Arc::new(RwLock::new(runtime)),
            ctx,
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        }
    }
}

/// Outcome of the most recent hot-reload attempt, exposed via
/// `session::config-status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReloadOutcome {
    /// The live runtime reflects the most recent configuration the worker loaded.
    Applied,
    /// The most recent `configuration:updated` delivered a value the worker could
    /// NOT build (e.g. an unreadable `data_dir` or a self-referential bridge
    /// url); the previous runtime is still active and the central store has
    /// DIVERGED from what this worker is running.
    Rejected,
}

/// Operator-visible hot-reload status. `rejected_reloads > 0` (or
/// `last_outcome == Rejected`) means a stored config was refused and the worker
/// is running an older adapter than the central store — actionable divergence.
#[derive(Clone, Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ReloadStatus {
    pub last_outcome: ReloadOutcome,
    /// Build error from the most recent rejected reload (why it was refused).
    pub last_error: Option<String>,
    /// Cumulative count of rejected reloads since boot (never reset).
    pub rejected_reloads: u64,
}

impl Default for ReloadStatus {
    fn default() -> Self {
        // The initial runtime is built from a validated config at boot.
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

/// Register the `session-manager` configuration schema with the
/// configuration worker. When `seed` is present, its value is installed as
/// `initial_value`. Otherwise, the built-in default is seeded only when no
/// stored value exists yet (re-registration preserves the stored value, so
/// this is safe to call every boot).
pub async fn register_config(iii: &III, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Session Manager",
        "description": "Durable conversation store settings: the storage adapter (fs / bridge) \
                        and its config, plus the default and maximum page sizes for \
                        session::list / session::messages.",
        "schema": WorkerConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = WorkerConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `session-manager` configuration (env-expanded by the
/// configuration worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &III) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

async fn should_seed_default_value(iii: &III) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn get_config_value(iii: &III) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))
}

/// Returns `Ok(None)` when the entry does not exist. The engine's
/// missing-entry codes vary in case (`function_not_found`,
/// `STATEMENT_NOT_FOUND`, `NOT_FOUND`), so match case-insensitively.
async fn try_get_config_value(iii: &III) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Apply a freshly-fetched config to the live runtime.
///
/// When only the list limits changed (the `adapter` is identical), swaps the
/// shared snapshot the service reads per call — no store/sink rebuild. When the
/// adapter changed, builds a new [`SessionRuntime`] (a failed build returns
/// `Err` and leaves the running runtime untouched), swaps it in, then replays
/// the new store's state through the trigger fan-out so subscribers stay live;
/// the previous bridge connection, if any, is shut down afterward.
///
/// MUST only be called from [`reload_serialized`] (i.e. while holding
/// `reload_lock`): the build happens before the swap, so an unserialized caller
/// could let a slow older build clobber a newer applied config.
pub async fn apply_runtime(state: &AppState, cfg: WorkerConfig) -> Result<(), String> {
    let adapter_changed = {
        let rt = state.runtime.read().await;
        rt.config.boot_signature() != cfg.boot_signature()
    };

    if !adapter_changed {
        // List-limit-only change: swap the shared snapshot the live service
        // reads per call. No store/sink rebuild, no resync.
        let next = Arc::new(cfg);
        *state.ctx.config_cell.write().await = next.clone();
        state.runtime.write().await.config = next;
        tracing::info!("session-manager list limits reloaded (adapter unchanged)");
        return Ok(());
    }

    // Adapter changed: snapshot the pre-swap sessions (for delete signalling),
    // build the new runtime, swap it in, then replay current state through the
    // new local emitter so open views reconcile in place.
    let old_metas = {
        let store = state.runtime.read().await.store.clone();
        store.list_metas().await.unwrap_or_else(|e| {
            tracing::warn!(
                error = %e,
                "resync: listing pre-swap sessions failed; deletions will not be signalled"
            );
            Vec::new()
        })
    };

    let new_runtime = build_runtime(&cfg, &state.ctx)?;
    let new_store = new_runtime.store.clone();
    let new_emitter = new_runtime.local_emitter.clone();

    // Swap the shared config snapshot first so any concurrent list call sees
    // limits consistent with the new runtime, then swap the runtime itself.
    *state.ctx.config_cell.write().await = Arc::new(cfg);
    let old_runtime = {
        let mut guard = state.runtime.write().await;
        std::mem::replace(&mut *guard, new_runtime)
    };

    match crate::resync::resync_triggers(&old_metas, new_store.as_ref(), new_emitter.as_ref()).await
    {
        Ok(stats) => tracing::info!(
            sessions = stats.sessions,
            entries = stats.entries,
            deleted = stats.deleted,
            events = stats.events,
            "adapter hot-reloaded; replayed store state to subscribers"
        ),
        Err(e) => tracing::warn!(
            error = %e,
            "adapter hot-reloaded but trigger resync failed; subscribers may be stale until the next mutation"
        ),
    }

    // Retire the previous bridge connection (if any) after the resync so any
    // in-flight reads on it have drained.
    if let Some(remote) = old_runtime.bridge_remote.clone() {
        remote.shutdown_async().await;
    }
    drop(old_runtime);
    Ok(())
}

/// Run a config reload under the serialized reload lock. The `fetch` future is
/// awaited INSIDE the lock, so overlapping `configuration:updated` events are
/// applied one at a time and each observes the latest authoritative value.
async fn reload_serialized<F, Fut>(state: &AppState, fetch: F) -> Result<ReloadOutcome, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<WorkerConfig, String>>,
{
    let _reload = state.reload_lock.lock().await;
    let cfg = match fetch().await {
        Ok(cfg) => cfg,
        Err(e) => {
            // Transient fetch failure (e.g. configuration::get timed out): the
            // authoritative value is unknown. Keep the previous runtime AND
            // surface the error so the dispatcher can retry — never ack success.
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous runtime, signaling retry"
            );
            return Err(e);
        }
    };
    match apply_runtime(state, cfg).await {
        Ok(()) => {
            state.reload_status.write().await.record_applied();
            Ok(ReloadOutcome::Applied)
        }
        Err(e) => {
            // Config was fetched but is unbuildable (unreadable data_dir,
            // self-referential bridge url). Re-fetching returns the SAME value,
            // so ack + keep last-good to avoid a retry storm; the error log and
            // session::config-status surface the divergence.
            tracing::error!(
                error = %e,
                "rejected configuration change; keeping previous runtime (config could not be built)"
            );
            state.reload_status.write().await.record_rejected(e);
            Ok(ReloadOutcome::Rejected)
        }
    }
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. On `configuration:updated` the handler rebuilds and swaps the
/// runtime (or just the list limits) from the authoritative value.
pub fn register_config_trigger(iii: &III, state: AppState) -> Result<(), IIIError> {
    let st = state.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: Value| {
            let st = st.clone();
            async move {
                on_config_change(&st).await.map_err(IIIError::Handler)?;
                Ok::<Value, IIIError>(json!({ "ok": true }))
            }
        })
        .description(
            "Internal: hot-reload session-manager from the authoritative configuration when it \
             changes — rebuilds the storage adapter and event plumbing on an adapter change \
             (replaying current state to subscribers) and swaps the list limits otherwise.",
        ),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
    })?;
    Ok(())
}

/// Register `session::config-status`, reporting the last hot-reload outcome so
/// operators can detect when a stored config was rejected and the active
/// adapter diverged from the central store.
pub fn register_config_status(iii: &III, state: AppState) {
    let st = state.clone();
    iii.register_function(
        CONFIG_STATUS_FN_ID,
        // Ignore the payload: a no-arg call, and the engine-injected
        // `_caller_worker_id` would break a typed param.
        RegisterFunction::new_async(move |_payload: Value| {
            let st = st.clone();
            async move {
                let status = { st.reload_status.read().await.clone() };
                Ok::<Value, IIIError>(
                    serde_json::to_value(status).expect("ReloadStatus serializes"),
                )
            }
        })
        .description(
            "Report the last configuration hot-reload outcome: last_outcome (applied|rejected), \
             last_error, and rejected_reloads (count since boot). A rejected outcome or non-zero \
             count means a stored config was refused and the active storage adapter diverged from \
             the central store. Takes no arguments.",
        ),
    );
}

/// Reload from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `session::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller inject arbitrary config without
/// updating persisted state. Re-fetch the stored value via `configuration::get`
/// instead. The previous runtime is always kept on any failure path.
async fn on_config_change(state: &AppState) -> Result<(), String> {
    reload_serialized(state, || fetch_config(&state.ctx.iii))
        .await
        .map(|_| ())
}

async fn trigger_with_retry(iii: &III, function_id: &str, payload: Value) -> Result<Value, String> {
    let mut last_err = String::new();
    for attempt in 1..=CONFIG_RETRIES {
        match iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(CONFIG_TIMEOUT_MS),
            })
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
    use crate::config::{BridgeBackendConfig, FsBackendConfig, StorageAdapter};
    use crate::events::{BridgeSubscribers, TriggerSets};

    const TEST_URL: &str = "ws://127.0.0.1:59611";

    /// Build an `AppState` over an fs runtime rooted at `data_dir`. The engine
    /// handle never connects (no live engine in unit tests); these tests only
    /// exercise the build/swap mechanics, not delivery.
    fn fs_state(data_dir: &std::path::Path) -> AppState {
        let iii = Arc::new(iii_sdk::register_worker(
            TEST_URL,
            iii_sdk::InitOptions::default(),
        ));
        let cfg = fs_config(data_dir);
        let config_cell: ConfigCell = Arc::new(RwLock::new(Arc::new(cfg.clone())));
        let ctx = Arc::new(SessionBuildContext {
            iii,
            local_url: TEST_URL.to_string(),
            sets: TriggerSets::new(),
            bridge_subscribers: BridgeSubscribers::new(),
            config_cell,
        });
        let runtime = build_runtime(&cfg, &ctx).expect("build fs runtime");
        AppState::new(runtime, ctx)
    }

    fn fs_config(data_dir: &std::path::Path) -> WorkerConfig {
        WorkerConfig {
            adapter: StorageAdapter::Fs(FsBackendConfig {
                data_dir: data_dir.display().to_string(),
            }),
            ..WorkerConfig::default()
        }
    }

    #[tokio::test]
    async fn list_limit_only_change_keeps_store() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fs_state(tmp.path());
        let store_before = state.runtime.read().await.store.clone();

        let mut cfg = (*state.runtime.read().await.config).clone();
        cfg.default_list_limit += 7;
        apply_runtime(&state, cfg).await.expect("list-limit reload");

        let store_after = state.runtime.read().await.store.clone();
        assert!(
            Arc::ptr_eq(&store_before, &store_after),
            "store must NOT be rebuilt on a list-limit-only change"
        );
        assert_eq!(
            state.ctx.config_cell.read().await.default_list_limit,
            WorkerConfig::default().default_list_limit + 7,
            "the shared snapshot the service reads must reflect the new limit"
        );
    }

    #[tokio::test]
    async fn adapter_change_rebuilds_store() {
        let tmp1 = tempfile::tempdir().unwrap();
        let tmp2 = tempfile::tempdir().unwrap();
        let state = fs_state(tmp1.path());
        let store_before = state.runtime.read().await.store.clone();

        apply_runtime(&state, fs_config(tmp2.path()))
            .await
            .expect("adapter reload");

        let store_after = state.runtime.read().await.store.clone();
        assert!(
            !Arc::ptr_eq(&store_before, &store_after),
            "store must be rebuilt on a data_dir change"
        );
    }

    #[tokio::test]
    async fn rejected_reload_keeps_previous_runtime() {
        let tmp = tempfile::tempdir().unwrap();
        let state = fs_state(tmp.path());
        let store_before = state.runtime.read().await.store.clone();

        // A bridge whose url equals the local engine would defer to itself, so
        // build_runtime rejects it — the previous runtime must survive.
        let bad = WorkerConfig {
            adapter: StorageAdapter::Bridge(BridgeBackendConfig {
                url: state.ctx.local_url.clone(),
                timeout_ms: 5000,
            }),
            ..WorkerConfig::default()
        };
        let outcome = reload_serialized(&state, || async move { Ok::<_, String>(bad) })
            .await
            .expect("reload_serialized acks even when the config is rejected");
        assert_eq!(outcome, ReloadOutcome::Rejected);

        let store_after = state.runtime.read().await.store.clone();
        assert!(
            Arc::ptr_eq(&store_before, &store_after),
            "runtime must be kept on a rejected reload"
        );
        let status = state.reload_status.read().await;
        assert_eq!(status.last_outcome, ReloadOutcome::Rejected);
        assert_eq!(status.rejected_reloads, 1);
        assert!(status.last_error.is_some());
    }
}
