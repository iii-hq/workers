//! Integration with the `configuration` worker — register, fetch, and
//! hot-reload the `coder` configuration entry.
//!
//! coder's config splits into two halves on a live update:
//!
//! - The JAIL SIGNATURE (`base_path` + `base_paths` + `non_accessible_globs` +
//!   `default_exclude_globs`) is everything the boot-time `PathResolver`
//!   compiles. The `PathResolver` is the security boundary and is built ONCE at
//!   boot and NEVER rebuilt at runtime; a config change that alters any of those
//!   four fields is REFUSED on hot-reload (logged "restart coder to apply", the
//!   previous snapshot kept) — mirroring storage's topology-change refusal.
//! - Every OTHER field is a numeric tuning knob (byte caps, page sizes, response
//!   budgets). When a freshly-fetched config's jail signature matches the boot
//!   signature, the snapshot is swapped live; handlers read the current snapshot
//!   per call.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::{CoderConfig, JailSignature};

/// Hot-swappable config snapshot shared with every cfg-taking handler. The
/// `Arc<RwLock<Arc<CoderConfig>>>` shape lets a handler take a `read().await`
/// and `clone()` the inner `Arc` out (a cheap refcount bump) without holding the
/// lock across its work, while `apply_config` whole-snapshot replaces the inner
/// `Arc` under the write lock. The `PathResolver` is NOT stored here — it is the
/// security jail, built once at boot and never swapped.
pub type ConfigCell = Arc<RwLock<Arc<CoderConfig>>>;

