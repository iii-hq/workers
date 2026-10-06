//! Shared config-client atomic initialization and serialized authoritative hot reload.
use crate::config::ClefConfig;
use iii_sdk::{errors::Error, IIIClient};
use std::sync::{Arc, OnceLock};
use tokio::sync::RwLock;

/// Each call clones the inner Arc once; reload never mutates an in-flight call.
pub type SharedConfig = Arc<RwLock<Arc<ClefConfig>>>;
pub const DEFAULT_CONFIG_ID: &str = "judge-clef";
pub const CONFIG_FN_ID: &str = "judge-clef::on-config-change";

pub fn config_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| DEFAULT_CONFIG_ID.into())
    })
    .as_str()
}
pub fn new_cell(config: ClefConfig) -> SharedConfig {
    Arc::new(RwLock::new(Arc::new(config)))
}
pub async fn apply_config(cell: &SharedConfig, config: ClefConfig) -> bool {
    if config.validate().is_err() {
        return false;
    }
    *cell.write().await = Arc::new(config);
    true
}
pub async fn register_config(iii: &IIIClient, seed: Option<&ClefConfig>) -> Result<(), String> {
    let spec = iii_config_client::EntrySpec {
        id: config_id(),
        form_id: DEFAULT_CONFIG_ID,
        name: "Judge clef",
        description:
            "clef provider for judge: Clef model, CPU/GPU placement and execution limits (runs in-process).",
        schema: ClefConfig::json_schema(),
        default_value: ClefConfig::default().to_json(),
    };
    iii_config_client::ensure(iii, &spec, seed.map(ClefConfig::to_json))
        .await
        // Remote errors may include credentials or configuration values.
        .map_err(|_| "clef configuration registration failed".to_string())
}
pub async fn fetch_config(iii: &IIIClient) -> Result<ClefConfig, String> {
    match iii_config_client::fetch(iii, config_id())
        .await
        .map_err(|_| "clef configuration fetch failed".to_string())?
    {
        Some(value) => ClefConfig::from_json(&value),
        None => Ok(ClefConfig::default()),
    }
}
/// Bind reload to authoritative storage, ignoring advisory event contents.
/// Call the returned Reload::run once at boot to close the subscription gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: SharedConfig,
) -> Result<iii_config_client::Reload, Error> {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        config_id(),
        CONFIG_FN_ID,
        "Internal: reload clef configuration from authoritative storage.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(config) => {
                        apply_config(&cell, config).await;
                        tracing::info!("clef configuration reloaded");
                    }
                    Err(_) => {
                        tracing::warn!(
                            "clef configuration reload failed; keeping previous snapshot"
                        )
                    }
                }
            }
        },
    )
}
