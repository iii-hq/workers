//! fp's entry in the builtin `configuration` worker (plumbing shared via
//! `crates/config-client`): register the `FpConfig` schema (+ default seed)
//! at boot, read the authoritative value, and bind a `configuration`
//! trigger so `configuration:updated` re-fetches and applies the change —
//! for fp that means binding or unbinding the `fp::inject-guidance`
//! pre-generate hook at runtime.

use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::config::FpConfig;
use crate::guidance;

pub const CONFIG_ID: &str = "fp";
pub const CONFIG_FN_ID: &str = "fp::on-config-change";

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: CONFIG_ID,
        form_id: CONFIG_ID,
        name: "fp",
        description: "fp worker settings — whether fp::pipe usage guidance is injected into agent system prompts (on by default).",
        schema: FpConfig::json_schema(),
        default_value: FpConfig::default().to_json(),
    }
}

pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    config_client::register(iii, &spec(), None).await
}

pub async fn fetch_config(iii: &IIIClient) -> Result<FpConfig, String> {
    match config_client::fetch(iii, CONFIG_ID).await? {
        Some(v) => FpConfig::from_json(&v),
        None => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(FpConfig::default())
        }
    }
}

/// Register `fp::on-config-change` and bind it to `configuration:updated`
/// for the `fp` entry. Every delivery re-fetches the authoritative value
/// under the shared reload lock and reconciles the guidance binding; the
/// returned [`config_client::Reload`] lets boot run one extra pass to close
/// the fetch→bind gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    state: guidance::GuidanceState,
) -> Result<config_client::Reload, Error> {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        CONFIG_ID,
        CONFIG_FN_ID,
        "Internal: reload fp settings from the authoritative configuration on change.",
        move || {
            let engine = engine.clone();
            let state = state.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(cfg) => {
                        guidance::apply(&engine, &state, cfg.inject_guidance);
                        tracing::info!(
                            inject_guidance = cfg.inject_guidance,
                            "fp configuration reloaded"
                        );
                    }
                    Err(e) => tracing::error!(error = %e, "config-change: keeping previous config"),
                }
            }
        },
    )
}
