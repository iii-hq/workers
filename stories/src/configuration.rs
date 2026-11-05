//! Bridge to the `configuration` worker through the shared
//! `iii-config-client` crate: register the `stories` entry (seeding only when
//! nothing is stored yet), fetch the authoritative value, and hot-apply
//! every `configuration::on-updated` update of the entry into the live
//! [`ConfigCell`].

use std::sync::Arc;

use iii_config_client::{EntrySpec, Reload};
use iii_sdk::IIIClient;
use serde_json::Value;

use crate::config::ConfigCell;
use crate::config::{self, CONFIG_DESCRIPTION, CONFIG_ID, CONFIG_NAME, StoriesConfig};

pub const RELOAD_FN_ID: &str = "stories::on-config-change";

/// The configuration entry this worker owns: `III_CONFIG_NAME` when a
/// supervisor set it (Compose assigns `<namespace>-stories` and migrates the
/// legacy `stories` entry into it), else the built-in `stories`. The form
/// family (`form_id`) stays `stories` either way.
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
        schema: config::schema(),
        default_value: StoriesConfig::default().to_json(),
    }
}

/// Register the schema every boot; `seed` (a `--config` value) or the built-in
/// default becomes `initial_value` only when nothing is stored yet.
pub async fn register(iii: &IIIClient, seed: Option<Value>) -> Result<(), String> {
    let seed = seed.map(|value| config::normalize(&value).to_json());
    iii_config_client::ensure(iii, &entry_spec(), seed).await
}

/// The live value, repaired by [`config::normalize`]; the built-in default
/// when nothing is stored.
pub async fn fetch(iii: &IIIClient) -> Result<StoriesConfig, String> {
    Ok(iii_config_client::fetch(iii, config_id())
        .await?
        .map(|value| config::normalize(&value))
        .unwrap_or_default())
}

/// Observe `configuration::on-updated` for the entry → refetch → swap the
/// cell. Call `.run()` on the returned handle once right after to close the
/// boot gap.
pub fn bind_reload(iii: &Arc<IIIClient>, cell: ConfigCell) -> Reload {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        config_id(),
        RELOAD_FN_ID,
        "Internal: re-read the stories configuration after an operator change.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                match fetch(&engine).await {
                    Ok(next) => {
                        let data_path = next.data_path_resolved();
                        *cell.write().unwrap_or_else(|p| p.into_inner()) = next;
                        tracing::info!(data_path = %data_path.display(), "stories configuration reloaded");
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "stories configuration reload failed; keeping previous config");
                    }
                }
            }
        },
    )
}
