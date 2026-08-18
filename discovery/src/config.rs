//! Config for the discovery worker, owned by the builtin `configuration`
//! worker (registered under id `discovery`). One knob: the minimum surface
//! width at which the pre-generate hint fires. Hot-applies, no restart.

use std::sync::Arc;

use iii_config_client as config_client;
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub const CONFIG_ID: &str = "discovery";
pub const CONFIG_FN_ID: &str = "discovery::on-config-change";

/// Live configuration cell shared with the hook.
pub type ConfigCell = Arc<RwLock<DiscoveryConfig>>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct DiscoveryConfig {
    /// Bind the `discovery::pre-generate` hook so the conditional search
    /// hint can be injected into agent generations. On by default; turning
    /// it off unbinds the hook entirely (hot, no restart) — the model then
    /// only finds `discovery::search_functions` through normal discovery.
    pub inject_hint: bool,
    /// Only inject the discovery hint when the session's surface spans at
    /// least this many distinct non-engine workers. Narrower surfaces
    /// resolve faster through normal discovery than through a standing
    /// hint; `0` hints on every surface.
    pub hint_min_workers: usize,
    /// When a search matches no installed function, also search the public
    /// worker registry (verified authors only) and return matching workers
    /// as an `installable` section — the model can propose `worker::add`.
    /// Fail-open: a registry error falls back to the plain refine guidance.
    pub registry_fallback: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            inject_hint: true,
            hint_min_workers: 2,
            registry_fallback: true,
        }
    }
}

impl DiscoveryConfig {
    pub fn json_schema() -> serde_json::Value {
        serde_json::to_value(schemars::schema_for!(DiscoveryConfig))
            .expect("DiscoveryConfig schema serializes")
    }

    /// Parse from the flat JSON object the configuration worker stores;
    /// missing keys fall back to defaults (`#[serde(default)]`).
    pub fn from_json(v: &serde_json::Value) -> Result<DiscoveryConfig, String> {
        serde_json::from_value(v.clone()).map_err(|e| format!("invalid discovery config: {e}"))
    }

    pub fn to_json(&self) -> serde_json::Value {
        serde_json::to_value(self).expect("DiscoveryConfig serializes")
    }
}

fn spec() -> config_client::EntrySpec {
    config_client::EntrySpec {
        id: CONFIG_ID,
        name: "discovery",
        description: "Discovery worker settings — whether the pre-generate search hint is bound at all (inject_hint, hot), the minimum surface width (hint_min_workers) at which it fires, and whether an empty search also consults the public worker registry (registry_fallback).",
        schema: DiscoveryConfig::json_schema(),
        default_value: DiscoveryConfig::default().to_json(),
    }
}

pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    config_client::register(iii, &spec(), None).await
}

pub async fn fetch_config(iii: &IIIClient) -> Result<DiscoveryConfig, String> {
    match config_client::fetch(iii, CONFIG_ID).await? {
        Some(v) => DiscoveryConfig::from_json(&v),
        None => {
            tracing::info!("no configuration value found; using built-in defaults");
            Ok(DiscoveryConfig::default())
        }
    }
}

/// Register `discovery::on-config-change` and bind it to
/// `configuration:updated` for the `discovery` entry. Every delivery
/// re-fetches the authoritative value into the shared cell; the returned
/// [`config_client::Reload`] lets boot run one extra pass to close the
/// fetch→bind gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell,
    hint_binding: crate::hook::HintBindingState,
) -> Result<config_client::Reload, Error> {
    let engine = iii.clone();
    config_client::on_change(
        iii,
        CONFIG_ID,
        CONFIG_FN_ID,
        "Internal: reload discovery settings from the authoritative configuration on change.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            let hint_binding = hint_binding.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(cfg) => {
                        *cell.write().await = cfg;
                        crate::hook::apply(&engine, &hint_binding, cfg.inject_hint);
                        tracing::info!(
                            inject_hint = cfg.inject_hint,
                            hint_min_workers = cfg.hint_min_workers,
                            "discovery configuration reloaded"
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
    use serde_json::json;

    #[test]
    fn defaults_hint_at_two_workers() {
        assert!(DiscoveryConfig::default().inject_hint);
        assert!(DiscoveryConfig::from_json(&json!({})).unwrap().inject_hint);
        assert!(
            !DiscoveryConfig::from_json(&json!({ "inject_hint": false }))
                .unwrap()
                .inject_hint
        );
        assert_eq!(DiscoveryConfig::default().hint_min_workers, 2);
        assert_eq!(
            DiscoveryConfig::from_json(&json!({}))
                .unwrap()
                .hint_min_workers,
            2
        );
        assert!(DiscoveryConfig::default().registry_fallback);
        assert!(
            !DiscoveryConfig::from_json(&json!({ "registry_fallback": false }))
                .unwrap()
                .registry_fallback
        );
    }

    #[test]
    fn round_trips_through_json() {
        let cfg = DiscoveryConfig {
            inject_hint: false,
            hint_min_workers: 0,
            registry_fallback: false,
        };
        assert_eq!(DiscoveryConfig::from_json(&cfg.to_json()).unwrap(), cfg);
    }
}
