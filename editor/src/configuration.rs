//! Integration with the `configuration` worker — register the schema, fetch the
//! authoritative value at boot, and hot-reload it when it changes. Mirrors
//! [`context-manager`](../../context-manager/src/configuration.rs).
//!
//! Every field here is a tuning knob: caps, limits and a timeout. None of them
//! is structural — there is no adapter to rebuild and no trigger to re-bind,
//! because this worker owns no resources of its own (files go through `shell`,
//! the workspace record through `state`). So a reload is a snapshot swap, and
//! every handler reads the live snapshot per call rather than capturing one at
//! registration. Nothing requires a restart.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::config::WorkerConfig;

/// Hot-swappable config snapshot shared with every handler. The
/// `Arc<RwLock<Arc<WorkerConfig>>>` shape lets a handler take a `read().await`
/// and clone the inner `Arc` out (a refcount bump) without holding the lock
/// across its work, while [`apply_config`] replaces the inner `Arc` under the
/// write lock.
pub type ConfigCell = Arc<RwLock<Arc<WorkerConfig>>>;

pub const CONFIG_ID: &str = "editor";
const CONFIG_FN_ID: &str = "editor::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the attempt
/// number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;
/// How many times the background recovery re-attempts the boot handshake after
/// a fallback. At the backoff below this covers roughly twenty minutes, which is
/// far longer than any worker start-order race.
const RECOVERY_ATTEMPTS: u32 = 60;

/// Where a served configuration came from, for the boot log and for tests.
pub const SOURCE_SEED: &str = "the --config seed";
pub const SOURCE_DEFAULTS: &str = "the built-in defaults";

pub fn cell(cfg: WorkerConfig) -> ConfigCell {
    Arc::new(RwLock::new(Arc::new(cfg)))
}

/// Register the `editor` configuration schema.
///
/// When `seed` is present its value is installed as `initial_value`; otherwise
/// the built-in default is seeded only when nothing is stored yet, so calling
/// this on every boot never overwrites an operator's edit.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Editor",
        "description": "Editor workspace limits: diff size and context, file-finder and \
                        content-search caps, the largest file an open will pull back, and \
                        the per-git-invocation timeout. Nothing here grants access — the \
                        filesystem boundary is the shell worker's jail.",
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

/// Read the live configuration (env-expanded by the configuration worker —
/// `from_json` does NOT re-expand).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))?;
    if value.is_null() {
        tracing::info!("no stored configuration; using built-in defaults");
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

/// `Ok(None)` when the entry does not exist. The engine's missing-entry codes
/// vary in case (`function_not_found`, `NOT_FOUND`), so match case-insensitively.
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
    let mut last = String::new();
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
            Ok(value) => return Ok(value),
            Err(e) => {
                last = e.to_string();
                // A missing entry is an answer, not a transient failure — do
                // not spend the whole retry budget on it.
                if last.to_ascii_uppercase().contains("NOT_FOUND") {
                    break;
                }
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * attempt as u64,
                    ))
                    .await;
                }
            }
        }
    }
    Err(last)
}

/// What to serve when the boot handshake never completed.
///
/// The seed, whenever there is one. The `--config` file is where an operator
/// wrote their intent, and falling back to `WorkerConfig::default()` while a
/// seed exists would silently loosen every limit they had tightened — a safety
/// regression dressed as a resilience fix. With no seed at all nothing was
/// configured on this side, so the shipped baseline genuinely is the operator's
/// choice.
///
/// Either way this is a *stopgap*: [`spawn_boot_recovery`] keeps trying for the
/// authoritative value, and the returned source string is what the boot log
/// prints so "running on the seed" is never mistaken for "running on live
/// config".
pub fn fallback_config(seed: Option<&WorkerConfig>) -> (WorkerConfig, &'static str) {
    match seed {
        Some(seed) => (seed.clone(), SOURCE_SEED),
        None => (WorkerConfig::default(), SOURCE_DEFAULTS),
    }
}

