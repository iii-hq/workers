//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors [`shell`](../../shell/src/configuration.rs) /
//! [`database`](../../database/src/configuration.rs) /
//! [`coder`](../../coder/src/configuration.rs).
//!
//! context-manager's config splits into two halves on a live update:
//!
//! - The BOOT SIGNATURE (`lease_dir` + `summarizer_timeout_ms`) is
//!   everything consumed ONCE at startup: `lease_dir` builds the
//!   `FsLeaseStore` and `summarizer_timeout_ms` the `RouterSummarizer`.
//!   A config change that alters either is REFUSED on hot-reload (logged
//!   "restart required", the previous snapshot kept) — those adapters are
//!   built once at boot and never rebuilt.
//! - Every OTHER field is a per-call tuning knob (token reserves, prune
//!   thresholds, compaction tail, lease TTL). When a freshly-fetched
//!   config's boot signature matches the boot-time signature, the
//!   snapshot is swapped live; handlers read the current snapshot per
//!   call via [`Deps::config`](crate::ports::Deps::config).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::{BootSignature, WorkerConfig};

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a
/// `read().await` and `clone()` the inner `Arc` out (a cheap refcount
/// bump) without holding the lock across its work, while `apply_config`
/// whole-snapshot replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "context-manager";
const CONFIG_FN_ID: &str = "context::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the
/// attempt number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the `context-manager` configuration schema with the
/// configuration worker. When `seed` is present, its value is installed
/// as `initial_value`. Otherwise, the built-in default is seeded only
/// when no stored value exists yet (re-registration preserves the stored
/// value, so this is safe to call every boot).
pub async fn register_config(iii: &III, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Context Manager",
        "description": "Model-ready context assembly tuning: token-budget reserves, \
                        function-result prune thresholds, compaction tail size, the \
                        compaction lease TTL and directory, and the summariser timeout.",
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

/// Read the live `context-manager` configuration (env-expanded by the
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

/// Swap the config snapshot under the write lock. No adapter rebuild —
/// the boot signature is unchanged by construction (the caller has
/// already passed [`reloadable`]).
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Decide whether a freshly-fetched config can be hot-applied. Returns
/// the config when its boot signature matches the boot-time signature
/// (only per-call tuning knobs changed), or an error describing the
/// restart-required change.
fn reloadable(cfg: WorkerConfig, boot_sig: &BootSignature) -> Result<WorkerConfig, String> {
    if cfg.boot_signature() != *boot_sig {
        return Err(
            "configuration change alters lease_dir or summarizer_timeout_ms — these are \
             consumed once at boot (the FsLeaseStore and RouterSummarizer are built then and \
             never rebuilt); a worker restart is required to apply them"
                .to_string(),
        );
    }
    Ok(cfg)
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. `boot_sig` is the signature captured at startup; any reload
/// that would change it is refused (those require a worker restart).
pub fn register_config_trigger(
    iii: &III,
    cell: ConfigCell,
    boot_sig: BootSignature,
) -> Result<(), IIIError> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_payload: Value| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            let boot_sig = boot_sig.clone();
            async move {
                on_config_change(&engine, &cell, &boot_sig).await;
                Ok::<Value, IIIError>(json!({ "ok": true }))
            }
        })
        .description(
            "Internal: reload context-manager's tuning knobs from the authoritative \
             configuration when it changes; lease_dir / summarizer_timeout_ms changes require \
             a restart.",
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

/// Reload tuning knobs from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `context::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller inject arbitrary config without
/// updating persisted state. Re-fetch the stored value via
/// `configuration::get` instead. A restart-required change is refused; the
/// previous snapshot is always kept on any failure path.
async fn on_config_change(iii: &III, cell: &ConfigCell, boot_sig: &BootSignature) {
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
    let cfg = match reloadable(cfg, boot_sig) {
        Ok(cfg) => cfg,
        Err(reason) => {
            tracing::warn!(
                reason = %reason,
                "config-change refused: restart required; keeping previous config"
            );
            return;
        }
    };
    apply_config(cell, cfg).await;
    tracing::info!("context-manager tuning knobs reloaded (boot signature unchanged)");
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

    #[test]
    fn reloadable_allows_tuning_only_change() {
        let boot = WorkerConfig::default();
        let boot_sig = boot.boot_signature();
        let next = WorkerConfig {
            tail_turns: boot.tail_turns + 1,
            reserved_pct: boot.reserved_pct + 1,
            ..boot.clone()
        };
        let applied = reloadable(next, &boot_sig).expect("tuning-only change is reloadable");
        assert_eq!(applied.tail_turns, boot.tail_turns + 1);
    }

    #[test]
    fn reloadable_refuses_lease_dir_change() {
        let boot = WorkerConfig::default();
        let boot_sig = boot.boot_signature();
        let moved = WorkerConfig {
            lease_dir: "/tmp/somewhere-else".to_string(),
            ..boot.clone()
        };
        assert!(reloadable(moved, &boot_sig).is_err());
    }

    #[test]
    fn reloadable_refuses_summarizer_timeout_change() {
        let boot = WorkerConfig::default();
        let boot_sig = boot.boot_signature();
        let retimed = WorkerConfig {
            summarizer_timeout_ms: boot.summarizer_timeout_ms + 1,
            ..boot.clone()
        };
        assert!(reloadable(retimed, &boot_sig).is_err());
    }

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
}
