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

/// Process-stable entry identity; the form family remains CONFIG_ID.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONFIG_ID.to_string())
    })
    .as_str()
}
pub const CONFIG_FN_ID: &str = "fp::on-config-change";

/// Pair this process's dynamic entry ID with the stable function-provider form family.
fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: config_id(),
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

/// Parse the assigned entry, retaining standalone defaults only when no value exists.
pub async fn fetch_config(iii: &IIIClient) -> Result<FpConfig, String> {
    match config_client::fetch(iii, config_id()).await? {
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
        config_id(),
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
