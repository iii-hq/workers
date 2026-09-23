//! Integration with the `configuration` worker — register, fetch, and
//! hot-reload the `iii-directory` configuration entry.
//!
//! Mirrors the `database` worker's pattern: register a JSON Schema + seed
//! at boot, read the authoritative (env-expanded) value via
//! `configuration::get`, and bind a `configuration` trigger so a
//! `configuration:updated` event re-fetches and applies the change.
//!
//! Tunable fields (`registry_url`, `download_timeout_ms`,
//! `registry_cache_ttl_ms`, `filter_unregistered`, `function_search_mode`) hot-reload in place;
//! topology fields (`skills_folder`, `local_skills_folder`, `agents_folder`,
//! `agents_skills_folder`, `auto_download`, `function_search_model_path`,
//! `function_search_model_download`) are refused with a "restart
//! required" log because they define on-disk roots and boot-time task
//! wiring.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{FunctionSearchMode, SharedConfig, SkillsConfig, Topology};
use crate::functions::registry::RegistryCache;
use crate::functions::skills::RegisteredWorkersCache;

pub const DEFAULT_CONFIG_ID: &str = "iii-directory";

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
const CONFIG_FN_ID: &str = "directory::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Runtime resources a `configuration:updated` reload must reach: the live
/// config snapshot handlers read, the shared cache-TTL cell, the two caches
/// to clear, and the immutable boot-time topology used to detect
/// restart-requiring changes.
#[derive(Clone)]
pub struct SharedState {
    /// Live tunable snapshot read by handlers via `load_full()`.
    pub config: SharedConfig,
    /// Shared TTL (ms) backing both caches; updated on reload.
    pub cache_ttl_ms: Arc<AtomicU64>,
    /// Registry HTTP-response cache; cleared on reload.
    pub registry_cache: RegistryCache,
    /// Installed-worker-name cache; invalidated on reload.
    pub registered_cache: Arc<RegisteredWorkersCache>,
    /// Restart-only fields captured at boot.
    boot_topology: Topology,
    /// Live `directory::pre-generate` hook binding, reconciled with the
    /// `inject_hint` knob on every reload.
    pub hint_binding: crate::hook::HintBindingState,
    pub search: crate::functions::search::Deps,
    /// Keep each reload's config snapshot and semantic activation together.
    apply_lock: Arc<tokio::sync::Mutex<()>>,
}

impl SharedState {
    pub fn new(
        config: SharedConfig,
        cache_ttl_ms: Arc<AtomicU64>,
        registry_cache: RegistryCache,
        registered_cache: Arc<RegisteredWorkersCache>,
        boot_topology: Topology,
        hint_binding: crate::hook::HintBindingState,
        search: crate::functions::search::Deps,
    ) -> Self {
        Self {
            config,
            cache_ttl_ms,
            registry_cache,
            registered_cache,
            boot_topology,
            hint_binding,
            search,
            apply_lock: Arc::default(),
        }
    }
}

/// Register the `iii-directory` configuration schema. The candidate
/// `initial_value` (the `--config` seed, else built-in defaults) is forwarded
/// unconditionally to `configuration::ensure`, which installs it atomically
/// ONLY against an absent/null entry — a stored value (console Settings,
/// `configuration::set`) is preserved without a client-side
/// read-then-register race.
pub async fn register_config(iii: &IIIClient, seed: Option<&SkillsConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "iii-directory",
        "description": "Skills and agent-skills folders, workers-registry URL, download timeouts, \
                        skill-visibility filters, and the function-search knobs \
                        (inject_hint, hint_min_workers, registry_search, function_search_mode, \
                        function_search_model_path, function_search_judge_timeout_ms, \
                        function_search_judge_min_relevance, \
                        function_search_judge_side_lane_min_relevance, \
                        function_search_judge_question, \
                        function_search_judge_choice_min_probability) for the \
                        iii-directory worker.",
        "schema": SkillsConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = initial_value(seed);
    ensure_configuration(iii, payload).await
}

/// The `initial_value` candidate for `configuration::ensure`: the `--config`
/// seed, else the built-in defaults. Always forwarded; the engine installs it
/// only when nothing is stored yet, so runtime edits survive atomically.
fn initial_value(seed: Option<&SkillsConfig>) -> Value {
    seed.map_or_else(|| SkillsConfig::default().to_json(), SkillsConfig::to_json)
}

/// Read the live `iii-directory` configuration (env-expanded by the
/// configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<SkillsConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(SkillsConfig::default());
    }
    SkillsConfig::from_json(&value)
}

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    iii_config_client::initialization::ensure_with(payload, |function, payload| {
        trigger_configuration_with_retry(iii, function, payload)
    })
    .await
}

async fn get_config_value(iii: &IIIClient) -> Result<Value, String> {
    try_get_config_value(iii).await?.ok_or_else(|| {
        format!(
            "configuration `{config_entry}` not found",
            config_entry = config_id()
        )
    })
}

/// Returns `Ok(None)` when the entry does not exist (`NOT_FOUND`).
async fn try_get_config_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if is_not_found(&e) => Ok(None),
        Err(e) => Err(e),
    }
}

