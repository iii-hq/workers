//! Integration with the `configuration` worker — register, fetch, and hot-reload
//! the `database` configuration entry.

use crate::config::WorkerConfig;
use crate::handlers::AppState;
use crate::pool::{self, Pool};
use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{Deliver, IIIClient, RegisterFunction, Source};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::time::Duration;

pub const DEFAULT_CONFIG_ID: &str = "database";

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
const CONFIG_FN_ID: &str = "database::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

/// Register the schema. The optional seed or built-in default is installed
/// only when no stored value exists; live configuration always takes precedence.
pub async fn register_config(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": config_id(),
        "name": "Database",
        "description": "Connection pools for PostgreSQL, MySQL, and SQLite.",
        "schema": WorkerConfig::json_schema(),
        "metadata": { "ui_form": DEFAULT_CONFIG_ID },
    });
    // The candidate (seed, else the built-in default) is forwarded
    // unconditionally: `configuration::ensure` installs it atomically ONLY
    // against an absent/null entry, so a stored operator/Compose override is
    // preserved without a client-side read-then-register race.
    payload["initial_value"] = seed
        .map(|value| value.to_json())
        .unwrap_or_else(|| WorkerConfig::default().to_json());
    ensure_configuration(iii, payload).await
}

/// Initialize atomically when supported, otherwise use the warned legacy path.
async fn ensure_configuration(iii: &IIIClient, payload: serde_json::Value) -> Result<(), String> {
    initialization::ensure_with(payload, |function, payload| {
        trigger_configuration_with_retry(iii, function, payload)
    })
    .await
}

#[path = "../../crates/config-client/src/initialization.rs"]
mod initialization;

