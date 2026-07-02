//! Integration with the `configuration` worker: register a JSON Schema +
//! seed at boot, read the authoritative (env-expanded) value, and bind a
//! `configuration` trigger so `configuration:updated` re-fetches and applies
//! the change. All WebConfig fields hot-reload (no topology partition).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use iii_sdk::errors::Error;
use iii_sdk::protocol::{RegisterTriggerInput, TriggerRequest};
use iii_sdk::trigger::Trigger;
use iii_sdk::{IIIClient, RegisterFunction};
use serde_json::{json, Value};

use crate::config::{SharedConfig, WebConfig};
use crate::functions::inject_guidance;

pub const CONFIG_ID: &str = "web";
const CONFIG_FN_ID: &str = "web::on-config-change";
const CONFIG_TIMEOUT_MS: u64 = 5_000;
const CONFIG_RETRIES: u32 = 3;

const ON_REGISTRY_CHANGED_FN_ID: &str = "web::on-registry-changed";
const WORKERS_AVAILABLE_TRIGGER_TYPE: &str = "engine::workers-available";
const FUNCTIONS_AVAILABLE_TRIGGER_TYPE: &str = "engine::functions-available";

/// The harness-PROVIDED trigger type the web worker binds to (NOT an engine built-in).
/// The web worker can boot before the harness has registered it; since `register_trigger`
/// is fire-and-forget (the engine async-rejects a bind to an unknown trigger type and
/// silently drops it), binding too early loses the guidance hook, so we bind only once
/// it is present (driven by `setup_harness_hooks`).
const HARNESS_TRIGGER_TYPES: [&str; 1] = ["harness::hook::pre-generate"];

#[derive(Clone)]
pub struct SharedState {
    pub config: SharedConfig,
}

/// Best-effort trigger binding: a transient failure must not brick boot — it surfaces
/// as a `None` handle.
fn bind(iii: &IIIClient, trigger_type: &str, function_id: &str, config: Value) -> Option<Trigger> {
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
            tracing::warn!(trigger_type, function_id, error = %e, "trigger binding failed");
            None
        }
    }
}

/// True iff every id in `needed` appears as a trigger-type `id` in an
/// `engine::triggers::list` response. Pure, so it's unit-testable.
fn all_trigger_types_present(resp: &Value, needed: &[&str]) -> bool {
    let Some(types) = resp.get("triggers").and_then(|v| v.as_array()) else {
        return false;
    };
    needed.iter().all(|n| {
        types
            .iter()
            .any(|t| t.get("id").and_then(|v| v.as_str()) == Some(*n))
    })
}

/// One probe: is the harness pre-generate hook trigger type registered right now?
async fn harness_trigger_types_ready(iii: &IIIClient) -> bool {
    match iii
        .trigger(TriggerRequest {
            function_id: "engine::triggers::list".into(),
            payload: json!({ "include_internal": true }),
            action: None,
            timeout_ms: Some(CONFIG_TIMEOUT_MS),
        })
        .await
    {
        Ok(resp) => all_trigger_types_present(&resp, &HARNESS_TRIGGER_TYPES),
        Err(_) => false, // engine not reachable / harness not up yet
    }
}

/// Bind the `web::inject-guidance` pre_generate hook EXACTLY ONCE, and only after the
/// harness has registered its trigger type. `register_trigger` is fire-and-forget — the
/// engine silently drops a bind to an unknown trigger type — so binding before the
/// harness is up would lose the hook. The shared `bound` flag keeps this idempotent
/// across the boot-time attempt and every registry-change firing. `on_error: fail_open`
/// is MANDATORY: pre_generate defaults fail-CLOSED, which would abort generation if this
/// hook ever errored/timed out; a missing guidance line must never block a turn.
async fn try_bind_harness_hooks(iii: &IIIClient, bound: &AtomicBool) {
    if bound.load(Ordering::Acquire) {
        return;
    }
    if !harness_trigger_types_ready(iii).await {
        return; // harness not up yet; a later registry-change event retries
    }
    // Win the bind race exactly once even if several worker events fire together.
    if bound
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return;
    }
    if bind(
        iii,
        "harness::hook::pre-generate",
        inject_guidance::GUIDANCE_HOOK_ID,
        json!({ "on_error": "fail_open" }),
    )
    .is_some()
    {
        tracing::info!("web pre-generate hook bound: inject-guidance (guidance injection active)");
    } else {
        // Release the claim so a later engine::*-available event retries instead of
        // leaving the hook unbound until restart.
        bound.store(false, Ordering::Release);
        tracing::warn!(
            "web inject-guidance bind failed; releasing the claim to retry on the next \
             registry-change event"
        );
    }
}

