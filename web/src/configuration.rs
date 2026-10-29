//! Integration with the builtin `configuration` worker (plumbing shared via
//! `crates/config-client`): register the `WebConfig` schema + optional seed
//! at boot, read the authoritative (env-expanded) value, and observe
//! `configuration::on-updated` so `configuration:updated` re-fetches and
//! applies the change. All WebConfig fields hot-reload (no topology
//! partition); `inject_guidance` additionally observes or disposes the
//! `web::inject-guidance` observer of `harness::hook::on-pre-generate`.

use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::{IIIClient, Source};
use serde_json::{json, Value};

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

/// The harness hook point the guidance hook observes.
pub const PRE_GENERATE_HOOK_FN: &str = "harness::hook::on-pre-generate";

/// The live pre-generate guidance observer, if any. Shared with the
/// configuration change handler so a config flip can observe/dispose at
/// runtime.
pub type GuidanceState = config_client::ObserverSlot;

/// The args of the guidance observer. `on_error: fail_open` is MANDATORY:
/// pre-generate defaults fail-CLOSED, which would abort generation if this
/// hook ever errored/timed out; a missing guidance line must never block a
/// turn. `inject_prompt` carries the guidance statically, so the harness
/// appends it without calling this worker on every generate step.
pub fn guidance_observer_args() -> Value {
    json!({
        "on_error": "fail_open",
        "inject_prompt": inject_guidance::WEB_GUIDANCE,
    })
}

/// Reconcile the live `web::inject-guidance` observer with the configured
/// `inject_guidance` value: on → observe once; off → dispose the observer and
/// drop the handle. Idempotent under repeated config events.
///
/// Observing is one shot: while the harness is not up, the observer stays
/// `pending` and becomes active when `harness::hook::on-pre-generate`
/// registers.
pub fn apply_guidance(iii: &IIIClient, state: &GuidanceState, enabled: bool) {
    state.reconcile(
        enabled,
        || {
            iii.observe(
                Source::function(PRE_GENERATE_HOOK_FN).args(guidance_observer_args()),
                inject_guidance::GUIDANCE_HOOK_ID,
            )
        },
        "inject_guidance on: appending web::fetch guidance to agent system prompts",
        "inject_guidance off: web::fetch guidance stays out of agent system prompts",
    );
}

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: config_id(),
        form_id: DEFAULT_CONFIG_ID,
        name: "web",
        description:
            "Timeouts, byte caps, user-agent, and loopback policy for the web::fetch worker.",
        schema: WebConfig::json_schema(),
        default_value: WebConfig::default().to_json(),
    }
}

/// Register the schema and seed the built-in default (or a `--config` seed)
/// atomically via `config_client::ensure`: the candidate is installed ONLY
/// when nothing is stored yet, so an operator's console edit is preserved on
/// every boot instead of being clobbered by an unconditional re-seed.
pub async fn register_config(iii: &IIIClient, seed: Option<&WebConfig>) -> Result<(), String> {
    config_client::ensure(iii, &spec(), seed.map(WebConfig::to_json)).await
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

/// Register `web::on-config-change` and observe `configuration::on-updated`
/// with it for the `web` entry. Every delivery does ONE re-fetch under the
/// shared reload lock and applies it to both the config snapshot and the
/// guidance observer (so the two can never settle from different fetches);
/// the returned [`config_client::Reload`] lets boot run one extra pass to
/// close the fetch→observe gap.
pub fn register_config_observer(iii: &Arc<IIIClient>, state: SharedState) -> config_client::Reload {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The guidance observer fails open and carries the guidance as an arg:
    /// the harness never sees observer metadata.
    #[test]
    fn guidance_observer_fails_open_and_carries_the_prompt() {
        let args = guidance_observer_args();
        assert_eq!(args["on_error"], "fail_open");
        assert_eq!(args["inject_prompt"], inject_guidance::WEB_GUIDANCE);
        assert_eq!(args.as_object().map(|o| o.len()), Some(2));
    }
}
