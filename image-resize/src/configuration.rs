//! Bridge to the `configuration` worker through `iii-config-client`: declare
//! the entry, read it, and swap the live value on `configuration:updated`.

use std::sync::{Arc, OnceLock, PoisonError, RwLock};

use iii_config_client::{EntrySpec, Reload};
use iii_sdk::IIIClient;

use crate::config::{CONFIG_DESCRIPTION, CONFIG_ID, CONFIG_NAME, ResizeConfig};

pub const RELOAD_FN_ID: &str = "image-resize::on-config-change";

/// Each call takes one snapshot, so a reload never changes a call in flight.
pub type SharedConfig = Arc<RwLock<Arc<ResizeConfig>>>;

pub fn shared(config: ResizeConfig) -> SharedConfig {
    Arc::new(RwLock::new(Arc::new(config)))
}

pub fn snapshot(shared: &SharedConfig) -> Arc<ResizeConfig> {
    shared
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

fn resolve_config_id(assigned: Option<String>) -> String {
    assigned
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| CONFIG_ID.to_string())
}

/// The entry id: the name a supervisor assigned, else the worker's own.
pub fn config_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| resolve_config_id(std::env::var("III_CONFIG_NAME").ok()))
        .as_str()
}

/// Declare the schema on every boot; `seed` or the built-in default becomes
/// the value only while nothing is stored.
pub async fn register(iii: &IIIClient, seed: Option<&ResizeConfig>) -> Result<(), String> {
    let spec = EntrySpec {
        id: config_id(),
        form_id: CONFIG_ID,
        name: CONFIG_NAME,
        description: CONFIG_DESCRIPTION,
        schema: ResizeConfig::json_schema(),
        default_value: ResizeConfig::default().to_json(),
    };
    iii_config_client::ensure(iii, &spec, seed.map(ResizeConfig::to_json)).await
}

pub async fn fetch(iii: &IIIClient) -> Result<ResizeConfig, String> {
    match iii_config_client::fetch(iii, config_id()).await? {
        Some(value) => ResizeConfig::from_json(&value),
        None => Ok(ResizeConfig::default()),
    }
}

/// Swap in a fetched value. One that could not be read or is invalid keeps
/// the previous configuration.
pub fn apply(shared: &SharedConfig, fetched: Result<ResizeConfig, String>) -> bool {
    match fetched {
        Ok(next) => {
            let mut current = shared.write().unwrap_or_else(PoisonError::into_inner);
            if **current == next {
                return false;
            }
            *current = Arc::new(next);
            tracing::info!("image-resize configuration reloaded");
            true
        }
        Err(error) => {
            tracing::error!(%error, "image-resize configuration reload failed; keeping the previous one");
            false
        }
    }
}

/// Observe the entry's updates and re-read it on each. Run the result once
/// after binding: an update between the boot fetch and the observer reached no one.
pub fn bind_reload(iii: &Arc<IIIClient>, shared: SharedConfig) -> Reload {
    let engine = iii.clone();
    iii_config_client::on_change(
        iii,
        config_id(),
        RELOAD_FN_ID,
        "Internal: re-read the image-resize configuration after a change.",
        move || {
            let (engine, shared) = (engine.clone(), shared.clone());
            async move {
                apply(&shared, fetch(&engine).await);
            }
        },
    )
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn config(value: serde_json::Value) -> ResizeConfig {
        ResizeConfig::from_json(&value).unwrap()
    }

    #[test]
    fn apply_keeps_the_previous_config_on_error() {
        let first = config(json!({"width": 320}));
        let cell = shared(first.clone());
        assert!(!apply(&cell, Err("configuration::get timed out".into())));
        assert!(!apply(&cell, ResizeConfig::from_json(&json!({"width": 0}))));
        assert_eq!(*snapshot(&cell), first);
    }

    #[test]
    fn apply_swaps_a_changed_config() {
        let cell = shared(ResizeConfig::default());
        let next = config(json!({"strategy": "crop-to-fit"}));
        assert!(apply(&cell, Ok(next.clone())));
        assert_eq!(*snapshot(&cell), next);
        assert!(!apply(&cell, Ok(next)));
    }

    #[test]
    fn a_snapshot_outlives_a_reload() {
        let cell = shared(ResizeConfig::default());
        let before = snapshot(&cell);
        assert!(apply(&cell, Ok(config(json!({"width": 64})))));
        assert_eq!(*before, ResizeConfig::default());
        assert_eq!(snapshot(&cell).width.get(), 64);
    }

    #[test]
    fn config_id_prefers_a_nonblank_assigned_name() {
        assert_eq!(resolve_config_id(Some(" team-a ".into())), "team-a");
        assert_eq!(resolve_config_id(Some("  ".into())), "image-resize");
        assert_eq!(resolve_config_id(None), "image-resize");
    }
}
