//! Integration with the `configuration` worker — register the schema,
//! fetch the authoritative value at boot, and hot-reload it when it
//! changes. Mirrors [`context-manager`](../../context-manager/src/configuration.rs) /
//! [`session-manager`](../../session-manager/src/configuration.rs).
//!
//! Every field hot-reloads — nothing requires a restart:
//!
//! - `hook` is the STRUCTURAL field (the harness hook binding). On a
//!   change the config handler **re-binds** the hook live — it registers
//!   the new binding, then `unregister()`s the old (a fail-safe overlap;
//!   the gate is idempotent, so a brief double-fire is harmless) — keeping
//!   the [`Trigger`] handle in [`TriggerHandles`].
//! - Every OTHER field is a per-call tuning knob (the `*_timeout_ms`
//!   budgets and the approval defaults `default_mode` / `always_allow_seed`
//!   / `rules`), read from the live snapshot per call via
//!   [`Deps::config`](crate::functions::Deps::config); a change swaps the
//!   snapshot.
//!
//! `configuration` is a REQUIRED boot dependency: a failed register/fetch
//! aborts startup (the gate must run on a known, authoritative policy
//! surface, never a guessed one).

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::{IIIError, RegisterFunction, RegisterTriggerInput, Trigger, TriggerRequest, III};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a
/// `read().await` and `clone()` the inner `Arc` out (a cheap refcount
/// bump) without holding the lock across its work, while `apply_config`
/// whole-snapshot replaces the inner `Arc` under the write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "approval-gate";
const CONFIG_FN_ID: &str = "approval::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the
/// attempt number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Register the `approval-gate` configuration schema with the
/// configuration worker. When `seed` is present, its value is installed
/// as `initial_value`. Otherwise, the built-in default is seeded only
/// when no stored value exists yet (re-registration preserves the stored
/// value, so this is safe to call every boot).
pub async fn register_config(iii: &III, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Approval Gate",
        "description": "Policy and decision surface settings: the harness hook binding, the \
                        RPC timeout budgets, the deployment approval defaults (permission \
                        mode for new sessions and the auto-mode trust seed), and the agent \
                        permission rules.",
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

/// Read the live `approval-gate` configuration (env-expanded by the
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

/// Persist `value` as the stored configuration entry.
async fn set_config_value(iii: &III, value: Value) -> Result<(), String> {
    trigger_with_retry(iii, "configuration::set", json!({ "id": CONFIG_ID, "value": value }))
        .await
        .map(|_| ())
}

/// Pure decision for [`ensure_rules_seeded`]: given the currently stored
/// value, return the value to persist, or `None` when nothing needs
/// writing. Only an object that lacks a `rules` key is backfilled; a null /
/// non-object value is left to `register_config` + the in-memory defaults,
/// and an object that already has `rules` (even `[]`) is operator data and
/// untouched.
fn rules_backfill(stored: &Value) -> Option<Value> {
    let obj = stored.as_object()?;
    if obj.contains_key("rules") {
        return None;
    }
    let mut next = obj.clone();
    next.insert(
        "rules".into(),
        Value::Array(crate::config::default_rules_value()),
    );
    Some(Value::Object(next))
}

/// Backfill the stored entry with the default `rules` when an operator
/// value predates the field, so the console Configuration editor is
/// pre-filled with the shipped list. First-boot seeding is handled by
/// [`register_config`]'s `initial_value`; this covers only the migration
/// case where a value is already stored but carries no `rules` key. A
/// stored value that already has `rules` (even `[]`) is left untouched —
/// operator data wins. Idempotent.
pub async fn ensure_rules_seeded(iii: &III) -> Result<(), String> {
    let Some(stored) = try_get_config_value(iii).await? else {
        // Nothing stored — `register_config` already seeded the default.
        return Ok(());
    };
    if let Some(next) = rules_backfill(&stored) {
        set_config_value(iii, next).await?;
        tracing::info!("approval-gate configuration backfilled with default rules");
    }
    Ok(())
}

/// Swap the config snapshot under the write lock. Per-call tuning knobs
/// take effect on the next read; any trigger re-bind is done by the caller
/// before this swap.
pub async fn apply_config(cell: &ConfigCell, cfg: WorkerConfig) {
    *cell.write().await = Arc::new(cfg);
}

/// Live handle for the hot-reloadable harness hook binding. The config
/// handler re-binds it on a `hook` change (register the new binding, then
/// `unregister()` the old), so it never needs a restart.
pub struct TriggerHandles {
    pub hook: std::sync::Mutex<Option<Trigger>>,
}

/// Register a best-effort binding and return its handle. The trigger type
/// may not exist yet (standalone deployment); that surfaces later as an
/// async `trigger_type_not_found` log, never an `Err` here, so a `None`
/// means "no handle to retain" rather than a hard failure.
fn bind(iii: &III, trigger_type: &str, function_id: &str, config: Value) -> Option<Trigger> {
    match iii.register_trigger(RegisterTriggerInput {
        trigger_type: trigger_type.to_string(),
        function_id: function_id.to_string(),
        config,
        metadata: None,
    }) {
        Ok(handle) => {
            tracing::info!(trigger_type, function_id, "trigger binding requested");
            Some(handle)
        }
        Err(e) => {
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed (sibling absent?)");
            None
        }
    }
}

/// (Re)bind the `harness::hook::pre-trigger` hook from the current config.
pub fn bind_hook(iii: &III, cfg: &WorkerConfig) -> Option<Trigger> {
    bind(
        iii,
        "harness::hook::pre-trigger",
        "approval::gate",
        json!({
            "functions": cfg.hook.functions,
            "timeout_ms": cfg.hook.timeout_ms,
            "on_error": cfg.hook.on_error,
        }),
    )
}

/// Store the freshly-registered handle, then unregister the old one
/// (register-new-then-unregister-old: a fail-safe overlap — the gate is
/// never left unbound). A `None` new handle means the re-registration
/// didn't produce one; keep the old binding rather than tearing it down.
fn rebind_slot(slot: &std::sync::Mutex<Option<Trigger>>, new: Option<Trigger>) {
    let Some(new) = new else {
        return;
    };
    let old = slot
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .replace(new);
    if let Some(old) = old {
        old.unregister();
    }
}

/// Internal `approval::on-config-change` trigger payload. The handler
/// re-fetches the authoritative configuration, so this carries only the
/// (advisory) configuration id; a struct (not `Value`) keeps the request
/// schema concrete and unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    #[serde(default)]
    pub id: Option<String>,
}

