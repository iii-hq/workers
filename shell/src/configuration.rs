//! Integration with the `configuration` worker — register the shell security
//! policy schema, fetch the live value, and hot-reload the runtime (config +
//! host fs backend) when it changes. Mirrors `database/src/configuration.rs`.

use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::{Mutex, RwLock};

use crate::code::path::PathResolver;
use crate::code::state::CodeCells;
use crate::config::ShellConfig;
use crate::fs::host::{HostFsBackend, HostFsConfig, IiiChannelMaker};
use crate::fs::FsBackend;

pub const CONFIG_ID: &str = "shell";
const CONFIG_FN_ID: &str = "shell::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
/// Base backoff between configuration RPC retries; multiplied by the attempt
/// number for a linear backoff (250ms, 500ms, …).
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// Live runtime swapped on hot-reload. `config` and `host_backend` are always
/// built from the same `ShellConfig`, so a reader never mixes new config with
/// an old backend.
pub struct ShellRuntime {
    pub config: Arc<ShellConfig>,
    pub host_backend: Arc<dyn FsBackend>,
}

#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<RwLock<ShellRuntime>>,
    pub code_cells: Option<CodeCells>,
    pub iii: IIIClient,
    /// Serializes hot-reloads: held across the authoritative fetch + build + swap
    /// so an older event's slow build can never clobber a newer applied config.
    pub reload_lock: Arc<Mutex<()>>,
    /// Last hot-reload outcome, exposed via `shell::config-status` so operators
    /// can detect when a stored config was rejected and the active policy diverged.
    pub reload_status: Arc<RwLock<ReloadStatus>>,
}

pub fn build_code_cells(cfg: &ShellConfig) -> Result<CodeCells, String> {
    let (code_cfg, resolver) = build_code_snapshot(cfg)?;
    Ok(CodeCells {
        config: Arc::new(RwLock::new(code_cfg)),
        resolver: Arc::new(RwLock::new(resolver)),
    })
}

fn build_code_snapshot(
    cfg: &ShellConfig,
) -> Result<
    (
        Arc<crate::code::config::CoderConfig>,
        Arc<crate::code::path::PathResolver>,
    ),
    String,
> {
    if !cfg.fs.is_jailed() {
        return Err("coder surface requires a jailed fs.host_roots config".to_string());
    }
    let code_cfg = Arc::new(cfg.code_resolver_config());
    let resolver = Arc::new(
        PathResolver::new(&code_cfg)
            .map_err(|e| format!("failed to build code PathResolver (coder::*): {e}"))?,
    );
    Ok((code_cfg, resolver))
}

/// Outcome of the most recent hot-reload attempt, exposed via `shell::config-status`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReloadOutcome {
    /// The live runtime reflects the most recent configuration the worker loaded.
    Applied,
    /// The most recent configuration:updated delivered a value the worker could
    /// NOT build; the previous valid policy is still active and the central store
    /// has DIVERGED from what shell is enforcing.
    Rejected,
}

/// Operator-visible hot-reload status. `rejected_reloads > 0` (or
/// `last_outcome == Rejected`) means a stored config was refused and shell is
/// enforcing an older policy than the central store — actionable divergence.
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

/// Validate + compile a config into the shared, immutable form handlers read.
/// Pure (no I/O), so it is unit-testable without a live engine.
pub fn prepare_config(cfg: &ShellConfig) -> Result<Arc<ShellConfig>, String> {
    let mut prepared = cfg.clone();
    prepared
        .compile_denylist()
        .map_err(|e| format!("denylist compile: {e}"))?;
    prepared
        .validate_fs_jail()
        .map_err(|e| format!("fs jail: {e}"))?;
    Ok(Arc::new(prepared))
}