/// Read the live `database` configuration (env-expanded by the configuration worker).
pub async fn fetch_config(iii: &IIIClient) -> Result<WorkerConfig, String> {
    let value = get_config_value(iii).await?;
    if value.is_null() {
        tracing::info!("no configuration value found; using built-in default configuration");
        return Ok(WorkerConfig::default());
    }
    WorkerConfig::from_json_isolating(&value)
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

/// The stored value as written (`raw: true`): `${VAR}` templates and
/// `secret://` references unexpanded. `Value::Null` when nothing is stored.
pub async fn get_raw_value(iii: &IIIClient) -> Result<Value, String> {
    match trigger_configuration_with_retry(
        iii,
        "configuration::get",
        json!({ "id": config_id(), "raw": true }),
    )
    .await
    {
        Ok(resp) => Ok(resp.get("value").cloned().unwrap_or(Value::Null)),
        Err(e) if is_not_found(&e) => Ok(Value::Null),
        Err(e) => Err(e),
    }
}

/// Replace the stored value (validated against the registered schema by the
/// configuration worker, which then notifies every consumer).
pub async fn set_value(iii: &IIIClient, value: Value) -> Result<(), String> {
    // One attempt: a rejected value (schema) must surface at once, and the
    // caller reports a transport failure as such.
    iii.trigger(
        TriggerRequest {
            function_id: "configuration::set".to_string(),
            payload: json!({ "id": config_id(), "value": value }),
            action: None,
            timeout_ms: Some(CONFIG_TIMEOUT_MS),
        }
        .namespace("default"),
    )
    .await
    .map(|_| ())
    .map_err(|e| e.to_string())
}

/// Build connection pools for every configured database from literal values
/// (references fail), all-or-nothing. Tests and tools; the worker goes
/// through [`apply_config`], which resolves references and isolates a
/// failing connection from the rest.
pub async fn build_pools(cfg: &WorkerConfig) -> Result<HashMap<String, Pool>, String> {
    let mut pools = HashMap::new();
    for (name, db) in &cfg.databases {
        let p = pool::build(name, db)
            .await
            .map_err(|e| serde_json::to_string(&e).unwrap_or_else(|_| e.to_string()))?;
        tracing::info!(db = %name, driver = ?p.driver(), "pool ready");
        pools.insert(name.clone(), p);
    }
    Ok(pools)
}

/// Make `cfg` live: resolve its references, (re)build the pools whose entry
/// or resolved secrets changed, keep the others as they are, and swap the
/// snapshot. A connection that cannot be built (unresolvable reference, SSH
/// failure, unreadable certificate) gets no pool and an error that
/// `listDatabases` reports and its calls return; every other connection
/// keeps working. Never fails as a whole: `cfg` was validated when parsed.
pub async fn apply_config(
    state: &AppState,
    cfg: WorkerConfig,
    listeners: Option<&crate::triggers::NativeListeners>,
) -> Result<(), String> {
    use crate::connector::{self, Connector, Material, Stage};
    use std::collections::BTreeSet;
    use std::sync::Arc;

    let runtime = state.runtime.clone();
    let _reload = runtime.reload.lock().await;

    let references: BTreeSet<String> = cfg
        .databases
        .values()
        .flat_map(connector::references)
        .collect();
    runtime.secrets.ensure(references.iter().cloned()).await;
    runtime.secrets.retain(&references);

    let current_pools = state.pools.read().await.clone();
    let previous_errors = runtime.errors();
    let builds = cfg.databases.iter().map(|(name, db)| {
        let runtime = runtime.clone();
        let current = current_pools
            .get(name)
            .cloned()
            .zip(runtime.connector(name));
        async move {
            let material = Material::resolve(db, |key| runtime.secrets.lookup(key))?;
            let fingerprint = connector::fingerprint(db, &material);
            if let Some((pool, built_from)) = current {
                if built_from.fingerprint == fingerprint {
                    return Ok((pool, built_from));
                }
            }
            let connector = Connector::build(name, db, &material)?;
            connector.check_tunnel().await?;
            let connector = Arc::new(connector);
            let pool = pool::from_connector(name, connector.clone(), &db.pool).map_err(|e| {
                let message = match e {
                    crate::error::DbError::ConfigError { message } => message,
                    other => other.to_string(),
                };
                connector::ConnectError::new(Stage::Config, message)
            })?;
            tracing::info!(db = %name, driver = ?pool.driver(), "pool ready");
            Ok::<_, connector::ConnectError>((pool, connector))
        }
    });
    let names: Vec<String> = cfg.databases.keys().cloned().collect();
    let results = futures_util::future::join_all(builds).await;

    let mut new_pools = HashMap::new();
    let mut connectors = HashMap::new();
    let mut errors = HashMap::new();
    for (name, why) in &cfg.invalid {
        if previous_errors.get(name).map(|p| &p.message) != Some(why) {
            tracing::warn!(db = %name, error = %why, "connection entry invalid; skipped");
        }
        errors.insert(
            name.clone(),
            connector::ConnectError::new(Stage::Config, why.clone()),
        );
    }
    for (name, result) in names.into_iter().zip(results) {
        match result {
            Ok((pool, connector)) => {
                new_pools.insert(name.clone(), pool);
                connectors.insert(name, connector);
            }
            Err(e) => {
                // Only a new or changed failure is worth a log line: the
                // maintenance tick retries failed connections.
                if previous_errors.get(&name).map(|p| &p.message) != Some(&e.message) {
                    tracing::warn!(db = %name, stage = ?e.stage, error = %e.message, "connection unavailable");
                }
                errors.insert(name, e);
            }
        }
    }

    let registered = runtime.listeners();
    if let Some(listeners) = listeners.or(registered.as_deref()) {
        listeners.sync_with(&cfg, &connectors);
    }
    // Swap pools and the config snapshot inside one critical section (pools
    // lock first, then config) so a concurrent reader never observes new
    // pools paired with the old config or vice-versa.
    let mut pools_guard = state.pools.write().await;
    let mut config_guard = state.config.write().await;
    *pools_guard = new_pools;
    *config_guard = cfg;
    runtime.replace(errors, connectors);
    drop(config_guard);
    drop(pools_guard);
    // Observers the new config no longer supports (their database is gone)
    // get a late rejection: their `ready()` side moves to `error`.
    if let Some(bus) = &state.row_changes {
        bus.capture().revalidate().await;
    }
    Ok(())
}

/// How often the maintenance tick looks for failed connections to retry and
/// secret values past their TTL.
pub const MAINTENANCE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(15);

/// Retry failed connections (a secrets worker that came up late, a bastion
/// back online) and re-read secrets past their TTL; a value that changed
/// rebuilds only its connection's pool.
pub fn spawn_maintenance(
    state: AppState,
    listeners: Option<std::sync::Arc<crate::triggers::NativeListeners>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(MAINTENANCE_INTERVAL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        tick.tick().await;
        loop {
            tick.tick().await;
            if !(state.runtime.has_errors() || state.runtime.secrets.any_stale()) {
                continue;
            }
            let cfg = state.config.read().await.clone();
            let _ = apply_config(&state, cfg, listeners.as_deref()).await;
        }
    })
}

const SECRET_FN_ID: &str = "database::on-secret-change";

