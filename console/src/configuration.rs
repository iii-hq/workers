//! Register the `console` configuration entry — the server-side home for
//! console UI preferences (the Traces V2 saved views) and the injectable-UI
//! per-worker toggles (`injectableUi.disabledWorkers`). The frontend
//! reads/writes it with `configuration::get` / `configuration::set` over the
//! `/ws` proxy; registration here just guarantees the entry and its schema
//! exist before any browser connects.
//!
//! Unlike workers whose behavior depends on their configuration, the console
//! serves fine without it — when the `configuration` worker is disabled or
//! unreachable the UI degrades to in-browser defaults (and every worker's
//! injected UI stays enabled). Registration is therefore best-effort and
//! never blocks boot.
//!
//! [`start_injectable_ui_sync`] is the runtime half: apply the persisted
//! `disabledWorkers` set to the [`crate::ui_assets`] registry at boot, then
//! subscribe to `configuration:updated` events for this entry so saving the
//! toggle form applies live — tabs drop/load the affected workers' assets
//! without any restart (the state worker's `configuration.rs` precedent).

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::ui_assets::UiControl;

pub const CONFIG_ID: &str = "console";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;
const CONFIG_RETRY_BACKOFF_MS: u64 = 250;

/// The `console` entry schema. Deliberately permissive below the top level:
/// the UI owns the detailed view shape (grouping, filters, display, hidden
/// functions) and must be able to evolve it without a console-worker
/// redeploy. Only the envelope — `traces.views[]` entries carrying `id` and
/// `name` — is pinned.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
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

/// Out-of-the-box preferences seeded when the entry has never been
/// configured.
///
/// - `views`: `view-sessions` groups traces by session and labels rows with
///   the tag message; `activeViewId` selects it out of the box, and the
///   frontend falls back to the same id when the pointer is absent
///   (`DEFAULT_VIEW_ID` in web tracesViews.ts — keep the id in sync).
/// - `followTurns`: follow the active chat's live turn — on out of the box
///   (the frontend also defaults to on when the flag is absent).
/// - `spanFilters`: detail-view funnel defaults — hide the chatty
///   `harness::send` span group and the session/context bookkeeping workers.
fn default_value() -> Value {
    json!({
        "traces": {
            "views": [{
                "id": "view-sessions",
                "name": "sessions",
                "groupBy": "iii.session.name",
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

/// Best-effort registration of the `console` configuration entry. Seeds the
/// default value only when nothing is stored yet (a re-register with
/// `initial_value` would replace user-saved views). Failures are logged and
/// swallowed — the console must serve even without the configuration worker.
pub async fn register_console_config(iii: &IIIClient) {
    let seed = match existing_value(iii).await {
        Ok(existing) => existing.is_none(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                "console configuration lookup failed; skipping registration \
                 (UI preferences fall back to in-browser defaults)"
            );
            return;
        }
    };

    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "Console",
        "description": "Console UI preferences — Traces V2 saved views \
                        (grouping, filters, display settings, hidden functions), \
                        the selected view, and the per-worker injectable-UI \
                        toggles.",
        "schema": schema(),
    });
    if seed {
        payload["initial_value"] = default_value();
    }

    match trigger_with_retry(iii, "configuration::register", payload).await {
        Ok(_) => tracing::info!(id = CONFIG_ID, "console configuration registered"),
        Err(e) => tracing::warn!(
            error = %e,
            "console configuration registration failed; skipping \
             (UI preferences fall back to in-browser defaults)"
        ),
    }
}

const CONFIG_CHANGE_FN_ID: &str = "console::on-config-change";

/// Apply the persisted `injectableUi.disabledWorkers` set at boot, then
/// subscribe to `configuration:updated` events for the `console` entry so
/// toggle-form saves apply live. Best-effort like the registration above:
/// with no configuration worker every injected UI simply stays enabled.
pub async fn start_injectable_ui_sync(iii: &Arc<IIIClient>, control: UiControl) {
    match fetch_disabled_workers(iii).await {
        Ok(workers) => control.set_disabled_workers(workers).await,
        Err(e) => tracing::warn!(
            error = %e,
            "injectable-ui toggles: initial configuration fetch failed; \
             every worker's injected UI stays enabled until a config change arrives"
        ),
    }

    // Serialize fetch→apply across events: the SDK spawns each invocation on
    // its own task, and a stale fetch applied after a fresh one would undo
    // the newer toggle state (the state worker's ApplyLock precedent).
    let apply_lock = Arc::new(tokio::sync::Mutex::new(()));
    {
        let iii_for_fn = iii.clone();
        let control = control.clone();
        iii.register_function(
            CONFIG_CHANGE_FN_ID,
            RegisterFunction::new_async(move |_: ConfigChangeRequest| {
                let iii = iii_for_fn.clone();
                let control = control.clone();
                let apply_lock = apply_lock.clone();
                async move {
                    let _guard = apply_lock.lock().await;
                    // Never trust the trigger payload — re-fetch the
                    // authoritative value. A fetch failure keeps the
                    // previous toggle state in place.
                    match fetch_disabled_workers(&iii).await {
                        Ok(workers) => control.set_disabled_workers(workers).await,
                        Err(e) => tracing::warn!(
                            error = %e,
                            "injectable-ui toggles: config-change fetch failed; keeping previous state"
                        ),
                    }
                    Ok::<_, Error>(ConfigChangeAck { ok: true })
                }
            })
            .description(
                "Internal: re-apply the console's injectable-UI per-worker toggles \
                 when the console configuration entry changes.",
            )
            .metadata(json!({ "internal": true })),
        );
    }

    if let Err(e) = iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_CHANGE_FN_ID.to_string(),
        config: json!({
            "configuration_id": CONFIG_ID,
            "event_types": ["configuration:updated"],
        }),
        metadata: None,
        namespace: iii.namespace(),
    }) {
        tracing::warn!(
            error = %e,
            "injectable-ui toggles: configuration trigger registration failed; \
             toggle changes apply at the next console restart"
        );
    }
}

/// Read `injectableUi.disabledWorkers` from the stored `console` entry.
/// Missing entry / section / list ⇒ empty set (everything enabled).
async fn fetch_disabled_workers(iii: &IIIClient) -> Result<HashSet<String>, String> {
    let value = existing_value(iii).await?;
    Ok(value
        .as_ref()
        .map(disabled_workers_from)
        .unwrap_or_default())
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
async fn existing_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").filter(|v| !v.is_null()).cloned()),
        Err(e) if e.to_ascii_uppercase().contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
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
    fn seed_starts_with_no_disabled_workers() {
        let v = default_value();
        assert!(disabled_workers_from(&v).is_empty());
        assert!(v["injectableUi"]["disabledWorkers"]
            .as_array()
            .unwrap()
            .is_empty());
    }
}