/// Keep trying to complete the boot handshake after a fallback, and publish the
/// authoritative values the moment it lands.
///
/// Without this the fallback would be permanent in its likeliest cause. When it
/// is `configuration::register` that never landed — the configuration worker was
/// not up yet — the configuration worker holds no `editor` entry, so nothing can
/// ever emit the `configuration:updated` event the hot-reload trigger waits on,
/// and the stopgap limits would be the limits for the life of the process. A
/// worker permanently stuck on a seed is worse than one that refused to start.
///
/// It publishes through [`apply_config`], the same path the hot-reload handler
/// uses, so the git timeout the bus reads is republished too.
pub fn spawn_boot_recovery(
    iii: Arc<IIIClient>,
    cell: ConfigCell,
    git_timeout_ms: Arc<AtomicU64>,
    seed: Option<WorkerConfig>,
) {
    tokio::spawn(async move {
        for attempt in 1..=RECOVERY_ATTEMPTS {
            tokio::time::sleep(recovery_backoff(attempt)).await;
            if let Err(e) = register_config(&iii, seed.as_ref()).await {
                tracing::debug!(attempt, error = %e, "configuration still not registrable");
                continue;
            }
            match fetch_config(&iii).await {
                Ok(cfg) => {
                    apply_config(&cell, &git_timeout_ms, cfg).await;
                    tracing::warn!(
                        attempt,
                        "configuration handshake recovered; the editor is serving the \
                         authoritative limits and no longer the boot fallback"
                    );
                    return;
                }
                Err(e) => tracing::debug!(attempt, error = %e, "configuration still unreadable"),
            }
        }
        tracing::error!(
            attempts = RECOVERY_ATTEMPTS,
            "the configuration worker never answered; the editor is serving its boot fallback \
             limits for the rest of this process. Restart it once the configuration worker is up."
        );
    });
}

/// Linear, then capped: quick retries for a configuration worker that is merely
/// starting, then a slow poll that costs nothing while it stays down.
fn recovery_backoff(attempt: u32) -> Duration {
    Duration::from_millis((500 * u64::from(attempt)).min(30_000))
}

/// Swap the snapshot. Handlers read it per call, so the next invocation of
/// every function sees the new values.
pub async fn apply_config(cell: &ConfigCell, git_timeout_ms: &AtomicU64, cfg: WorkerConfig) {
    // The bus holds the git timeout separately (it is read on a path that has
    // no access to the snapshot), so it has to be published here or it would
    // be the one field a reload silently skipped.
    git_timeout_ms.store(cfg.git_timeout_ms, Ordering::Relaxed);
    *cell.write().await = Arc::new(cfg);
}

/// Internal `editor::on-config-change` payload. The handler re-fetches the
/// authoritative value, so this carries only the (advisory) id; a struct rather
/// than a `Value` keeps the request schema concrete.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches).
    #[serde(default)]
    pub id: Option<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger. Registered here rather than in `functions::register_all` so it
/// stays off the public `catalog()`.
///
/// The bind is retried on the same shape as the boot handshake, and for the same
/// reason: a bus call that fails once against a busy engine is not a reason to
/// give up, because the caller's only other move is to exit and be restarted
/// straight back into the same race. Only a bind that never succeeds is an
/// error, and the caller decides what to do about it.
pub async fn register_config_trigger(
    iii: &IIIClient,
    cell: ConfigCell,
    git_timeout_ms: Arc<AtomicU64>,
) -> Result<(), Error> {
    let cell_for_fn = cell.clone();
    let timeout_for_fn = git_timeout_ms.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let cell = cell_for_fn.clone();
            let timeout = timeout_for_fn.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &cell, &timeout).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: hot-reload the editor's limits from the authoritative configuration \
             when it changes.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let mut last: Option<Error> = None;
    for attempt in 1..=CONFIG_RETRIES {
        match iii.register_trigger(RegisterTriggerInput {
            trigger_type: "configuration".to_string(),
            function_id: CONFIG_FN_ID.to_string(),
            config: json!({
                "configuration_id": CONFIG_ID,
                "event_types": ["configuration:updated"],
            }),
            metadata: None,
        }) {
            Ok(_) => return Ok(()),
            Err(e) => {
                tracing::warn!(
                    attempt,
                    error = %e,
                    "could not bind the configuration trigger; retrying"
                );
                last = Some(e);
                if attempt < CONFIG_RETRIES {
                    tokio::time::sleep(Duration::from_millis(
                        CONFIG_RETRY_BACKOFF_MS * attempt as u64,
                    ))
                    .await;
                }
            }
        }
    }
    Err(last.unwrap_or_else(|| {
        Error::Handler("binding the configuration trigger never succeeded".into())
    }))
}