/// Register the internal `secrets::changed` handler and bind it. A rotated,
/// deleted or re-shared secret that a connection references is re-read, and
/// only the connections whose resolved values changed are rebuilt. The
/// payload is advisory (it never carries a value) and anything unrelated is
/// ignored, so a direct call can at most cause a re-read.
/// A `secrets::changed` delivery for `reference` (canonical). Whatever uses
/// it, its cached value is dropped; when a stored connection uses it, it is
/// re-read now and the connections whose resolved values changed are rebuilt
/// (deleted → those connections report the error; re-created → they come
/// back).
pub async fn on_secret_changed(
    state: &AppState,
    reference: &str,
    listeners: Option<&crate::triggers::NativeListeners>,
) {
    let cfg = state.config.read().await.clone();
    let referenced = cfg.databases.values().any(|db| {
        crate::connector::references(db)
            .iter()
            .any(|r| r == reference)
    });
    if !referenced {
        state.runtime.secrets.forget(reference);
        return;
    }
    state
        .runtime
        .secrets
        .refresh([reference.to_string()], crate::secrets::Refetch::Invalidate)
        .await;
    let _ = apply_config(state, cfg, listeners).await;
}

pub fn register_secret_trigger(
    iii: &IIIClient,
    state: AppState,
    listeners: Option<std::sync::Arc<crate::triggers::NativeListeners>>,
) {
    let st = state.clone();
    iii.register_function(
        SECRET_FN_ID,
        RegisterFunction::new_async(move |event: OnSecretChangeEvent| {
            let st = st.clone();
            let listeners = listeners.clone();
            async move {
                let payload = json!({ "name": event.name, "ref": event.reference });
                let Some(reference) = crate::secrets::event_reference(&payload) else {
                    return Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse {
                        ok: false,
                    });
                };
                on_secret_changed(&st, &reference, listeners.as_deref()).await;
                Ok(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: re-read a rotated or revoked secret:// / env:// value and rebuild the \
             connections that reference it.",
        )
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
    // `secrets::changed` belongs to the secrets worker: when it is absent the
    // engine parks this binding and activates it once the type registers.
    #[allow(deprecated)]
    if let Err(e) = iii.register_trigger(RegisterTriggerInput::new(
        crate::secrets::CHANGED_TRIGGER_TYPE.to_string(),
        SECRET_FN_ID.to_string(),
        json!({}),
    )) {
        tracing::warn!(error = %e, "binding secrets::changed failed; secret references refresh on their TTL only");
    }
}

/// Advisory `secrets::changed` delivery (never carries a value).
#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
pub struct OnSecretChangeEvent {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default, rename = "ref")]
    pub reference: Option<String>,
    /// `created` | `rotated` | `deleted` | `access_changed` (advisory).
    #[serde(default)]
    #[allow(dead_code)]
    pub action: Option<String>,
}

/// Event delivered to the internal `database::on-config-change` handler. A struct
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

/// Ack returned by the internal `database::on-config-change` handler.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct OnConfigChangeResponse {
    pub ok: bool,
}

/// Register the internal config-change handler. The returned binding hears
/// the changes: [`ConfigBinding::follow`] picks the way the engine on the
/// current connection serves them.
pub fn register_config_trigger(
    iii: &IIIClient,
    state: AppState,
    listeners: Option<std::sync::Arc<crate::triggers::NativeListeners>>,
) -> ConfigBinding {
    let st = state.clone();
    let engine = iii.clone();
    iii.register_function(
        CONFIG_FN_ID,
        RegisterFunction::new_async(move |_event: OnConfigChangeEvent| {
            let st = st.clone();
            let engine = engine.clone();
            let listeners = listeners.clone();
            async move {
                on_config_change(&engine, &st, listeners.as_deref()).await;
                Ok::<OnConfigChangeResponse, Error>(OnConfigChangeResponse { ok: true })
            }
        })
        .description(
            "Internal: reload connection pools from the authoritative configuration when it changes.",
        )
        .metadata(json!({ "internal": true })),
    );
    ConfigBinding {
        iii: iii.clone(),
        observer: std::sync::Mutex::new(None),
        trigger: std::sync::Mutex::new(None),
    }
}

/// How the worker hears its configuration entry change: an observer of the
/// engine's `configuration::on-updated` on an engine with observe, the old
/// `configuration` trigger on one without.
pub struct ConfigBinding {
    iii: IIIClient,
    observer: std::sync::Mutex<Option<iii_sdk::ObserverHandle>>,
    trigger: std::sync::Mutex<Option<iii_sdk::trigger::Trigger>>,
}

