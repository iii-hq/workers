//! Bridge to the `configuration` worker through `iii-config-client`: register
//! the `secrets` entry (seeding the defaults only when nothing is stored),
//! fetch the authoritative value, and re-point the store — and the watch on
//! its env file — when it changes.
use std::sync::Arc;

use iii_config_client::{EntrySpec, Reload};
use iii_sdk::errors::Error;
use iii_sdk::IIIClient;

use crate::config::{SecretsConfig, CONFIG_DESCRIPTION, CONFIG_ID, CONFIG_NAME};
use crate::functions::Ctx;
use crate::store::StorePaths;

pub const RELOAD_FN_ID: &str = "secrets::on-config-change";

/// Process-stable entry id: `III_CONFIG_NAME` when Compose assigns one.
pub fn config_id() -> &'static str {
    static ID: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    ID.get_or_init(|| {
        std::env::var("III_CONFIG_NAME")
            .ok()
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| CONFIG_ID.to_owned())
    })
    .as_str()
}

pub async fn register(iii: &IIIClient) -> Result<(), String> {
    let spec = EntrySpec {
        id: config_id(),
        form_id: CONFIG_ID,
        name: CONFIG_NAME,
        description: CONFIG_DESCRIPTION,
        schema: SecretsConfig::schema(),
        default_value: SecretsConfig::default().to_json(),
    };
    iii_config_client::ensure(iii, &spec, None)
        .await
        .map_err(|_| "secrets configuration registration failed".to_owned())
}

pub async fn fetch(iii: &IIIClient) -> Result<SecretsConfig, String> {
    match iii_config_client::fetch(iii, config_id())
        .await
        .map_err(|_| "secrets configuration fetch failed".to_owned())?
    {
        Some(value) => SecretsConfig::from_json(&value),
        None => Ok(SecretsConfig::default()),
    }
}

/// Bind `configuration:updated` → refetch → re-point the store. A new env
/// file is followed from then on, and every shared variable whose value
/// differs there is reported on `secrets::changed`. Run the returned handle
/// once at boot to close the subscription gap.
pub fn bind_reload(iii: &Arc<IIIClient>, ctx: Arc<Ctx>) -> Result<Reload, Error> {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        config_id(),
        RELOAD_FN_ID,
        "Internal: re-read the secrets configuration after an operator change.",
        move || {
            let engine = engine.clone();
            let ctx = ctx.clone();
            async move {
                match fetch(&engine).await {
                    Ok(config) => {
                        if ctx
                            .store
                            .reconfigure(StorePaths::from_config(&config))
                            .await
                        {
                            let vault = ctx.store.vault_path().await;
                            tracing::info!(
                                vault = %vault.display(),
                                env_file = config.env_file,
                                "secrets configuration reloaded; vault re-opened"
                            );
                            crate::envwatch::follow(&ctx).await;
                            for event in ctx.store.env_changes().await {
                                ctx.changed(&event);
                            }
                        }
                    }
                    Err(error) => {
                        tracing::warn!(error = %error, "secrets configuration reload failed; keeping the current vault")
                    }
                }
            }
        },
    )
}