/// Apply a freshly-fetched configuration: swap the live snapshot, update the
/// shared cache TTL, and clear both caches so a repointed `registry_url`
/// takes effect immediately and stale entries from the old registry drop.
pub async fn apply_config(state: &SharedState, cfg: SkillsConfig) {
    let _apply = state.apply_lock.lock().await;
    let activate_semantic = cfg.function_search_mode != FunctionSearchMode::Lexical
        && state.config.load().function_search_mode == FunctionSearchMode::Lexical;
    state
        .search
        .semantic
        .set_enabled(cfg.function_search_mode != FunctionSearchMode::Lexical);
    state
        .cache_ttl_ms
        .store(cfg.registry_cache_ttl_ms, Ordering::Relaxed);
    state.config.store(Arc::new(cfg));
    if activate_semantic {
        // A mode change may leave the catalog fingerprint unchanged. Rebuild
        // explicitly instead of waiting for a functions-available event.
        // Hold the catalog lock until rebuild records its desired fingerprint,
        // so a newer refresh cannot be superseded by this snapshot.
        let tools = state.search.catalog.read().await;
        state.search.semantic.rebuild(tools.clone());
    }
    state.registry_cache.clear().await;
    state.registered_cache.invalidate().await;
}

/// Trigger payload for `directory::on-config-change`. The handler ignores it
/// (it re-fetches from the authoritative configuration); the empty struct
/// exists so the function publishes a typed request schema rather than AnyValue.
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeRequest {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeResponse {
    ok: bool,
}

/// Register the internal config-change handler and bind a `configuration`
/// trigger for `configuration:updated` on the `iii-directory` entry.
pub fn register_config_trigger(iii: &IIIClient, state: SharedState) -> Result<(), Error> {
    let st = state.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_req: OnConfigChangeRequest| {
            let st = st.clone();
            let engine = engine.clone();
            async move {
                on_config_change(&engine, &st).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload tunable iii-directory settings from the authoritative \
             configuration when it changes.",
        )
        .metadata(json!({ "internal": true })),
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
/// `directory::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller repoint the registry URL or
/// download roots without updating persisted state. Re-fetch the stored
/// value via `configuration::get` instead. Topology changes are refused —
/// the on-disk roots and the auto-download wiring are fixed at boot.
async fn on_config_change(iii: &IIIClient, state: &SharedState) {
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
    if cfg.topology() != state.boot_topology {
        tracing::warn!(
            "configuration change alters topology (skills_folder, local_skills_folder, \
             agents_folder, agents_skills_folder, auto_download, \
             function_search_model_path, or function_search_model_download); a restart is required \
             to apply it — keeping previous configuration"
        );
        return;
    }
    let inject_hint = cfg.inject_hint;
    let model_path = cfg.resolved_function_search_model_path();
    crate::config::warn_if_search_mode_lacks_model(
        cfg.function_search_mode,
        model_path.is_some(),
        model_path
            .as_deref()
            .is_some_and(crate::functions::search_semantic::bundle_complete),
    );
    apply_config(state, cfg).await;
    // Reconcile the pre-generate hook binding with the reloaded knob (hot,
    // no restart): on → bind once; off → unregister.
    crate::hook::apply(iii, &state.hint_binding, inject_hint);
    tracing::info!("iii-directory configuration reloaded (tunable fields applied; caches cleared)");
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
                if matches!(&e, iii_sdk::errors::Error::Remote { code, .. } if code == "function_not_found" || code == "NOT_FOUND")
                {
                    return Err(last_err);
                }
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

/// `true` only when the error carries the configuration worker's standalone
/// `NOT_FOUND` entry code, identified by the outermost `remote error (<code>)` envelope code rather than a substring or token scan of the message, so
/// a compound code such as `RESOURCE_NOT_FOUND`/`STATEMENT_NOT_FOUND` or the
/// engine's lowercase missing-FUNCTION code `function_not_found` still
/// propagates as a failure instead of being read as "nothing stored yet".
fn is_not_found(error: &str) -> bool {
    // Anchor on the SDK's own rendering instead of scanning the whole string:
    // a remote failure prints as `remote error ({code}): {message}`, and this
    // worker wraps a retried get as
    // `configuration::get failed after CONFIG_RETRIES attempts: {err}`. Peel
    // exactly that one wrapper (never a foreign one or a different attempt
    // count) and then require the NOT_FOUND envelope at the very start, so a
    // NOT_FOUND code buried in an unrelated message, a nested envelope, or a
    // different wrapper stays a real failure and propagates.
    const RETRY_WRAPPER: &str = "configuration::get failed after 3 attempts: ";
    const _: () = assert!(CONFIG_RETRIES == 3);
    let raw = error.trim();
    let raw = raw.strip_prefix(RETRY_WRAPPER).unwrap_or(raw);
    raw == "NOT_FOUND"
        || raw == "remote error (NOT_FOUND):"
        || raw.starts_with("remote error (NOT_FOUND): ")
}

#[cfg(test)]
mod tests {
    include!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../crates/config-client/tests/support/is_not_found_cases.rs"
    ));

    /// The missing-entry classifier only seeds on the configuration worker's
    /// standalone `NOT_FOUND` envelope; every unrelated failure or compound
    /// code propagates instead of clobbering a stored value with a default.
    #[test]
    fn is_not_found_matches_only_the_envelope_code() {
        assert_missing_entry_contract(super::is_not_found);
    }

    use super::*;

    #[test]
    fn candidate_initial_value_is_seed_else_default() {
        let seed = SkillsConfig {
            function_search_mode: FunctionSearchMode::Lexical,
            ..SkillsConfig::default()
        };
        // The candidate is always the seed (else the built-in default); the
        // engine's `configuration::ensure` decides seed-vs-preserve, not this.
        assert_eq!(initial_value(Some(&seed)), seed.to_json());
        assert_eq!(initial_value(None), SkillsConfig::default().to_json());
    }
}