/// Bind the harness guidance hook without racing the harness's startup — and without
/// polling. The web worker can boot before the harness registers its hook trigger type,
/// so we subscribe to two engine registry-change triggers and (re)try the bind on each
/// firing; `try_bind_harness_hooks` no-ops until the harness's type appears, then binds
/// once. We need BOTH: `engine::workers-available` is immediate but can precede a
/// worker's own type registration, while `engine::functions-available` fires after
/// function registration and so reliably post-dates the harness's types. The initial
/// attempt covers the warm-start case (harness already connected, so no new event fires).
///
// ponytail: this presence-gated bind is hand-rolled per worker (workflow and rbac-proxy
// already do the same); extract a shared crate only if a 4th consumer appears.
pub async fn setup_harness_hooks(iii: &Arc<IIIClient>) {
    let bound = Arc::new(AtomicBool::new(false));

    let iii_for_fn = iii.clone();
    let bound_for_fn = bound.clone();
    iii.register_function(
        ON_REGISTRY_CHANGED_FN_ID,
        RegisterFunction::new_async(move |_event: Value| {
            let iii = iii_for_fn.clone();
            let bound = bound_for_fn.clone();
            async move {
                try_bind_harness_hooks(&iii, &bound).await;
                Ok::<Value, Error>(json!({ "ok": true }))
            }
        })
        .description(
            "Internal: on engine::workers-available / engine::functions-available, bind the \
             web inject-guidance pre-generate hook once the harness has registered its \
             trigger type. Not called directly.",
        )
        .request_format(registry_changed_request_schema())
        .response_format(registry_changed_response_schema()),
    );

    // Arm the event-driven retries BEFORE the initial probe, so a harness that comes up
    // mid-probe is still caught. Both binds are race-free (mandatory in-process types).
    for trigger_type in [
        WORKERS_AVAILABLE_TRIGGER_TYPE,
        FUNCTIONS_AVAILABLE_TRIGGER_TYPE,
    ] {
        let _ = bind(iii, trigger_type, ON_REGISTRY_CHANGED_FN_ID, json!({}));
    }

    // Warm start: the harness may already be connected, in which case no further
    // registry-change event fires for it — attempt the bind now.
    try_bind_harness_hooks(iii, &bound).await;
}

/// Explicit wire schemas for `web::on-registry-changed`: the handler keeps a lossless
/// `Value` signature (registry-change event shapes belong to the engine, and the
/// payload is ignored anyway), so schemars would emit the AnyValue schema — which the
/// registry publish gate rejects (this blocked the web/v1.2.0 release).
fn registry_changed_request_schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "OnRegistryChangedEvent",
        "description": "Registry-change event fired by engine::workers-available / \
                        engine::functions-available. The payload is ignored — any JSON \
                        is accepted; the handler just retries the guidance-hook bind.",
        "type": ["null", "boolean", "number", "string", "array", "object"]
    })
}

fn registry_changed_response_schema() -> Value {
    json!({
        "$schema": "http://json-schema.org/draft-07/schema#",
        "title": "OnRegistryChangedResponse",
        "type": "object",
        "properties": {
            "ok": {
                "type": "boolean",
                "description": "Always true; the bind attempt is fire-and-forget."
            }
        },
        "required": ["ok"]
    })
}