/// Ack returned by the internal `approval::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. `handles` holds the live hook `Trigger` the handler re-binds
/// when `hook` changes. The handler re-fetches via `configuration::get`
/// and ignores the trigger payload, so a direct call can never inject
/// config.
pub fn register_config_trigger(
    iii: &III,
    cell: ConfigCell,
    handles: Arc<TriggerHandles>,
) -> Result<(), IIIError> {
    let cell_for_fn = cell.clone();
    let handles_for_fn = handles.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let handles = handles_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &handles).await;
                Ok::<OnConfigChangeResponse, IIIError>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload approval-gate from the authoritative configuration when it \
             changes — re-binds the harness hook on a hook change and swaps the per-call \
             snapshot (timeouts + approval defaults) otherwise.",
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

/// Reload from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `approval::on-config-change` is a discoverable bus function, so
/// trusting `payload.new_value` would let any caller inject arbitrary
/// config without updating persisted state. Re-fetch the stored value via
/// `configuration::get` instead.
///
/// A `hook` change re-binds the harness hook live
/// (register-new-then-unregister-old); every other field hot-applies via
/// the snapshot swap. The previous config is always kept on a fetch
/// failure.
async fn on_config_change(iii: &III, cell: &ConfigCell, handles: &TriggerHandles) {
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

    // Re-bind the hook only when it actually changed, so an unrelated
    // tuning change never churns the security hook.
    let old = cell.read().await.clone();
    if old.boot_signature() != cfg.boot_signature() {
        rebind_slot(&handles.hook, bind_hook(iii, &cfg));
        tracing::info!("approval-gate hook re-bound (hook config changed)");
    }

    apply_config(cell, cfg).await;
    tracing::info!("approval-gate configuration reloaded");
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
    use crate::types::PermissionMode;

    #[test]
    fn rules_backfill_only_touches_objects_missing_rules() {
        use serde_json::json;
        // An object without `rules` gets the shipped defaults added, other
        // fields preserved.
        let out = rules_backfill(&json!({ "default_mode": "auto" })).unwrap();
        assert_eq!(
            out["rules"],
            Value::Array(crate::config::default_rules_value())
        );
        assert_eq!(out["default_mode"], json!("auto"));
        // An object that already carries `rules` (even empty) is operator
        // data — left untouched.
        assert!(rules_backfill(&json!({ "rules": [] })).is_none());
        assert!(rules_backfill(&json!({ "rules": ["*"] })).is_none());
        // Null / non-object values are not backfilled here.
        assert!(rules_backfill(&Value::Null).is_none());
        assert!(rules_backfill(&json!("garbage")).is_none());
    }

    #[tokio::test]
    async fn apply_config_swaps_snapshot() {
        let cell: ConfigCell = Arc::new(RwLock::new(Arc::new(WorkerConfig::default())));
        assert_eq!(cell.read().await.default_mode, PermissionMode::Manual);

        let tuned = WorkerConfig {
            default_mode: PermissionMode::Full,
            ..WorkerConfig::default()
        };
        apply_config(&cell, tuned).await;
        assert_eq!(cell.read().await.default_mode, PermissionMode::Full);
    }

    #[test]
    fn rebind_slot_registers_new_then_unregisters_old() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        // The old handle's unregister closure bumps a counter, proving
        // register-new-then-unregister-old tears the old binding down only
        // after the new one is stored.
        let unregistered = Arc::new(AtomicUsize::new(0));
        let counter = unregistered.clone();
        let old = Trigger::new(Arc::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
        }));
        let slot = std::sync::Mutex::new(Some(old));

        rebind_slot(&slot, Some(Trigger::new(Arc::new(|| {}))));
        assert_eq!(
            unregistered.load(Ordering::SeqCst),
            1,
            "the previous binding must be unregistered after the new one is stored"
        );

        // A `None` new handle (re-registration produced nothing) keeps the
        // current binding untouched.
        rebind_slot(&slot, None);
        assert_eq!(unregistered.load(Ordering::SeqCst), 1);
    }
}