pub const CONFIG_ID: &str = "coder";
const CONFIG_FN_ID: &str = "coder::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register the `coder` configuration schema with the configuration worker.
/// When `seed` is present, its value is installed as `initial_value`. Otherwise,
/// the built-in default is seeded only when no stored value exists yet.
pub async fn register_config(iii: &III, seed: Option<&CoderConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Coder",
        "description": "Path-jailed code file access for iii agents: read/search/edit/create/move files inside a fixed set of allowed roots, with non-accessible glob protection and token-bounded response budgets.",
        "schema": CoderConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default_value(iii).await? {
        payload["initial_value"] = CoderConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Read the live `coder` configuration (env-expanded by the configuration
/// worker — `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &III) -> Result<CoderConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(CoderConfig::default());
    }
    CoderConfig::from_json(&value)
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

/// Returns `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_config_value(iii: &III) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Decide whether a freshly-fetched config can be hot-applied. Returns the
/// config when its jail signature matches the boot-time signature (only numeric
/// tuning knobs changed), or an error describing the jail change that requires a
/// worker restart. Mirrors storage's `reloadable` topology gate.
fn reloadable(cfg: CoderConfig, boot_sig: &JailSignature) -> Result<CoderConfig, String> {
    if cfg.jail_signature() != *boot_sig {
        return Err(
            "configuration change alters the path jail (base_paths / non_accessible_globs / \
             default_exclude_globs) — these define the security boundary and the compiled \
             PathResolver; a worker restart is required to apply them"
                .to_string(),
        );
    }
    Ok(cfg)
}

/// Swap the config snapshot under the write lock. No resolver rebuild — the jail
/// is unchanged by construction (the caller has already passed `reloadable`).
pub async fn apply_config(cell: &ConfigCell, cfg: CoderConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Internal `coder::on-config-change` trigger payload. The handler re-fetches
/// the authoritative configuration, so this carries only the (advisory)
/// configuration id; a struct (not `Value`) keeps the request schema concrete
/// and unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `coder::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger.
///
/// `boot_sig` is the jail signature captured at startup; any reload that would
/// change it is refused (those require a worker restart because the
/// `PathResolver` is never rebuilt).
pub fn register_config_trigger(
    iii: &III,
    cell: ConfigCell,
    boot_sig: JailSignature,
) -> Result<(), IIIError> {
    let cell_for_fn = cell.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let engine = engine.clone();
            let boot_sig = boot_sig.clone();
            async move {
                on_config_change(&engine, &cell, &boot_sig).await;
                Ok::<OnConfigChangeResponse, IIIError>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload coder's tuning limits from the authoritative configuration when it \
             changes; jail-defining changes require a restart.",
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

/// Reload coder's tuning limits from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `coder::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller inject arbitrary config (loosening
/// byte caps or — worse — the jail) without updating persisted state. Re-fetch
/// the stored value via `configuration::get` instead. A jail-changing update is
/// refused (it requires a restart); the previous snapshot is always kept on any
/// failure path.
async fn on_config_change(iii: &III, cell: &ConfigCell, boot_sig: &JailSignature) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous limits"
            );
            return;
        }
    };
    let cfg = match reloadable(cfg, boot_sig) {
        Ok(cfg) => cfg,
        Err(reason) => {
            tracing::warn!(
                reason = %reason,
                "config-change refused: jail change requires restart; keeping previous limits"
            );
            return;
        }
    };
    apply_config(cell, cfg).await;
    tracing::info!("coder tuning limits reloaded (jail unchanged)");
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
                    tokio::time::sleep(Duration::from_millis(250 * u64::from(attempt))).await;
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
    use std::path::PathBuf;

    #[test]
    fn reloadable_allows_numeric_only_change() {
        // Two configs sharing a jail signature, differing only in a numeric
        // tuning knob, hot-apply (no PathResolver rebuild needed).
        let boot = CoderConfig {
            base_paths: vec![PathBuf::from("/tmp/a")],
            non_accessible_globs: vec!["**/.env".to_string()],
            default_exclude_globs: vec!["**/target/**".to_string()],
            ..CoderConfig::default()
        };
        let next = CoderConfig {
            max_read_bytes: boot.max_read_bytes + 1,
            ..boot.clone()
        };
        let boot_sig = boot.jail_signature();
        let applied = reloadable(next, &boot_sig).expect("numeric-only change is reloadable");
        assert_eq!(applied.max_read_bytes, boot.max_read_bytes + 1);
    }

    #[test]
    fn reloadable_refuses_jail_change() {
        let boot = CoderConfig {
            base_paths: vec![PathBuf::from("/tmp/a")],
            non_accessible_globs: vec!["**/.env".to_string()],
            default_exclude_globs: vec!["**/target/**".to_string()],
            ..CoderConfig::default()
        };
        let boot_sig = boot.jail_signature();

        // base_path change -> refused.
        let changed_base_path = CoderConfig {
            base_path: Some(PathBuf::from("/tmp/legacy")),
            ..boot.clone()
        };
        assert!(reloadable(changed_base_path, &boot_sig).is_err());

        // base_paths change -> refused.
        let changed_base_paths = CoderConfig {
            base_paths: vec![PathBuf::from("/tmp/a"), PathBuf::from("/tmp/b")],
            ..boot.clone()
        };
        assert!(reloadable(changed_base_paths, &boot_sig).is_err());

        // non_accessible_glob change -> refused.
        let changed_non_accessible = CoderConfig {
            non_accessible_globs: vec!["**/.env".to_string(), "**/*.pem".to_string()],
            ..boot.clone()
        };
        assert!(reloadable(changed_non_accessible, &boot_sig).is_err());

        // default_exclude_glob change -> refused.
        let changed_default_exclude = CoderConfig {
            default_exclude_globs: vec!["**/build/**".to_string()],
            ..boot.clone()
        };
        assert!(reloadable(changed_default_exclude, &boot_sig).is_err());
    }

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(CoderConfig::default())));
        let before = cell.read().await.clone();
        assert_eq!(before.max_read_bytes, CoderConfig::default().max_read_bytes);

        let tuned = CoderConfig {
            max_read_bytes: 7,
            ..CoderConfig::default()
        };
        apply_config(&cell, tuned).await;

        let after = cell.read().await.clone();
        assert_eq!(after.max_read_bytes, 7);
    }
}
