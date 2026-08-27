//! Integration with the builtin `configuration` worker. The `console` entry is
//! the source of truth for the HTTP listener port as well as Console UI
//! preferences and injectable-UI per-worker toggles.
//!
//! The local YAML/CLI port is a first-registration seed and a fallback when the
//! configuration worker is unavailable. Once stored, `http_port` is fetched
//! before the listener binds and is applied live on `configuration:updated`.
//! Port changes use the same bind-new-before-stop-old strategy as the HTTP
//! worker: a failed bind leaves both the previous port snapshot and listener
//! untouched; a successful bind starts the replacement before gracefully
//! draining the old listener.
//!
//! Configuration integration remains best-effort so a directly-run Console
//! can still serve when the configuration worker is disabled or unreachable.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};
use tokio::sync::RwLock;

use crate::server::{self, AppState, ServerControlCell};
use crate::ui_assets::UiControl;

/// Live port snapshot used by `console::status` and the rebind path.
pub type PortCell = Arc<RwLock<u16>>;

/// Serializes fetch → validate → bind → swap across overlapping update events.
pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;

/// The runtime-owned slice of the broader Console configuration entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeConfig {
    pub http_port: u16,
    disabled_workers: HashSet<String>,
}

impl RuntimeConfig {
    pub fn fallback(http_port: u16) -> Self {
        Self {
            http_port,
            disabled_workers: HashSet::new(),
        }
    }
}

pub fn new_port_cell(http_port: u16) -> PortCell {
    Arc::new(RwLock::new(http_port))
}

pub const DEFAULT_CONFIG_ID: &str = "console";

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
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// The `console` entry schema. `http_port` is strictly bounded to a TCP port;
/// the UI-owned preference sections stay deliberately permissive so their
/// detailed shapes can evolve without a console-worker redeploy. Only the
/// preference envelope — `traces.views[]` entries carrying `id` and `name` —
/// is pinned.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "http_port": {
                "type": "integer",
                "minimum": 0,
                "maximum": 65535,
                "default": 3113,
                "description": "TCP port for the Console UI, injected assets, and /ws proxy. Changes rebind the listener live."
            },
            "traces": {
                "type": "object",
                "description": "Traces V2 UI preferences.",
                "properties": {
                    "views": {
                        "type": "array",
                        "description": "Named saved views: grouping + filters + display settings.",
                        "items": {
                            "type": "object",
                            "properties": {
                                "id": { "type": "string" },
                                "name": { "type": "string" }
                            },
                            "required": ["id", "name"],
                            "additionalProperties": true
                        }
                    },
                    "activeViewId": {
                        "type": ["string", "null"],
                        "description": "Id of the selected view; null = the unfiltered all-traces list. When absent the UI selects the seeded sessions view."
                    },
                    "followTurns": {
                        "type": "boolean",
                        "description": "Auto-open the trace of the active chat's live turn. When absent the UI defaults to on."
                    }
                },
                "additionalProperties": true
            },
            "injectableUi": {
                "type": "object",
                "description": "Injectable worker UI controls (console:script / console:style assets).",
                "properties": {
                    "disabledWorkers": {
                        "type": "array",
                        "description": "Workers whose injected console UI is turned off. Their assets are held (not served or loaded in tabs) until removed from this list; changes apply live. The console itself cannot be disabled.",
                        "items": { "type": "string" }
                    }
                },
                "additionalProperties": true
            }
        },
        "additionalProperties": true
    })
}

/// Out-of-the-box port and preferences seeded when the entry has never been
/// configured.
///
/// - `views`: `view-sessions` groups traces by `iii.session.id` (stamped on
///   every harness turn span, so no trace is dropped for lacking it — the
///   engine's `group_by` skips spans without the grouping attribute, and
///   `iii.session.name` only exists once a session has a title) and labels
///   rows with the tag message; `activeViewId` selects it out of the box, and the
///   frontend falls back to the same id when the pointer is absent
///   (`DEFAULT_VIEW_ID` in web tracesViews.ts — keep the id in sync).
/// - `followTurns`: follow the active chat's live turn — on out of the box
///   (the frontend also defaults to on when the flag is absent).
/// - `spanFilters`: detail-view funnel defaults — hide the chatty
///   `harness::send` span group and the session/context bookkeeping workers.
fn default_value(http_port: u16) -> Value {
    json!({
        "http_port": http_port,
        "traces": {
            "views": [{
                "id": "view-sessions",
                "name": "sessions",
                "groupBy": "iii.session.id",
                "hiddenFunctions": [],
                "label": { "mode": "attribute", "attribute": "iii.tag.message" },
                "filters": {}
            }],
            "activeViewId": "view-sessions",
            "followTurns": true,
            "spanFilters": {
                "hiddenGroups": ["harness::send"],
                "hiddenWorkers": ["context-manager", "session-manager"]
            }
        },
        "injectableUi": {
            "disabledWorkers": []
        }
    })
}

