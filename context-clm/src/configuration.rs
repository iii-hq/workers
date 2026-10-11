//! Bridge to the `configuration` worker through `iii-config-client`: register
//! the `context-clm` entry (seeding the defaults only when nothing is stored),
//! fetch the authoritative value, and swap the live snapshot when it changes.
//! Tier 1 (ConfigCell only): every field but `data_dir` is read per call,
//! nothing rebuilds; the store's folder is fixed at boot.
use std::sync::{Arc, OnceLock};

use iii_config_client::{EntrySpec, Reload};
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use tokio::sync::RwLock;

use crate::config::{ClmConfig, CONFIG_DESCRIPTION, CONFIG_ID, CONFIG_NAME};

/// Hot-swappable snapshot: handlers clone the inner `Arc` out of a short read
/// lock; a reload replaces it whole.
pub type ConfigCell<T> = Arc<RwLock<Arc<T>>>;

pub const CONFIG_FN_ID: &str = "context-clm::on-config-change";

/// Process-stable entry id: `III_CONFIG_NAME` when Compose assigns one.
pub fn config_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONFIG_ID.to_owned())
    })
    .as_str()
}

pub fn new_cell(config: ClmConfig) -> ConfigCell<ClmConfig> {
    Arc::new(RwLock::new(Arc::new(config)))
}

pub async fn register_config(iii: &IIIClient) -> Result<(), String> {
    let spec = EntrySpec {
        id: config_id(),
        form_id: CONFIG_ID,
        name: CONFIG_NAME,
        description: CONFIG_DESCRIPTION,
        schema: ClmConfig::json_schema(),
        default_value: ClmConfig::default().to_json(),
    };
    iii_config_client::ensure(iii, &spec, None)
        .await
        .map_err(|e| format!("context-clm configuration registration failed: {e}"))
}

pub async fn fetch_config(iii: &IIIClient) -> Result<ClmConfig, String> {
    match iii_config_client::fetch(iii, config_id())
        .await
        .map_err(|e| format!("context-clm configuration fetch failed: {e}"))?
    {
        Some(value) => ClmConfig::from_json(&value),
        None => Ok(ClmConfig::default()),
    }
}

/// Bind `configuration:updated` -> refetch -> swap. An invalid value keeps
/// the previous snapshot. Run the returned handle once at boot to close the
/// subscription gap.
pub fn register_config_trigger(
    iii: &Arc<IIIClient>,
    cell: ConfigCell<ClmConfig>,
) -> Result<Reload, Error> {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        config_id(),
        CONFIG_FN_ID,
        "Internal: re-read the context-clm configuration after an operator change.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                match fetch_config(&engine).await {
                    Ok(config) => {
                        *cell.write().await = Arc::new(config);
                        tracing::info!("context-clm configuration reloaded");
                    }
                    Err(error) => tracing::warn!(
                        %error,
                        "context-clm configuration reload failed; keeping the previous snapshot"
                    ),
                }
            }
        },
    )
}

/// `context-clm::configuration-id`: the console's lookup of this instance's entry.
pub fn register_identity(iii: &IIIClient) {
    iii_console_ui::register_configuration_identity(iii, crate::WORKER_NAME, config_id());
}