impl ConfigBinding {
    /// Binds the way an engine that does (`true`) or does not (`false`)
    /// support observe serves configuration changes, and drops the other.
    /// Idempotent; called again when a reconnect lands on an engine of the
    /// other kind (an upgrade or a rollback under the running worker).
    pub fn follow(&self, engine_observes: bool) {
        let mut observer = self.observer.lock().unwrap_or_else(|e| e.into_inner());
        let mut trigger = self.trigger.lock().unwrap_or_else(|e| e.into_inner());
        if engine_observes {
            if observer.is_none() {
                *observer = Some(
                    self.iii.observe(
                        Source::function("configuration::on-updated")
                            .namespace("default")
                            .filters(json!({
                                "input.id": { "$eq": config_id() },
                                "input.event_type": { "$eq": "configuration:updated" },
                            }))
                            // The event's input carries the whole configuration value
                            // (url passwords included), and whatever an activation
                            // ships lands in this worker's trace span. The filter reads
                            // the input; the activation ships only the caller — the
                            // handler re-fetches the value anyway.
                            .capture([iii_sdk::protocol::Capture::Caller])
                            .deliver(Deliver::Envelope),
                        CONFIG_FN_ID,
                    ),
                );
            }
            if let Some(old) = trigger.take() {
                old.unregister();
            }
            return;
        }
        // An observer made on an earlier engine stays: the SDK fails it with
        // `UNSUPPORTED` here and replays it to the next engine with observe.
        if trigger.is_none() {
            // Deprecated in 0.25; kept for engines without observe.
            #[allow(deprecated)]
            let bound = self.iii.register_trigger(RegisterTriggerInput::new(
                "configuration".to_string(),
                CONFIG_FN_ID.to_string(),
                json!({
                    "configuration_id": config_id(),
                    "event_types": ["configuration:updated"],
                }),
            ));
            match bound {
                Ok(bound) => *trigger = Some(bound),
                Err(e) => tracing::error!(
                    error = %e,
                    "binding the configuration trigger failed; configuration changes are not heard"
                ),
            }
        }
    }
}

/// Reload pools from the AUTHORITATIVE configuration.
///
/// The caller-supplied trigger payload is intentionally ignored:
/// `database::on-config-change` is a discoverable bus function, so trusting
/// `payload.new_value` would let any caller replace the live connection pools
/// (e.g. point them at an attacker-controlled database) without updating
/// persisted state. Re-fetch the stored value via `configuration::get` instead.
async fn on_config_change(
    iii: &IIIClient,
    state: &AppState,
    listeners: Option<&crate::triggers::NativeListeners>,
) {
    let cfg = match fetch_config(iii).await {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::error!(
                error = %e,
                "config-change: failed to fetch authoritative configuration; keeping previous pools"
            );
            return;
        }
    };
    match apply_config(state, cfg, listeners).await {
        Ok(()) => tracing::info!("database pools reloaded after configuration change"),
        Err(e) => tracing::error!(
            error = %e,
            "failed to rebuild pools after configuration change; keeping previous pools"
        ),
    }
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

    /// A hand-edited entry with an absurd pool size becomes that
    /// connection's error; the worker and the other connections carry on.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_bad_stored_entry_costs_only_its_connection() {
        use crate::handlers::AppState;
        let cfg = crate::config::WorkerConfig::from_json_isolating(&serde_json::json!({
            "databases": {
                "good": { "url": "sqlite::memory:" },
                "huge": { "url": "sqlite::memory:", "pool": { "max": 1_000_000_000u64 } }
            }
        }))
        .unwrap();
        let st = AppState {
            pools: Default::default(),
            config: std::sync::Arc::new(tokio::sync::RwLock::new(cfg.clone())),
            handles: std::sync::Arc::new(crate::handle::HandleRegistry::new()),
            transactions: crate::transaction::TxRegistry::new(),
            log: iii_helpers::observability::Logger::new(),
            row_changes: None,
            runtime: Default::default(),
        };
        super::apply_config(&st, cfg, None).await.unwrap();
        let ok = crate::handlers::query::handle(
            &st,
            serde_json::from_value(serde_json::json!({ "db": "good", "sql": "SELECT 1 AS one" }))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(ok.rows[0]["one"], 1);
        let err = crate::handlers::query::handle(
            &st,
            serde_json::from_value(serde_json::json!({ "db": "huge", "sql": "SELECT 1" })).unwrap(),
        )
        .await
        .unwrap_err();
        assert!(
            err.contains("CONFIG_ERROR") && err.contains("`pool.max`"),
            "{err}"
        );
        let listed = crate::handlers::list_databases::handle(&st, Default::default())
            .await
            .unwrap();
        let huge = listed.databases.iter().find(|d| d.name == "huge").unwrap();
        assert!(huge.error.as_deref().unwrap().contains("`pool.max`"));
        assert_eq!(listed.count, 2);
    }
}