/// Build the live runtime: validate the config, then build the host fs backend.
pub fn build_runtime(cfg: &ShellConfig, iii: &IIIClient) -> Result<ShellRuntime, String> {
    let config = prepare_config(cfg)?;
    if !config.fs.is_jailed() {
        tracing::warn!(
            "fs.host_roots is empty — host backend is unjailed; every absolute \
             path is reachable by shell::fs::* (denylist still applies)"
        );
    }
    let host_fs_cfg = Arc::new(HostFsConfig {
        host_roots: config.fs.roots(),
        max_read_bytes: config.fs.max_read_bytes,
        max_write_bytes: config.fs.max_write_bytes,
        denylist_paths: config.fs.denylist_paths.clone(),
        // D4: shell::fs honors the SAME protected globs the code surface uses,
        // so secrets are declared once (under `code.non_accessible_globs`).
        non_accessible_globs: config.code.non_accessible_globs.clone(),
        allow_special_bits: config.fs.allow_special_bits,
    });
    let chan_maker = Arc::new(IiiChannelMaker::new(iii.clone()));
    let host_backend: Arc<dyn FsBackend> = Arc::new(
        HostFsBackend::try_new(host_fs_cfg, chan_maker)
            .map_err(|e| format!("host backend init: {} ({})", e.message, e.code))?,
    );
    Ok(ShellRuntime {
        config,
        host_backend,
    })
}

/// Register the `shell` configuration schema with the configuration worker.
///
/// `initial_value` (used only on first registration; preserved afterwards) is
/// taken from an explicit `--config` seed when given, otherwise from the
/// built-in zero-config default when no value is stored yet — so the worker
/// boots with no config file at all (database parity). Unlike database we
/// cannot seed `ShellConfig::default()`: it is intentionally unjailed/invalid,
/// so the built-in seed is `ShellConfig::seed_default()`, a bootable permissive
/// dev default.
pub async fn register_config(iii: &IIIClient, seed: Option<&ShellConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Shell",
        "description": "Command allowlist/denylist, timeout & output caps, and the fs jail.",
        "schema": ShellConfig::json_schema(),
    });
    let candidate: Option<ShellConfig> = match seed {
        Some(s) => Some(s.clone()),
        None if should_seed_default_value(iii).await? => Some(ShellConfig::seed_default()),
        None => None,
    };
    if let Some(cfg) = &candidate {
        // Validate with the SAME checks build_runtime uses (denylist regex
        // compile, fs-jail rule, host_roots/denylist reachability) BEFORE
        // persisting. configuration::register preserves the value after first
        // registration, so a one-line typo in --config — or an unbootable
        // built-in seed — would become a persistent outage. If invalid, register
        // the schema only and let the worker fail closed with a clear error.
        match build_runtime(cfg, iii) {
            Ok(_) => payload["initial_value"] = cfg.to_json(),
            Err(e) => tracing::error!(
                error = %e,
                "ignoring invalid config seed; not registering it as initial_value"
            ),
        }
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

/// Seed the built-in default only when nothing is stored yet — never overwrite
/// an operator's persisted value.
async fn should_seed_default_value(iii: &IIIClient) -> Result<bool, String> {
    match try_get_config_value(iii).await? {
        None => Ok(true),
        Some(value) if value.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

/// Read the live `shell` configuration (env-expanded by the configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<ShellConfig, String> {
    let value = get_config_value(iii).await?;
    parse_fetched_value(value)
}

/// Parse a successfully fetched raw configuration value. Split from
/// [`fetch_config`] so `reload_serialized` can classify a PARSE failure of a
/// fetched value as `Rejected` (permanent — re-fetching returns the same
/// value) rather than a transient fetch error (retryable). Conflating the two
/// turns every un-migrated stored value into a retry storm and hides the
/// divergence from `shell::config-status`.
fn parse_fetched_value(value: Value) -> Result<ShellConfig, String> {
    if value.is_null() {
        // Null means register_config did not seed (its seed_default failed
        // validation) or the stored value was nulled at runtime. Fall back to
        // the unjailed Default::default(), which build_runtime rejects: boot
        // fails closed, and a steady-state hot-reload keeps last-good rather
        // than silently widening the live jail to the /tmp seed default. Boot
        // zero-config comes from register_config seeding seed_default(), not
        // from this fallback.
        tracing::warn!(
            "configuration value is null; falling back to the built-in default, which is \
             intentionally invalid (unjailed) and is rejected by build_runtime — boot \
             fails closed, hot-reload keeps last-good"
        );
        return Ok(ShellConfig::default());
    }
    ShellConfig::from_json(&value)
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii)
        .await?
        .ok_or_else(|| format!("configuration `{CONFIG_ID}` not found"))
}

async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        // `trigger_with_retry` flattens the structured `Error` to its
        // Display string, so we substring-match the recovered message rather
        // than branch on `Error::Remote { code }`. The engine's missing-entry
        // codes vary in case (`function_not_found`, `STATEMENT_NOT_FOUND`,
        // `NOT_FOUND`), so uppercase before matching to catch them all. A
        // false negative is non-fatal — it just propagates the raw retry error
        // instead of the cleaner "not found", and boot fails closed either way.
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Build a new runtime and hot-swap it. On any failure the previous runtime is
/// left untouched (last-good). A jail-widening reload succeeds but is logged.
///
/// MUST only be called from `reload_serialized` (i.e. while holding
/// `reload_lock`): the build happens before the swap, so an unserialized caller
/// could let a slow older build clobber a newer applied config (the TOCTOU this
/// module guards against). Kept private so the reload lock is the only entry point.
async fn apply_config(state: &AppState, cfg: ShellConfig) -> Result<(), String> {
    let new_runtime = build_runtime(&cfg, &state.iii)?;
    let code_snapshot = state
        .code_cells
        .as_ref()
        .and_then(|_| match build_code_snapshot(&cfg) {
            Ok(snapshot) => Some(snapshot),
            Err(e) => {
                tracing::error!(
                    error = %e,
                    "keeping last-good coder config/resolver after rejected code-surface reload"
                );
                None
            }
        });
    let mut guard = state.runtime.write().await;
    let was_jailed = guard.config.fs.is_jailed();
    let now_jailed = new_runtime.config.fs.is_jailed();
    if was_jailed && !now_jailed {
        tracing::warn!(
            "configuration change WIDENED the fs jail (host_roots cleared); the entire \
             host filesystem is now reachable through shell::fs::* (denylist still applies)"
        );
    }
    *guard = new_runtime;
    drop(guard);
    if let (Some(cells), Some((code_cfg, resolver))) = (&state.code_cells, code_snapshot) {
        *cells.resolver.write().await = resolver;
        *cells.config.write().await = code_cfg;
    }
    Ok(())
}

/// Event delivered to the internal `shell::on-config-change` handler. A struct
/// (not `Value`) keeps the request schema concrete; the handler re-fetches the
/// configuration id; unknown fields are ignored.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnConfigChangeEvent {
    /// Configuration id that changed (advisory; the handler re-fetches the value).
    /// Schema-only: kept to publish a typed request schema; the handler ignores it.
    #[serde(default)]
    #[allow(dead_code)]
    pub id: Option<String>,
}

