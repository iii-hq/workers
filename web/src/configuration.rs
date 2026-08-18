//! Integration with the builtin `configuration` worker (plumbing shared via
//! `crates/config-client`): register the `WebConfig` schema + optional seed
//! at boot, read the authoritative (env-expanded) value, and bind a
//! `configuration` trigger so `configuration:updated` re-fetches and applies
//! the change. All WebConfig fields hot-reload (no topology partition);
//! `inject_guidance` additionally binds or unbinds the
//! `web::inject-guidance` pre-generate hook.

use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::IIIClient;
use serde_json::json;

use crate::config::{SharedConfig, WebConfig};
use crate::functions::inject_guidance;

pub const DEFAULT_CONFIG_ID: &str = "web";

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
pub const CONFIG_FN_ID: &str = "web::on-config-change";

#[derive(Clone)]
pub struct SharedState {
    pub config: SharedConfig,
    pub guidance: GuidanceState,
}

/// The live pre-generate guidance binding, if any. Shared with the
/// configuration change handler so a config flip can bind/unbind at runtime.
pub type GuidanceState = config_client::BindingSlot;

/// Reconcile the live `web::inject-guidance` binding with the configured
/// `inject_guidance` value: on → bind once; off → unregister and drop the
/// handle. Idempotent under repeated config events, and a failed bind
/// retries on the next event.
///
/// Binding is one shot: if the harness is not up yet, the engine parks the
/// binding as a pending intent and activates it when the trigger type
/// registers (recoverable triggers, iii #1962). `on_error: fail_open` is
/// MANDATORY: pre_generate defaults fail-CLOSED, which would abort
/// generation if this hook ever errored/timed out; a missing guidance line
/// must never block a turn.
pub fn apply_guidance(iii: &IIIClient, state: &GuidanceState, enabled: bool) {
    state.reconcile(
        enabled,
        || {
            config_client::try_bind(
                iii,
                RegisterTriggerInput::new(
                    "harness::hook::pre-generate".to_string(),
                    inject_guidance::GUIDANCE_HOOK_ID.to_string(),
                    json!({ "on_error": "fail_open" }),
                )
                .with_metadata(json!({ "inject_prompt": inject_guidance::WEB_GUIDANCE })),
            )
        },
        "inject_guidance on: appending web::fetch guidance to agent system prompts",
        "inject_guidance off: web::fetch guidance stays out of agent system prompts",
    );
}

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: config_id(),
        name: "web",
        description:
            "Timeouts, byte caps, user-agent, and loopback policy for the web::fetch worker.",
        schema: WebConfig::json_schema(),
        default_value: WebConfig::default().to_json(),
    }
}

/// Register the schema. A `--config` seed (like the built-in default) is
/// installed only when nothing is stored yet — `configuration::register`
/// REPLACES the stored value whenever `initial_value` is supplied, so an
/// unconditional seed would clobber operator console edits on every boot.
pub async fn register_config(iii: &IIIClient, seed: Option<&WebConfig>) -> Result<(), String> {
    config_client::register(iii, &spec(), seed.map(WebConfig::to_json)).await
}

pub async fn fetch_config(iii: &IIIClient) -> Result<WebConfig, String> {
    match config_client::fetch(iii, config_id()).await? {
        Some(v) => WebConfig::from_json(&v),
        None => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(WebConfig::default())
        }
    }
}

pub async fn apply_config(state: &SharedState, cfg: WebConfig) {
    state.config.store(std::sync::Arc::new(cfg));
}

/// Register `web::on-config-change` and bind it to `configuration:updated`
/// for the `web` entry. Every delivery does ONE re-fetch under the shared
/// reload lock and applies it to both the config snapshot and the guidance
/// binding (so the two can never settle from different fetches); the
/// returned [`config_client::Reload`] lets boot run one extra pass to close
/// the fetch→bind gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    state: SharedState,
) -> Result<config_client::Reload, Error> {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        config_id(),
        CONFIG_FN_ID,
        "Internal: reload web settings from the authoritative configuration on change.",
        move || {
            let engine = engine.clone();
            let state = state.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(cfg) => {
                        let inject = cfg.inject_guidance;
                        apply_config(&state, cfg).await;
                        apply_guidance(&engine, &state.guidance, inject);
                        tracing::info!(inject_guidance = inject, "web configuration reloaded");
                    }
                    Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
                }
            }
        },
    )
}
