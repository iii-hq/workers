//! This worker's entry in the builtin `configuration` worker (plumbing
//! shared via `crates/config-client`): register the schema (+ default seed)
//! at boot, read the authoritative value, and bind a `configuration`
//! trigger so `configuration:updated` re-fetches and applies the change —
//! which here means binding or unbinding the
//! `sandbox-code-runner::inject-guidance` pre-generate hook at runtime.
//!
//! Deliberately separate from [`crate::config`]: that is the operator FILE
//! config (timeouts, idle TTL) loaded once at boot; this entry carries the
//! knobs meant to flip live from the console.

use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::errors::Error;
use iii_sdk::protocol::RegisterTriggerInput;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::functions::inject_guidance;

pub const CONFIG_ID: &str = "sandbox-code-runner";
/// Internal hot-reload hook; denied to agents in iii-permissions.yaml and
/// seeded into the runtime-id registry (`functions::seeded_ids`) so a guest
/// `register_function` cannot claim it.
pub const CONFIG_FN_ID: &str = "sandbox-code-runner::on-config-change";

/// The `sandbox-code-runner` configuration entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct RunnerSharedConfig {
    /// Append the sandbox-code-runner usage guidance to every agent system
    /// prompt via the harness `pre-generate` hook. On by default; turn it
    /// off to shrink prompts (the harness's `# Granted functions` catalog
    /// still advertises the `sandbox-code-runner::*` ids). Hot-applies —
    /// the worker binds or unbinds the hook on change.
    pub inject_guidance: bool,
}

impl Default for RunnerSharedConfig {
    fn default() -> Self {
        Self {
            inject_guidance: true,
        }
    }
}

impl RunnerSharedConfig {
    pub fn json_schema() -> Value {
        serde_json::to_value(schemars::schema_for!(RunnerSharedConfig))
            .expect("RunnerSharedConfig schema serializes")
    }

    /// Parse from the flat JSON object the configuration worker stores;
    /// missing keys fall back to defaults (`#[serde(default)]`).
    pub fn from_json(v: &Value) -> Result<RunnerSharedConfig, String> {
        serde_json::from_value(v.clone())
            .map_err(|e| format!("invalid sandbox-code-runner config: {e}"))
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("RunnerSharedConfig serializes")
    }
}

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: CONFIG_ID,
        name: "sandbox-code-runner",
        description: "sandbox-code-runner settings — whether its usage guidance is injected into agent system prompts (on by default).",
        schema: RunnerSharedConfig::json_schema(),
        default_value: RunnerSharedConfig::default().to_json(),
    }
}

pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    config_client::register(iii, &spec(), None).await
}

pub async fn fetch_config(iii: &IIIClient) -> Result<RunnerSharedConfig, String> {
    match config_client::fetch(iii, CONFIG_ID).await? {
        Some(v) => RunnerSharedConfig::from_json(&v),
        None => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(RunnerSharedConfig::default())
        }
    }
}

/// The live pre-generate guidance binding, if any. Shared with the
/// configuration change handler so a config flip can bind/unbind at runtime.
pub type GuidanceState = config_client::BindingSlot;

/// Reconcile the live guidance binding with the configured `inject_guidance`
/// value: on → bind once; off → unregister and drop the handle. Idempotent
/// under repeated config events, and a failed bind retries on the next event.
///
/// `on_error: fail_open` is MANDATORY — `pre_generate` defaults to
/// fail-CLOSED, and a missing guidance line must never abort an agent's turn.
pub fn apply_guidance(iii: &IIIClient, state: &GuidanceState, enabled: bool) {
    state.reconcile(
        enabled,
        || {
            config_client::try_bind(
                iii,
                RegisterTriggerInput {
                    trigger_type: "harness::hook::pre-generate".to_string(),
                    function_id: inject_guidance::GUIDANCE_HOOK_ID.to_string(),
                    config: json!({ "on_error": "fail_open" }),
                    metadata: Some(json!({
                        "inject_prompt": inject_guidance::CODE_RUNNER_GUIDANCE
                    })),
                },
            )
        },
        "inject_guidance on: appending sandbox-code-runner guidance to agent system prompts",
        "inject_guidance off: sandbox-code-runner guidance stays out of agent system prompts",
    );
}

/// Register `sandbox-code-runner::on-config-change` and bind it to
/// `configuration:updated` for this worker's entry. Every delivery
/// re-fetches the authoritative value under the shared reload lock and
/// reconciles the guidance binding; the returned [`config_client::Reload`]
/// lets boot run one extra pass to close the fetch→bind gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    state: GuidanceState,
) -> Result<config_client::Reload, Error> {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        CONFIG_ID,
        CONFIG_FN_ID,
        "Internal: reload sandbox-code-runner settings from the authoritative configuration on change.",
        move || {
            let engine = engine.clone();
            let state = state.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(cfg) => {
                        apply_guidance(&engine, &state, cfg.inject_guidance);
                        tracing::info!(
                            inject_guidance = cfg.inject_guidance,
                            "sandbox-code-runner configuration reloaded"
                        );
                    }
                    Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
                }
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn injection_defaults_on() {
        assert!(RunnerSharedConfig::default().inject_guidance);
        assert!(
            RunnerSharedConfig::from_json(&json!({}))
                .unwrap()
                .inject_guidance
        );
    }

    #[test]
    fn parses_the_stored_flat_shape() {
        // `false` is the non-default value, so this parse is discriminating.
        let flat = RunnerSharedConfig::from_json(&json!({ "inject_guidance": false })).unwrap();
        assert!(!flat.inject_guidance);
    }

    #[test]
    fn round_trips_through_json() {
        let cfg = RunnerSharedConfig {
            inject_guidance: false,
        };
        assert_eq!(RunnerSharedConfig::from_json(&cfg.to_json()).unwrap(), cfg);
    }
}