/// Ack returned by the internal `shell::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler and bind a `configuration` trigger.
pub fn register_config_trigger(iii: &IIIClient, state: AppState) -> Result<(), Error> {
    let st = state.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let st = st.clone();
            async move {
                on_config_change(&st).await.map_err(Error::from)?;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description("Internal: reload the security policy + fs backend on configuration change."),
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

/// Run a config reload under the serialized reload lock. The `fetch` future is
/// awaited INSIDE the lock, so overlapping `configuration:updated` events are
/// applied one at a time and each observes the latest authoritative value — a
/// slow build from an older event can never overwrite a newer applied config.
///
/// Three-way classification: a TRANSPORT failure (fetch Err) is transient —
/// surface Err so the dispatcher retries. A PARSE failure of the fetched value
/// (removed 0.7.0 keys, malformed JSON shape) is permanent — re-fetching
/// returns the same bytes — so it is `Rejected`: keep last-good, record for
/// `shell::config-status`, ack (no retry storm). An unbuildable parsed config
/// is likewise `Rejected` via `apply_config`.
async fn reload_serialized<F, Fut>(state: &AppState, fetch: F) -> Result<ReloadOutcome, String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    let _reload = state.reload_lock.lock().await;
    let raw = match fetch().await {
        Ok(raw) => raw,
        Err(e) => {
            // Transient fetch failure (e.g. configuration::get timed out): the
            // authoritative value is unknown. Keep the previous runtime AND
            // surface the error so the dispatcher can retry — never ack success.
            tracing::error!(
                error = %e,
                "failed to fetch configuration on change; keeping previous runtime, signaling retry"
            );
            return Err(e);
        }
    };
    let cfg = match parse_fetched_value(raw) {
        Ok(cfg) => cfg,
        Err(e) => {
            // The value WAS fetched; it just doesn't parse (e.g. an
            // un-migrated 0.6.x shape carrying removed keys). Same class as
            // unbuildable below: permanent until the store changes.
            tracing::error!(
                error = %e,
                "rejected configuration change; keeping previous runtime (not retrying — value does not parse)"
            );
            state.reload_status.write().await.record_rejected(e);
            return Ok(ReloadOutcome::Rejected);
        }
    };
    match apply_config(state, cfg).await {
        Ok(()) => {
            tracing::info!("shell runtime reloaded after configuration change");
            // Record AFTER apply_config returned so we never hold the runtime
            // and reload_status locks at the same time.
            state.reload_status.write().await.record_applied();
            Ok(ReloadOutcome::Applied)
        }
        Err(e) => {
            // Config was fetched but is unbuildable (bad denylist regex, unreachable
            // jail root, …). Re-fetching returns the SAME rejected value, so ack +
            // keep last-good to avoid a retry storm; the loud error log is the signal.
            // Record the rejection so `shell::config-status` makes the divergence
            // (active policy older than the central store) operator-visible.
            tracing::error!(
                error = %e,
                "rejected configuration change; keeping previous runtime (not retrying — config invalid)"
            );
            state.reload_status.write().await.record_rejected(e);
            // Steady-state hot-reload tolerates this (last-good); boot reconcile
            // turns it into a hard failure — see `reconcile_with`.
            Ok(ReloadOutcome::Rejected)
        }
    }
}

async fn on_config_change(state: &AppState) -> Result<(), String> {
    // SECURITY: do not trust the event payload. `shell::on-config-change` is a
    // registered (callable) function, so a direct caller could forge `new_value`
    // to hot-swap the security policy (allowlist, fs jail, sandbox toggle),
    // bypassing configuration::set authorization. Always re-read the authoritative
    // value from the configuration worker and apply that — a spurious direct call
    // can at most trigger a reload of the already-stored config.
    // Reloads are serialized and re-fetch inside the lock, so concurrent
    // configuration:updated events converge to the latest authoritative config
    // and a slow build from an older event cannot roll back a newer policy.
    // Steady-state: an invalid config keeps last-good (Rejected is not an error
    // here), only a transient fetch failure propagates Err for the dispatcher.
    reload_serialized(state, || get_config_value(&state.iii))
        .await
        .map(|_| ())
}

/// Re-fetch the authoritative config and apply it, serialized with any
/// trigger-driven reload. Called once at boot right after the trigger is
/// registered to close the window between the initial fetch and trigger
/// registration: an update that landed in that window had no listener, and this
/// picks it up. FAIL-CLOSED: unlike a steady-state hot-reload, a rejected
/// (unbuildable) authoritative config aborts startup rather than serving the
/// stale last-good runtime, and a transient fetch failure also aborts.
pub async fn reconcile(state: &AppState) -> Result<(), String> {
    reconcile_with(state, || get_config_value(&state.iii)).await
}

/// Boot reconcile core, split from `fetch_config` so the fail-closed mapping is
/// unit-testable without a live engine.
async fn reconcile_with<F, Fut>(state: &AppState, fetch: F) -> Result<(), String>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<Value, String>>,
{
    match reload_serialized(state, fetch).await? {
        ReloadOutcome::Applied => Ok(()),
        ReloadOutcome::Rejected => Err(
            "stored `shell` configuration is invalid and could not be built; refusing to \
             start under a stale policy (fix the central config, then restart)"
                .to_string(),
        ),
    }
}

async fn trigger_with_retry(
    iii: &IIIClient,
    function_id: &str,
    payload: Value,
) -> Result<Value, String> {
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
                    tracing::warn!(function_id, attempt, error = %last_err, "configuration RPC failed; retrying");
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
    fn build_runtime_errors_on_unreachable_host_root() {
        // register_worker returns immediately; connect() fires a background thread
        // that silently retries but never panics or blocks this thread.
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59599", iii_sdk::InitOptions::default());
        let mut cfg = ShellConfig::default();
        cfg.fs.host_roots = vec![std::path::PathBuf::from("/nonexistent/shell-jail-xyz")];
        cfg.fs.allow_unjailed = false;
        let res = build_runtime(&cfg, &iii);
        assert!(res.is_err(), "build_runtime must return Err, not panic");
    }

    #[test]
    fn prepare_config_rejects_unjailed_default() {
        let err = prepare_config(&ShellConfig::default()).expect_err("default is unsafe");
        assert!(err.contains("fs jail"));
    }

    #[test]
    fn prepare_config_accepts_seed_default() {
        // The zero-config seed must boot (jailed to /tmp) — that is the whole
        // point of seeding it instead of the unjailed Default::default().
        prepare_config(&ShellConfig::seed_default()).expect("seed_default boots");
    }

    #[test]
    fn prepare_config_accepts_pinned_host_roots() {
        let mut c = ShellConfig::default();
        c.fs.host_roots = vec![std::path::PathBuf::from("/tmp/shell")];
        prepare_config(&c).expect("pinned host_roots is valid");
    }

    #[test]
    fn legacy_host_root_config_is_rejected_with_hint() {
        // INVERSION of the old T6 regression test: through 0.6.x a single
        // fs.host_root was honored as a one-entry jail, and this test proved
        // that pre-merge shape still booted. 0.7.0 removes the alias outright,
        // so the SAME config shape must now FAIL CLOSED at parse with a hint
        // naming fs.host_roots — serde would otherwise ignore the stale key
        // and the worker would refuse to start with a message that never
        // names the actual mistake.
        let yaml = "allowlist: []\nfs:\n  host_root: /tmp/legacy-root\n";
        let err = ShellConfig::from_yaml(yaml).expect_err("the 0.6.x alias must be rejected");
        assert!(err.contains("removed in 0.7.0"), "{err}");
        assert!(err.contains("fs.host_roots"), "{err}");
    }

    #[test]
    fn prepare_config_rejects_bad_denylist() {
        let mut c = ShellConfig::default();
        c.fs.allow_unjailed = true;
        c.denylist_patterns = vec!["(".into()]; // invalid regex
        let err = prepare_config(&c).expect_err("bad regex rejected");
        assert!(err.contains("denylist"));
    }

    #[test]
    fn build_runtime_accepts_shipped_collect_config() {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/config.collect.yaml");
        let cfg = ShellConfig::from_file(path).expect("config.collect.yaml parses");
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59598", iii_sdk::InitOptions::default());
        build_runtime(&cfg, &iii)
            .expect("config.collect.yaml must boot for CI interface collection");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_reloads_converge_to_newest_config() {
        // Guards the reload-path TOCTOU: an OLDER event with a slow fetch/build
        // must NOT clobber a NEWER applied config. Two existing jail dirs so
        // build_runtime succeeds for both; we assert the newest one wins.
        //
        // Scope: this exercises the reload_lock serialization mechanics (the
        // newer reload blocks until the older one finishes, then applies last)
        // with injected configs. It does NOT exercise the live configuration::get
        // contract — that the in-lock re-fetch returns the latest stored value is
        // a property of the configuration worker, not of this serialization.
        let dir_old = std::env::temp_dir().join("shell-reload-old-9b3c");
        let dir_new = std::env::temp_dir().join("shell-reload-new-9b3c");
        std::fs::create_dir_all(&dir_old).unwrap();
        std::fs::create_dir_all(&dir_new).unwrap();

        let iii = iii_sdk::register_worker("ws://127.0.0.1:59597", iii_sdk::InitOptions::default());
        let mut base = ShellConfig::default();
        base.fs.host_roots = vec![dir_old.clone()];
        let initial = build_runtime(&base, &iii).expect("initial runtime");
        let state = AppState {
            runtime: Arc::new(RwLock::new(initial)),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };

        let mut cfg_old = ShellConfig::default();
        cfg_old.fs.host_roots = vec![dir_old.clone()];
        let mut cfg_new = ShellConfig::default();
        cfg_new.fs.host_roots = vec![dir_new.clone()];

        // OLDER reload: acquires the reload lock first, then stalls inside the
        // (lock-held) fetch — simulating a slow build for an older event.
        let s1 = state.clone();
        let old = cfg_old.clone();
        let h1 = tokio::spawn(async move {
            let _ = reload_serialized(&s1, || async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                Ok::<_, String>(old.to_json())
            })
            .await;
        });

        // Give h1 time to take the reload lock before the newer reload starts.
        tokio::time::sleep(std::time::Duration::from_millis(40)).await;

        // NEWER reload: dispatched later, fetches immediately. With the fix it
        // blocks on the lock until the older reload finishes, then applies last.
        let s2 = state.clone();
        let new = cfg_new.clone();
        let h2 = tokio::spawn(async move {
            let _ = reload_serialized(&s2, || async move { Ok::<_, String>(new.to_json()) }).await;
        });

        h1.await.unwrap();
        h2.await.unwrap();

        let final_root = state.runtime.read().await.config.fs.host_roots.clone();
        assert_eq!(
            final_root,
            vec![dir_new],
            "newest config must win; a stale older build must not clobber it"
        );
    }

    #[tokio::test]
    async fn reload_propagates_transient_fetch_failure() {
        // A transient fetch failure (configuration::get timing out) must NOT be
        // acked: reload_serialized returns Err and the runtime is unchanged.
        let dir = std::env::temp_dir().join("shell-reload-fetchfail-9b3c");
        std::fs::create_dir_all(&dir).unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59596", iii_sdk::InitOptions::default());
        let mut base = ShellConfig::default();
        base.fs.host_roots = vec![dir.clone()];
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&base, &iii).expect("initial"))),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };
        let res = reload_serialized(&state, || async {
            Err::<Value, String>("get timed out".into())
        })
        .await;
        assert!(
            res.is_err(),
            "transient fetch failure must surface as Err, not ack success"
        );
        assert_eq!(
            state.runtime.read().await.config.fs.host_roots,
            vec![dir],
            "runtime unchanged on fetch failure"
        );
        // A transient fetch failure is NOT a config rejection: status untouched.
        {
            let s = state.reload_status.read().await;
            assert_eq!(s.last_outcome, ReloadOutcome::Applied);
            assert_eq!(s.rejected_reloads, 0);
        }
    }

    #[tokio::test]
    async fn reload_acks_and_keeps_last_good_on_invalid_config() {
        // An invalid fetched config (unjailed default, rejected by prepare_config)
        // must keep last-good AND return Ok — returning Err would retry-storm on a
        // value re-fetching cannot fix.
        let dir = std::env::temp_dir().join("shell-reload-invalid-9b3c");
        std::fs::create_dir_all(&dir).unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59595", iii_sdk::InitOptions::default());
        let mut good = ShellConfig::default();
        good.fs.host_roots = vec![dir.clone()];
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&good, &iii).expect("initial"))),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };
        // ShellConfig::default() is unjailed (empty host_roots, allow_unjailed false) → rejected by prepare_config.
        let res = reload_serialized(&state, || async {
            Ok::<_, String>(ShellConfig::default().to_json())
        })
        .await;
        assert!(
            matches!(res, Ok(ReloadOutcome::Rejected)),
            "invalid config must be acked as Rejected (no retry storm), not Err"
        );
        assert_eq!(
            state.runtime.read().await.config.fs.host_roots,
            vec![dir],
            "runtime keeps last-good on invalid config"
        );
    }

    /// A stored value that FAILS TO PARSE (e.g. a 0.6.x shape carrying the
    /// removed `inherit_env`/`allowed_env` keys) must classify the same as an
    /// unbuildable-but-parseable config: `Rejected`, keep last-good, ack (no
    /// retry storm), and record the rejection for `shell::config-status`.
    /// Before the fix this hit the transient-fetch-failure branch instead —
    /// re-fetching returns the identical bytes, so the dispatcher would retry
    /// forever against a value that can never parse.
    #[tokio::test]
    async fn reload_rejects_and_records_unparseable_stored_value() {
        let dir = std::env::temp_dir().join("shell-reload-unparseable-9b3c");
        std::fs::create_dir_all(&dir).unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59593", iii_sdk::InitOptions::default());
        let mut good = ShellConfig::default();
        good.fs.host_roots = vec![dir.clone()];
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&good, &iii).expect("initial"))),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };
        // A literal 0.6.x stored shape: top-level inherit_env/allowed_env,
        // no `env` block — fails check_removed_keys inside parse_fetched_value.
        let stale_060_shape = serde_json::json!({
            "inherit_env": true,
            "allowed_env": ["PATH", "HOME"],
        });
        let res =
            reload_serialized(&state, || async move { Ok::<_, String>(stale_060_shape) }).await;
        assert!(
            matches!(res, Ok(ReloadOutcome::Rejected)),
            "unparseable stored value must be Rejected (permanent), not Err (transient): {res:?}"
        );
        assert_eq!(
            state.runtime.read().await.config.fs.host_roots,
            vec![dir],
            "runtime keeps last-good on an unparseable stored value"
        );
        let s = state.reload_status.read().await;
        assert_eq!(s.last_outcome, ReloadOutcome::Rejected);
        assert!(
            s.last_error
                .as_deref()
                .is_some_and(|e| e.contains("removed in 0.7.0")),
            "config-status must surface the migration hint: {:?}",
            s.last_error
        );
        assert_eq!(s.rejected_reloads, 1);
    }

    #[tokio::test]
    async fn reload_applies_newly_fetched_config() {
        // The reconcile primitive: a valid newly-fetched config is applied and Ok'd.
        let dir_a = std::env::temp_dir().join("shell-reload-applya-9b3c");
        let dir_b = std::env::temp_dir().join("shell-reload-applyb-9b3c");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59594", iii_sdk::InitOptions::default());
        let mut a = ShellConfig::default();
        a.fs.host_roots = vec![dir_a.clone()];
        let mut b = ShellConfig::default();
        b.fs.host_roots = vec![dir_b.clone()];
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&a, &iii).expect("initial"))),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };
        let res = reload_serialized(&state, {
            let b = b.clone();
            move || async move { Ok::<_, String>(b.to_json()) }
        })
        .await;
        assert!(matches!(res, Ok(ReloadOutcome::Applied)));
        assert_eq!(
            state.runtime.read().await.config.fs.host_roots,
            vec![dir_b],
            "reconcile/reload applies the freshly fetched config"
        );
    }

    #[tokio::test]
    async fn reload_applies_coder_config_and_resolver_cells() {
        let dir_a = std::env::temp_dir().join("shell-reload-code-a-9b3c");
        let dir_b = std::env::temp_dir().join("shell-reload-code-b-9b3c");
        std::fs::create_dir_all(&dir_a).unwrap();
        std::fs::create_dir_all(&dir_b).unwrap();
        std::fs::write(dir_b.join("new.txt"), "ok").unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59591", iii_sdk::InitOptions::default());
        let mut a = ShellConfig::default();
        a.fs.host_roots = vec![dir_a.clone()];
        let mut b = ShellConfig::default();
        b.fs.host_roots = vec![dir_b.clone()];
        let cells = build_code_cells(&a).expect("initial code cells");
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&a, &iii).expect("initial"))),
            code_cells: Some(cells.clone()),
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };

        let before = cells.resolver.read().await.clone();
        assert_eq!(before.roots(), &[std::fs::canonicalize(&dir_a).unwrap()]);

        let res = reload_serialized(&state, {
            let b = b.clone();
            move || async move { Ok::<_, String>(b.to_json()) }
        })
        .await;
        assert!(matches!(res, Ok(ReloadOutcome::Applied)));

        let after = cells.resolver.read().await.clone();
        assert_eq!(after.roots(), &[std::fs::canonicalize(&dir_b).unwrap()]);
        assert!(
            after.resolve("new.txt").is_ok(),
            "coder resolver must see the newly reloaded shell root"
        );
        assert_eq!(cells.config.read().await.base_paths, vec![dir_b]);
    }

    #[tokio::test]
    async fn reload_status_tracks_rejection_then_recovery() {
        // Finding 2: a rejected (unbuildable) config keeps last-good but must be
        // VISIBLE — recorded as Rejected with a reason and a cumulative count;
        // a later valid reload flips back to Applied but preserves the count.
        let dir = std::env::temp_dir().join("shell-reload-status-9b3c");
        std::fs::create_dir_all(&dir).unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59592", iii_sdk::InitOptions::default());
        let mut good = ShellConfig::default();
        good.fs.host_roots = vec![dir.clone()];
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&good, &iii).expect("initial"))),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };

        // Initial status is Applied with no rejections.
        {
            let s = state.reload_status.read().await;
            assert_eq!(s.last_outcome, ReloadOutcome::Applied);
            assert_eq!(s.rejected_reloads, 0);
        }

        // A rejected (invalid: unjailed default) reload keeps last-good AND records it.
        let res = reload_serialized(&state, || async {
            Ok::<_, String>(ShellConfig::default().to_json())
        })
        .await;
        assert!(res.is_ok(), "invalid config is acked (no storm)");
        {
            let s = state.reload_status.read().await;
            assert_eq!(s.last_outcome, ReloadOutcome::Rejected);
            assert!(s.last_error.is_some());
            assert_eq!(s.rejected_reloads, 1);
        }
        // Runtime kept the previous (valid) policy.
        assert_eq!(
            state.runtime.read().await.config.fs.host_roots,
            vec![dir.clone()]
        );

        // A subsequent valid reload flips back to Applied but preserves the count.
        let mut good2 = ShellConfig::default();
        good2.fs.host_roots = vec![dir.clone()];
        let res = reload_serialized(&state, {
            let g = good2.clone();
            move || async move { Ok::<_, String>(g.to_json()) }
        })
        .await;
        assert!(res.is_ok());
        {
            let s = state.reload_status.read().await;
            assert_eq!(s.last_outcome, ReloadOutcome::Applied);
            assert_eq!(s.last_error, None);
            assert_eq!(
                s.rejected_reloads, 1,
                "cumulative rejection count is preserved across recovery"
            );
        }
    }

    #[test]
    fn reload_status_serializes_to_documented_shape() {
        // shell::config-status publishes this shape via response_format; pin the
        // operator/automation contract (snake_case outcome + the three fields) so
        // it cannot drift silently.
        let mut s = ReloadStatus::default();
        let v = serde_json::to_value(&s).expect("serialize");
        assert_eq!(v["last_outcome"], "applied");
        assert_eq!(v["last_error"], serde_json::Value::Null);
        assert_eq!(v["rejected_reloads"], 0);

        s.record_rejected("host backend init: boom".into());
        let v = serde_json::to_value(&s).expect("serialize");
        assert_eq!(v["last_outcome"], "rejected");
        assert_eq!(v["last_error"], "host backend init: boom");
        assert_eq!(v["rejected_reloads"], 1);
    }

    #[tokio::test]
    async fn boot_reconcile_fails_closed_on_invalid_or_unreachable_config() {
        // F1: at boot, an invalid OR unfetchable authoritative config must abort
        // startup (fail-closed) — NOT keep last-good like a steady-state reload.
        // A valid config reconciles cleanly.
        let dir = std::env::temp_dir().join("shell-reconcile-failclosed-9b3c");
        std::fs::create_dir_all(&dir).unwrap();
        let iii = iii_sdk::register_worker("ws://127.0.0.1:59591", iii_sdk::InitOptions::default());
        let mut good = ShellConfig::default();
        good.fs.host_roots = vec![dir.clone()];
        let state = AppState {
            runtime: Arc::new(RwLock::new(build_runtime(&good, &iii).expect("initial"))),
            code_cells: None,
            iii: iii.clone(),
            reload_lock: Arc::new(Mutex::new(())),
            reload_status: Arc::new(RwLock::new(ReloadStatus::default())),
        };

        // Invalid (unjailed default) authoritative config → fail closed.
        let res = reconcile_with(&state, || async {
            Ok::<_, String>(ShellConfig::default().to_json())
        })
        .await;
        assert!(
            res.is_err(),
            "boot reconcile must fail closed on an invalid stored config"
        );

        // Transient fetch failure → fail closed too.
        let res = reconcile_with(&state, || async {
            Err::<Value, String>("get timed out".into())
        })
        .await;
        assert!(
            res.is_err(),
            "boot reconcile must fail closed on a transient fetch failure"
        );

        // A valid config reconciles cleanly.
        let mut good2 = ShellConfig::default();
        good2.fs.host_roots = vec![dir.clone()];
        let res = reconcile_with(&state, {
            let g = good2.clone();
            move || async move { Ok::<_, String>(g.to_json()) }
        })
        .await;
        assert!(res.is_ok(), "boot reconcile applies a valid config");
    }
}
