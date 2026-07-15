//! Register the `console` configuration entry — the server-side home for
//! console UI preferences (starting with the Traces V2 saved views). The
//! frontend reads/writes it with `configuration::get` / `configuration::set`
//! over the `/ws` proxy; registration here just guarantees the entry and its
//! schema exist before any browser connects.
//!
//! Unlike workers whose behavior depends on their configuration, the console
//! serves fine without it — when the `configuration` worker is disabled or
//! unreachable the UI degrades to in-browser defaults. Registration is
//! therefore best-effort and never blocks boot.

use std::time::Duration;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

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
                        (grouping, filters, display settings, hidden functions) \
                        and the selected view.",
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
