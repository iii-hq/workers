//! Bridge to the `configuration` worker through the shared
//! `iii-config-client` crate: register the `kanban` entry (seeding only when
//! nothing is stored yet), fetch the authoritative value, and hot-apply
//! `configuration:updated` into the live [`ConfigCell`].

use std::sync::Arc;

use iii_config_client::{EntrySpec, Reload};
use iii_sdk::IIIClient;
use iii_sdk::errors::Error;
use serde_json::Value;

use crate::config::{self, CONFIG_DESCRIPTION, CONFIG_ID, CONFIG_NAME, KanbanConfig};
use crate::store::ConfigCell;

pub const RELOAD_FN_ID: &str = "kanban::on-config-change";

fn entry_spec() -> EntrySpec {
    EntrySpec {
        id: CONFIG_ID,
        form_id: CONFIG_ID,
        name: CONFIG_NAME,
        description: CONFIG_DESCRIPTION,
        schema: config::schema(),
        default_value: KanbanConfig::default().to_json(),
    }
}

/// Register the schema every boot; `seed` (a `--config` value) or the built-in
/// default becomes `initial_value` only when nothing is stored yet.
pub async fn register(iii: &IIIClient, seed: Option<Value>) -> Result<(), String> {
    let seed = seed.map(|value| config::normalize(&value).to_json());
    iii_config_client::register(iii, &entry_spec(), seed).await
}

/// The live value, repaired by [`config::normalize`]; the built-in default
/// when nothing is stored.
pub async fn fetch(iii: &IIIClient) -> Result<KanbanConfig, String> {
    Ok(iii_config_client::fetch(iii, CONFIG_ID)
        .await?
        .map(|value| config::normalize(&value))
        .unwrap_or_default())
}

/// Bind `configuration:updated` → refetch → swap the cell. Call `.run()` on
/// the returned handle once right after binding to close the boot gap.
pub fn bind_reload(iii: &Arc<IIIClient>, cell: ConfigCell) -> Result<Reload, Error> {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        CONFIG_ID,
        RELOAD_FN_ID,
        "Internal: re-read the kanban configuration after an operator change.",
        move || {
            let engine = engine.clone();
            let cell = cell.clone();
            async move {
                match fetch(&engine).await {
                    Ok(next) => {
                        let board_file = next.board_file();
                        *cell.write().unwrap_or_else(|p| p.into_inner()) = next;
                        tracing::info!(board_file = %board_file.display(), "kanban configuration reloaded");
                    }
                    Err(error) => {
                        tracing::error!(error = %error, "kanban configuration reload failed; keeping previous config");
                    }
                }
            }
        },
    )
}