/// Register the `console` configuration entry. `seed_http_port` is included in
/// `initial_value` only when no value is stored, so runtime edits survive
/// restarts. Callers intentionally treat errors as best-effort fallbacks.
pub async fn register_console_config(iii: &IIIClient, seed_http_port: u16) -> Result<(), String> {
    let existing = existing_value(iii)
        .await
        .map_err(|error| format!("console configuration lookup failed: {error}"))?;
    let seed = existing.is_none();
    // Entries created by older Console versions already contain preferences
    // but no port. Backfill the active local seed after the schema refresh so
    // the configuration form displays the listener's real value and future
    // restarts no longer depend on a possibly-changed local seed.
    let backfill = existing.as_ref().and_then(|value| {
        let mut value = value.as_object()?.clone();
        if value.contains_key("http_port") {
            return None;
        }
        value.insert("http_port".to_string(), json!(seed_http_port));
        Some(Value::Object(value))
    });

    let mut payload = json!({
        "id": config_id(),
        "name": "Console",
        "description": "Console server and UI settings — live HTTP port binding, \
                        Traces V2 saved views, and per-worker injectable-UI toggles.",
        "schema": schema(),
    });
    if seed {
        payload["initial_value"] = default_value(seed_http_port);
    }

    trigger_configuration_with_retry(iii, "configuration::register", payload).await?;
    if let Some(value) = backfill {
        set_value(iii, value).await?;
        tracing::info!(
            id = config_id(),
            http_port = seed_http_port,
            "backfilled http_port in existing console configuration"
        );
    }
    tracing::info!(id = config_id(), "console configuration registered");
    Ok(())
}

const CONFIG_CHANGE_FN_ID: &str = "console::on-config-change";

/// Fetch the runtime-owned fields from the authoritative entry. A missing
/// `http_port` (including entries created by older Console versions) retains
/// the supplied seed/fallback port.
pub async fn fetch_runtime_config(
    iii: &IIIClient,
    fallback_http_port: u16,
) -> Result<RuntimeConfig, String> {
    let value = existing_value(iii).await?;
    runtime_config_from(value.as_ref(), fallback_http_port)
}

fn runtime_config_from(
    value: Option<&Value>,
    fallback_http_port: u16,
) -> Result<RuntimeConfig, String> {
    let http_port = match value.and_then(|value| value.get("http_port")) {
        None => fallback_http_port,
        Some(Value::Number(number)) => number
            .as_u64()
            .and_then(|port| u16::try_from(port).ok())
            .ok_or_else(|| {
                "stored `console.http_port` must be an integer from 0 to 65535".to_string()
            })?,
        Some(_) => {
            return Err("stored `console.http_port` must be an integer from 0 to 65535".to_string())
        }
    };

    Ok(RuntimeConfig {
        http_port,
        disabled_workers: value.map(disabled_workers_from).unwrap_or_default(),
    })
}

/// Apply the runtime UI slice fetched during boot.
pub async fn apply_runtime_ui(config: &RuntimeConfig, control: Option<&UiControl>) {
    if let Some(control) = control {
        control
            .set_disabled_workers(config.disabled_workers.clone())
            .await;
    }
}

/// Register the configuration update handler and subscription. Every delivery
/// re-fetches the authoritative value under `apply_lock`; the discoverable
/// handler never trusts caller-supplied payload data.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    port: PortCell,
    state: AppState,
    server_control: ServerControlCell,
    ui_control: Option<UiControl>,
    apply_lock: ApplyLock,
) -> Result<(), Error> {
    let iii_for_fn = iii.clone();
    iii.register_function(
        CONFIG_CHANGE_FN_ID,
        RegisterFunction::new_async(move |_: ConfigChangeRequest| {
            let iii = iii_for_fn.clone();
            let port = port.clone();
            let state = state.clone();
            let server_control = server_control.clone();
            let ui_control = ui_control.clone();
            let apply_lock = apply_lock.clone();
            async move {
                apply_current_config(
                    &iii,
                    &port,
                    &state,
                    &server_control,
                    ui_control.as_ref(),
                    &apply_lock,
                )
                .await;
                Ok::<_, Error>(ConfigChangeAck { ok: true })
            }
        })
        .description(
            "Internal: re-apply the Console HTTP port and injectable-UI toggles \
             when its configuration entry changes.",
        )
        .metadata(json!({ "internal": true })),
    );

    iii.register_trigger(RegisterTriggerInput::new(
        "configuration".to_string(),
        CONFIG_CHANGE_FN_ID.to_string(),
        json!({
            "configuration_id": config_id(),
            "event_types": ["configuration:updated"],
        }),
    ))?;
    Ok(())
}