pub async fn register_config(iii: &IIIClient, seed: Option<&WebConfig>) -> Result<(), String> {
    let mut payload = json!({
        "id": CONFIG_ID,
        "name": "web",
        "description": "Timeouts, byte caps, user-agent, and loopback policy for the web::fetch worker.",
        "schema": WebConfig::json_schema(),
    });
    if let Some(seed) = seed {
        payload["initial_value"] = seed.to_json();
    } else if should_seed_default(iii).await? {
        payload["initial_value"] = WebConfig::default().to_json();
    }
    trigger_with_retry(iii, "configuration::register", payload).await?;
    Ok(())
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WebConfig, String> {
    match try_get_value(iii).await? {
        Some(v) if !v.is_null() => WebConfig::from_json(&v),
        _ => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(WebConfig::default())
        }
    }
}

async fn should_seed_default(iii: &IIIClient) -> Result<bool, String> {
    match try_get_value(iii).await? {
        None => Ok(true),
        Some(v) if v.is_null() => Ok(true),
        Some(_) => Ok(false),
    }
}

async fn try_get_value(iii: &IIIClient) -> Result<Option<Value>, String> {
    match trigger_with_retry(iii, "configuration::get", json!({ "id": CONFIG_ID })).await {
        Ok(resp) => Ok(resp.get("value").cloned()),
        Err(e) if e.contains("NOT_FOUND") => Ok(None),
        Err(e) => Err(e),
    }
}

pub async fn apply_config(state: &SharedState, cfg: WebConfig) {
    state.config.store(std::sync::Arc::new(cfg));
}

#[derive(Debug, Default, serde::Deserialize, schemars::JsonSchema)]
struct OnConfigChangeRequest {}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
struct OnConfigChangeResponse {
    ok: bool,
}

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
            "Internal: reload web settings from the authoritative configuration on change.",
        ),
    );

    iii.register_trigger(RegisterTriggerInput {
        trigger_type: "configuration".to_string(),
        function_id: CONFIG_FN_ID.to_string(),
        config: json!({ "configuration_id": CONFIG_ID, "event_types": ["configuration:updated"] }),
        metadata: None,
    })?;
    Ok(())
}

async fn on_config_change(iii: &IIIClient, state: &SharedState) {
    match fetch_config(iii).await {
        Ok(cfg) => {
            apply_config(state, cfg).await;
            tracing::info!("web configuration reloaded");
        }
        Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
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

    #[test]
    fn all_trigger_types_present_requires_every_needed_id() {
        // The harness pre-generate hook type present → ready.
        let ready = json!({
            "triggers": [
                { "id": "harness::hook::pre-generate" },
                { "id": "cron" },
            ]
        });
        assert!(all_trigger_types_present(&ready, &HARNESS_TRIGGER_TYPES));

        // Harness up but pre-generate not yet registered → NOT ready (keep waiting).
        let partial = json!({ "triggers": [ { "id": "harness::turn-completed" } ] });
        assert!(!all_trigger_types_present(&partial, &HARNESS_TRIGGER_TYPES));

        // No triggers field at all (engine unreachable / empty) → NOT ready.
        assert!(!all_trigger_types_present(
            &json!({}),
            &HARNESS_TRIGGER_TYPES
        ));
    }

    /// Mirrors the registry publish gate (`collect_worker_interface.py`): both
    /// explicit schemas must carry a schema-defining keyword, not the AnyValue
    /// schema (which blocked the web/v1.2.0 release).
    #[test]
    fn registry_changed_schemas_pass_the_publish_typed_gate() {
        for (field, schema) in [
            ("request_schema", registry_changed_request_schema()),
            ("response_schema", registry_changed_response_schema()),
        ] {
            let obj = schema
                .as_object()
                .unwrap_or_else(|| panic!("{field} must be a JSON object"));
            assert!(
                ["type", "properties", "$ref"]
                    .iter()
                    .any(|k| obj.contains_key(*k)),
                "web::on-registry-changed {field} is untyped: {schema}"
            );
        }
    }
}
