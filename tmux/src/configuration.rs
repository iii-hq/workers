//! Bridge to the `configuration` worker through the shared
//! `iii-config-client` crate: register the `tmux` entry (seeding only when
//! nothing is stored yet), fetch the authoritative value, and hot-apply
//! `configuration:updated` into the live [`SharedConfig`].

use std::sync::Arc;

use iii_config_client::{EntrySpec, Reload};
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::config::{SharedConfig, WorkerConfig, CONFIG_DESCRIPTION, CONFIG_ID, CONFIG_NAME};

pub const RELOAD_FN_ID: &str = "tmux::on-config-change";

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

fn entry_spec() -> EntrySpec {
    EntrySpec {
        id: config_id(),
        form_id: CONFIG_ID,
        name: CONFIG_NAME,
        description: CONFIG_DESCRIPTION,
        schema: WorkerConfig::json_schema(),
        default_value: WorkerConfig::default().to_json(),
    }
}

/// Register the schema every boot; `seed` (a `--config` value) or the
/// built-in default becomes `initial_value` only when nothing is stored yet.
pub async fn register(iii: &IIIClient, seed: Option<&WorkerConfig>) -> Result<(), String> {
    iii_config_client::ensure(iii, &entry_spec(), seed.map(WorkerConfig::to_json)).await
}

/// The live value; the built-in default when nothing is stored.
pub async fn fetch(iii: &IIIClient) -> Result<WorkerConfig, String> {
    match iii_config_client::fetch(iii, config_id()).await? {
        Some(value) => WorkerConfig::from_json(&value),
        None => Ok(WorkerConfig::default()),
    }
}

/// Bind `configuration:updated` → refetch → swap. An invalid value keeps
/// the previous one. Call `.run()` on the result once to close the boot gap.
pub fn bind_reload(iii: &Arc<IIIClient>, shared: SharedConfig) -> Result<Reload, Error> {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        config_id(),
        RELOAD_FN_ID,
        "Internal: re-read the tmux configuration after an operator change.",
        move || {
            let engine = engine.clone();
            let shared = shared.clone();
            async move {
                match fetch(&engine).await {
                    Ok(next) => {
                        if *shared.load_full() != next {
                            shared.store(Arc::new(next));
                            tracing::info!("tmux configuration reloaded");
                        }
                    }
                    Err(error) => {
                        tracing::error!(%error, "tmux configuration reload failed; keeping the previous one");
                    }
                }
            }
        },
    )
}