/// Catch up after trigger registration, and serve as the trigger handler body.
/// Errors are logged and keep the previous listener/runtime state intact.
pub async fn apply_current_config(
    iii: &IIIClient,
    port: &PortCell,
    state: &AppState,
    server_control: &ServerControlCell,
    ui_control: Option<&UiControl>,
    apply_lock: &ApplyLock,
) {
    let _guard = apply_lock.lock().await;
    let current_port = *port.read().await;
    let candidate = match fetch_runtime_config(iii, current_port).await {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(%error, "console config-change fetch failed; keeping previous state");
            return;
        }
    };

    if candidate.http_port != current_port {
        if let Err(error) = rebind(port, state, server_control, candidate.http_port).await {
            tracing::error!(
                %error,
                old = current_port,
                new = candidate.http_port,
                "console rebind failed; keeping previous port and listener"
            );
            return;
        }
        tracing::info!(
            old = current_port,
            new = candidate.http_port,
            "console server rebound after configuration change; old port shutting down"
        );
    }

    apply_runtime_ui(&candidate, ui_control).await;
}

/// Rebind the Console listener with the HTTP worker's bind-new-before-stop-old
/// ordering. All fallible work completes before the live port snapshot or
/// current-server slot changes.
async fn rebind(
    port: &PortCell,
    state: &AppState,
    control: &ServerControlCell,
    new_port: u16,
) -> anyhow::Result<()> {
    let listener = server::bind_listener(new_port).await?;
    let new_control = server::spawn_server(listener, state.clone());

    let old = {
        let mut current = control.lock().await;
        if current.is_none() {
            server::stop_old_server(new_control);
            anyhow::bail!("console server is already shut down");
        }

        *port.write().await = new_port;
        current.replace(new_control)
    };

    if let Some(old) = old {
        server::stop_old_server(old);
    }
    Ok(())
}

