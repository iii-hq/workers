//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors [`ide`](../../ide/src/configuration.rs) /
//! [`database`](../../database/src/configuration.rs) /
//! [`coder`](../../coder/src/configuration.rs).
//!
//! Every field hot-reloads — nothing requires a restart:
//!
//! - `summarizer_timeout_ms` and the other numeric/threshold knobs are
//!   read from the live snapshot per call (the `RouterSummarizer` reads
//!   its timeout from the [`ConfigCell`]); a change takes effect on the
//!   next call once the snapshot is swapped.
//! - `lease_dir` is the one STRUCTURAL field (the boot signature): a
//!   change rebuilds the `FsLeaseStore` and swaps it into the shared
//!   [`LeaseCell`](crate::ports::LeaseCell) on the fly. A rebuild that
//!   fails (an unopenable dir) keeps the previous store + config
//!   (last-good); it never refuses with "restart required".

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::adapters::fs_lease::FsLeaseStore;
use crate::config::WorkerConfig;
use crate::ports::{LeaseCell, LeaseStore};

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a
/// `read().await` and `clone()` the inner `Arc` out (a cheap refcount
/// bump) without holding the lock across its work, while `apply_config`
/// whole-snapshot replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const DEFAULT_CONFIG_ID: &str = "context-manager";

/// The configuration entry this worker owns.
///
/// `III_CONFIG_NAME` when a supervisor set it, else the built-in name. A worker
/// that hardcodes its id turns that id into a global scarce name: two instances
/// share one entry and take turns overwriting it, and each write wakes both.
/// Being told which entry is its own is what lets them differ.
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
const CONFIG_FN_ID: &str = "context::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the
/// attempt number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the schema. The optional seed or built-in default is installed
/// only when no stored value exists; live configuration always takes precedence.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Context Manager",
        "description": "Model-ready context assembly tuning: token-budget reserves, \
                        function-result prune thresholds, compaction tail size, the \
                        compaction lease TTL and directory, and the summariser timeout.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // A seed initializes an absent entry; it never replaces a Compose override.
    if should_seed_default_value(iii).await? {
        payload["initial_value"] = seed
            .map(|value| value.to_json())
            .unwrap_or_else(|| WorkerConfig::default().to_json());
    }
    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `context-manager` configuration (env-expanded by the
/// configuration worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json(&value)
}

/// `true` for the one error that is a definitive answer rather than a
/// failure: the configuration worker's uppercase `NOT_FOUND` entry code.
/// Matched case-SENSITIVELY — the engine's missing-function code is the
/// lowercase `function_not_found` and a backend lookup failure is
/// `statement_not_found`; those must propagate as errors instead of being
/// read as "nothing stored yet", which would seed a default over a stored
/// operator/override value.
fn is_not_found(error: &str) -> bool {
    match error.split_once("remote error (") {
        Some((_, rest)) => rest
            .split_once(')')
            .is_some_and(|(code, _)| code == "NOT_FOUND"),
        None => error.trim() == "NOT_FOUND",
    }
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

/// Returns `Ok(None)` when the entry does not exist. The engine's
/// missing-entry code is the uppercase `NOT_FOUND`; match it case-SENSITIVELY so a
/// service/transport failure (`function_not_found`/`statement_not_found`)
/// propagates as an error instead of seeding over a stored value.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Swap the config snapshot under the write lock. Tuning knobs take effect
/// on the next per-call read; any structural rebuild (the lease store) is
/// done by the caller before this swap.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Internal `context::on-config-change` trigger payload. The handler
/// re-fetches the authoritative configuration, so this carries only the
/// (advisory) configuration id; a struct (not `Value`) keeps the request
/// schema concrete and unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `context::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. `leases` is the live lease-store cell the handler rebuilds and
/// swaps when `lease_dir` changes; every other field is read per call, so
/// nothing requires a restart.
pub fn register_config_trigger(
    iii: &IIIClient,
    cell: ConfigCell,
    leases: LeaseCell,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let leases_for_fn = leases.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let leases = leases_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &leases).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload context-manager from the authoritative configuration when it \
             changes — rebuilds the compaction lease store on a lease_dir change and swaps the \
             per-call tuning snapshot otherwise.",
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

/// Reload from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `context::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller inject arbitrary config without
/// updating persisted state. Re-fetch the stored value via
/// `configuration::get` instead.
///
/// When `lease_dir` changed, rebuild the `FsLeaseStore` and swap it in
/// before swapping the snapshot. A rebuild failure (an unopenable dir)
/// keeps the previous store AND config (last-good) so the live snapshot's
/// `lease_dir` never diverges from the store actually in use.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, leases: &LeaseCell) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous config"
            );
            return;
        }
    };

    let lease_dir_changed = cell.read().await.boot_signature() != cfg.boot_signature();
    if lease_dir_changed {
        match FsLeaseStore::new(cfg.resolved_lease_dir()) {
            Ok(store) => {
                let store: Arc<dyn LeaseStore> = Arc::new(store);
                *leases.write().await = store;
                tracing::info!("context-manager lease store rebuilt (lease_dir changed)");
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "config-change: rebuilding the lease store failed; keeping the previous store and config (last-good)"
                );
                return;
            }
        }
    }

    apply_config(cell, cfg).await;
    tracing::info!("context-manager configuration reloaded");
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
    /// A context-service lookup failure must never authorize installing defaults.
    #[test]
    fn missing_entry_detection_does_not_mask_service_failures() {
        assert!(super::is_not_found("NOT_FOUND"));
        assert!(super::is_not_found(
            "remote error (NOT_FOUND): configuration not found"
        ));
        assert!(super::is_not_found(
            "configuration::get failed after 3 attempts: remote error (NOT_FOUND): missing"
        ));
        assert!(!super::is_not_found("function_not_found"));
        assert!(!super::is_not_found("statement_not_found"));
        assert!(!super::is_not_found("RESOURCE_NOT_FOUND"));
        assert!(!super::is_not_found(
            "remote error (ADAPTER_ERROR): NOT_FOUND"
        ));
        assert!(!super::is_not_found(
            "remote error (OTHER): remote error (NOT_FOUND): nested"
        ));
        assert!(!super::is_not_found(
            "configuration::get failed after 3 attempts: function_not_found"
        ));
    }

    use super::*;

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(
            cell.read().await.tail_turns,
            WorkerConfig::default().tail_turns
        );

        let tuned = WorkerConfig {
            tail_turns: 9,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.tail_turns, 9);
    }

    #[tokio::test]
    async fn lease_dir_change_swaps_the_store() {
        // The structural reload path swaps the live lease store in place
        // (a `lease_dir` change rebuilds the FsLeaseStore) rather than
        // refusing — no restart.
        let d1 = tempfile::tempdir().unwrap();
        let d2 = tempfile::tempdir().unwrap();
        let leases = crate::ports::lease_cell(Arc::new(FsLeaseStore::new(d1.path()).unwrap()));
        let before = leases.read().await.clone();

        let next: Arc<dyn LeaseStore> = Arc::new(FsLeaseStore::new(d2.path()).unwrap());
        *leases.write().await = next;

        let after = leases.read().await.clone();
        assert!(
            !Arc::ptr_eq(&before, &after),
            "lease store must be swapped on a lease_dir change"
        );
    }
}