/// Reload from the AUTHORITATIVE configuration.
///
/// The trigger payload is deliberately ignored: `editor::on-config-change` is a
/// bus function, so trusting a caller-supplied value would let anyone inject
/// limits without updating persisted state. A failed fetch keeps the previous
/// snapshot (last-good) rather than falling back to defaults, which would
/// silently widen every cap.
async fn on_config_change(iii: &IIIClient, cell: &ConfigCell, git_timeout_ms: &AtomicU64) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(cell, git_timeout_ms, cfg).await;
            tracing::info!("editor configuration reloaded");
        }
        Err(e) => tracing::error!(
            error = %e,
            "config-change: failed to fetch authoritative configuration; keeping previous config"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A boot that exhausted its retries must keep the operator's numbers. The
    /// seed is `config.yaml`, so falling back to `WorkerConfig::default()` while
    /// one exists would hand back every limit the operator had tightened.
    #[test]
    fn a_failed_boot_falls_back_to_the_seed_not_the_defaults() {
        let seed = WorkerConfig {
            max_file_bytes: 64_000,
            git_timeout_ms: 3_000,
            ..WorkerConfig::default()
        };
        assert_ne!(
            seed,
            WorkerConfig::default(),
            "the fixture has to differ from the baseline to prove anything"
        );

        let (cfg, source) = fallback_config(Some(&seed));
        assert_eq!(cfg, seed, "the seed's values, not the shipped ones");
        assert_eq!(cfg.max_file_bytes, 64_000);
        assert_eq!(cfg.git_timeout_ms, 3_000);
        assert_eq!(source, SOURCE_SEED);
    }

    /// With nothing seeded, nothing was configured on this side, so the baseline
    /// is the operator's choice by default.
    #[test]
    fn without_a_seed_the_fallback_is_the_baseline() {
        let (cfg, source) = fallback_config(None);
        assert_eq!(cfg, WorkerConfig::default());
        assert_eq!(source, SOURCE_DEFAULTS);
    }

    /// A reload has to publish BOTH halves of the snapshot. The bus reads the
    /// git timeout off an atomic, on a path with no access to the cell, so a
    /// swap that updated only the cell would leave every git invocation on the
    /// boot-time timeout — the one field a reload silently skipped.
    #[tokio::test]
    async fn applying_a_config_publishes_the_snapshot_and_the_git_timeout() {
        let boot = WorkerConfig::default();
        let git_timeout = Arc::new(AtomicU64::new(boot.git_timeout_ms));
        let live = cell(boot.clone());

        let next = WorkerConfig {
            git_timeout_ms: boot.git_timeout_ms + 1_000,
            max_file_bytes: 123,
            ..WorkerConfig::default()
        };
        assert_ne!(next, boot, "the fixture has to differ to prove anything");

        apply_config(&live, &git_timeout, next.clone()).await;

        assert_eq!(
            **live.read().await,
            next,
            "the next call of every handler reads the new snapshot"
        );
        assert_eq!(
            git_timeout.load(Ordering::Relaxed),
            next.git_timeout_ms,
            "the bus takes its timeout from the atomic, not from the cell"
        );
    }

    /// The boot fallback is a stopgap, not a ceiling. Once the handshake
    /// recovers, the authoritative values replace the seed through exactly the
    /// path a hot reload uses — including the timeout the seed had set.
    #[tokio::test]
    async fn the_authoritative_config_replaces_a_boot_fallback() {
        let seed = WorkerConfig {
            max_file_bytes: 64_000,
            git_timeout_ms: 3_000,
            ..WorkerConfig::default()
        };
        let (booted, source) = fallback_config(Some(&seed));
        assert_eq!(source, SOURCE_SEED);

        let git_timeout = Arc::new(AtomicU64::new(booted.git_timeout_ms));
        let live = cell(booted);
        assert_eq!(git_timeout.load(Ordering::Relaxed), 3_000);

        let authoritative = WorkerConfig {
            max_file_bytes: 9_000_000,
            git_timeout_ms: 45_000,
            ..WorkerConfig::default()
        };
        apply_config(&live, &git_timeout, authoritative.clone()).await;

        assert_eq!(**live.read().await, authoritative);
        assert_eq!(
            git_timeout.load(Ordering::Relaxed),
            45_000,
            "the seed's timeout must not outlive the recovery"
        );
    }

    #[test]
    fn recovery_backoff_grows_then_caps() {
        assert_eq!(recovery_backoff(1), Duration::from_millis(500));
        assert_eq!(recovery_backoff(4), Duration::from_millis(2_000));
        assert_eq!(
            recovery_backoff(RECOVERY_ATTEMPTS),
            Duration::from_millis(30_000),
            "a configuration worker that stays down must not be polled faster and faster"
        );
    }
}