/// Tolerant extraction: non-object sections and non-string entries are
/// ignored rather than failing the whole apply.
fn disabled_workers_from(value: &Value) -> HashSet<String> {
    value
        .get("injectableUi")
        .and_then(|ui| ui.get("disabledWorkers"))
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeAck {
    pub ok: bool,
}

#[derive(Debug, Default, Clone, serde::Deserialize, schemars::JsonSchema)]
pub struct ConfigChangeRequest {}

/// `Ok(None)` when the entry does not exist or holds `null`.
pub(crate) async fn existing_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_configuration_with_retry(iii, "configuration::get", json!({ "id": config_id() }))
        .await
    {
        Ok(resp) => Ok(resp.get("value").filter(|v| !v.is_null()).cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

/// Replace the stored `console` entry wholesale (`configuration::set`).
pub(crate) async fn set_value(iii: &IIIClient, value: Value) -> Result<(), String> {
    trigger_configuration_with_retry(
        iii,
        "configuration::set",
        json!({ "id": config_id(), "value": value }),
    )
    .await
    .map(|_| ())
}

pub(crate) async fn trigger_configuration_with_retry(
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
    fn disabled_workers_parse_matrix() {
        let none = json!({});
        assert!(disabled_workers_from(&none).is_empty());

        let empty = json!({ "injectableUi": { "disabledWorkers": [] } });
        assert!(disabled_workers_from(&empty).is_empty());

        let two = json!({ "injectableUi": { "disabledWorkers": ["state", "grok"] } });
        let parsed = disabled_workers_from(&two);
        assert_eq!(parsed.len(), 2);
        assert!(parsed.contains("state"));
        assert!(parsed.contains("grok"));

        // Junk entries are skipped, not fatal — a hand-edited entry must
        // never wedge the apply.
        let junk = json!({ "injectableUi": { "disabledWorkers": ["state", 7, null, {}] } });
        assert_eq!(disabled_workers_from(&junk).len(), 1);
        let wrong_shape = json!({ "injectableUi": "nope" });
        assert!(disabled_workers_from(&wrong_shape).is_empty());
    }

    #[test]
    fn schema_pins_the_injectable_ui_section() {
        let s = schema();
        let section = &s["properties"]["injectableUi"]["properties"]["disabledWorkers"];
        assert_eq!(section["type"], "array");
        assert_eq!(section["items"]["type"], "string");
    }

    #[test]
    fn schema_exposes_the_live_http_port() {
        let port = &schema()["properties"]["http_port"];
        assert_eq!(port["type"], "integer");
        assert_eq!(port["minimum"], 0);
        assert_eq!(port["maximum"], 65_535);
        assert_eq!(port["default"], 3113);
    }

    #[test]
    fn runtime_port_uses_stored_value_or_fallback() {
        let stored = json!({
            "http_port": 9123,
            "injectableUi": { "disabledWorkers": ["state"] }
        });
        let parsed = runtime_config_from(Some(&stored), 3113).unwrap();
        assert_eq!(parsed.http_port, 9123);
        assert!(parsed.disabled_workers.contains("state"));

        assert_eq!(
            runtime_config_from(Some(&json!({})), 4555)
                .unwrap()
                .http_port,
            4555
        );
        assert_eq!(runtime_config_from(None, 4666).unwrap().http_port, 4666);
        assert!(runtime_config_from(Some(&json!({ "http_port": 70_000 })), 3113).is_err());
        assert!(runtime_config_from(Some(&json!({ "http_port": "3113" })), 3113).is_err());
    }

    #[test]
    fn seed_groups_sessions_by_id_not_name() {
        // Grouping must key on `iii.session.id` — the engine's `group_by`
        // drops spans lacking the attribute, and `iii.session.name` is only
        // stamped once a session has a title, which hid every untitled
        // session's traces from the default view.
        let v = default_value(3113);
        let view = &v["traces"]["views"][0];
        assert_eq!(view["id"], "view-sessions");
        assert_eq!(view["groupBy"], "iii.session.id");
        assert_eq!(v["traces"]["activeViewId"], "view-sessions");
        assert_eq!(v["http_port"], 3113);
    }

    #[test]
    fn seed_starts_with_no_disabled_workers() {
        let v = default_value(3113);
        assert!(disabled_workers_from(&v).is_empty());
        assert!(v["injectableUi"]["disabledWorkers"]
            .as_array()
            .unwrap()
            .is_empty());
    }

    fn two_free_ports() -> (u16, u16) {
        loop {
            let first = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let second = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let ports = (
                first.local_addr().unwrap().port(),
                second.local_addr().unwrap().port(),
            );
            if ports.0 != ports.1 {
                return ports;
            }
        }
    }

    async fn wait_for_connection(port: u16, expected: bool) -> bool {
        for _ in 0..50 {
            let connected = tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok();
            if connected == expected {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }

    #[tokio::test]
    async fn rebind_starts_new_port_before_stopping_old_listener() {
        let (old_port, new_port) = two_free_ports();
        let state = AppState::new(Arc::new("ws://127.0.0.1:1".to_string()), None, None);
        let handle = server::start(old_port, state.clone()).await.unwrap();
        let port = new_port_cell(old_port);

        assert!(wait_for_connection(old_port, true).await);
        rebind(&port, &state, &handle.control, new_port)
            .await
            .unwrap();

        assert_eq!(*port.read().await, new_port);
        assert_eq!(handle.current_addr().await.unwrap().port(), new_port);
        assert!(wait_for_connection(new_port, true).await);
        assert!(wait_for_connection(old_port, false).await);

        handle.shutdown().await;
    }

    #[tokio::test]
    async fn failed_rebind_keeps_old_port_and_listener() {
        let (old_port, occupied_port) = two_free_ports();
        let occupied = tokio::net::TcpListener::bind(("0.0.0.0", occupied_port))
            .await
            .unwrap();
        let state = AppState::new(Arc::new("ws://127.0.0.1:1".to_string()), None, None);
        let handle = server::start(old_port, state.clone()).await.unwrap();
        let port = new_port_cell(old_port);

        assert!(rebind(&port, &state, &handle.control, occupied_port)
            .await
            .is_err());
        assert_eq!(*port.read().await, old_port);
        assert_eq!(handle.current_addr().await.unwrap().port(), old_port);
        assert!(wait_for_connection(old_port, true).await);

        drop(occupied);
        handle.shutdown().await;
    }
}
