//! Authoritative configuration reloads without interrupting native sessions.
use std::sync::{Arc, RwLock};

use anyhow::{Context, Result};
use iii_sdk::{IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{
    config::{Config, config_id},
    sessions::SessionManager,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ConfigSource {
    Central,
    Local,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ReloadOutcome {
    Applied,
    RestartRequired,
    Rejected,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct ConfigStatus {
    pub id: String,
    pub source: ConfigSource,
    pub last_outcome: ReloadOutcome,
    pub last_error: Option<String>,
    pub rejected_reloads: u64,
    pub restart_required_fields: Vec<String>,
    /// The actual live settings, including retained structural values.
    pub active: Config,
}

impl ConfigStatus {
    fn new(manager: &SessionManager, local: bool) -> Self {
        Self {
            id: config_id().to_owned(),
            source: if local {
                ConfigSource::Local
            } else {
                ConfigSource::Central
            },
            last_outcome: ReloadOutcome::Applied,
            last_error: None,
            rejected_reloads: 0,
            restart_required_fields: Vec::new(),
            active: manager.config(),
        }
    }
}

// Ignore engine-injected routing metadata while retaining a concrete request schema.
#[derive(Deserialize, JsonSchema)]
struct ConfigStatusRequest {}

/// Register operator status and, for central configuration, the typed reload hook.
/// Call after the regular function surface and event service are registered.
pub async fn register(
    iii: &Arc<IIIClient>,
    manager: &Arc<SessionManager>,
    local: bool,
) -> Result<()> {
    let status = Arc::new(RwLock::new(ConfigStatus::new(manager, local)));
    let for_status = status.clone();
    iii.register_function(
        "whatsapp::config-status",
        RegisterFunction::new(move |_request: ConfigStatusRequest| {
            Ok::<_, iii_sdk::Error>(for_status.read().unwrap().clone())
        })
        .description(
            "Report active configuration, rejected reloads, and fields pending a worker restart",
        )
        .metadata(json!({"internal": true, "trace_hidden": true})),
    );
    if local {
        return Ok(());
    }

    let for_reload = iii.clone();
    let manager = manager.clone();
    let reload = iii_config_client::on_change(
        iii,
        config_id(),
        "whatsapp::on-config-change",
        "Internal: re-read authoritative WhatsApp configuration and apply live resource limits",
        move || {
            let iii = for_reload.clone();
            let manager = manager.clone();
            let status = status.clone();
            async move {
                // The shared helper serializes the entire closure, including this fetch.
                // Trigger payloads can never supply or override configuration values.
                let fetched = iii_config_client::fetch(&iii, config_id()).await;
                apply_fetched(&manager, &status, fetched);
            }
        },
    )?;
    // A change between the startup fetch and trigger binding must also be observed.
    reload.run().await;
    Ok(())
}

fn apply_fetched(
    manager: &SessionManager,
    status: &RwLock<ConfigStatus>,
    fetched: std::result::Result<Option<Value>, String>,
) {
    let result = fetched
        .map_err(anyhow::Error::msg)
        .and_then(|value| value.context("missing WhatsApp configuration"))
        .and_then(|value| Config::from_json(&value))
        .and_then(|config| manager.apply_config(config));
    let mut status = status.write().unwrap();
    match result {
        Ok(restart_required_fields) => {
            status.last_outcome = if restart_required_fields.is_empty() {
                ReloadOutcome::Applied
            } else {
                ReloadOutcome::RestartRequired
            };
            status.last_error = None;
            status.restart_required_fields = restart_required_fields;
            status.active = manager.config();
        }
        Err(error) => {
            status.last_outcome = ReloadOutcome::Rejected;
            status.last_error = Some(format!("{error:#}"));
            status.rejected_reloads = status.rejected_reloads.saturating_add(1);
            tracing::warn!(%error, "WhatsApp configuration reload rejected; retaining active settings");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn malformed_missing_and_failed_reads_preserve_last_good_runtime_and_report_errors() {
        let directory = TempDir::new().unwrap();
        let initial = Config {
            data_dir: directory.path().join("sessions"),
            ..Config::default()
        };
        let manager = SessionManager::open(initial.clone()).unwrap();
        let status = RwLock::new(ConfigStatus::new(&manager, false));
        for fetched in [
            Ok(Some(json!({"max_sessions": 0}))),
            Ok(Some(json!({"unexpected": true}))),
            Ok(None),
            Err("configuration service unavailable".to_owned()),
        ] {
            apply_fetched(&manager, &status, fetched);
            assert_eq!(manager.config(), initial);
            let status = status.read().unwrap();
            assert_eq!(status.active, initial);
            assert_eq!(status.last_outcome, ReloadOutcome::Rejected);
            assert!(status.last_error.is_some());
        }
        assert_eq!(status.read().unwrap().rejected_reloads, 4);

        let changed = Config {
            max_media_bytes: 1024,
            ..initial
        };
        apply_fetched(
            &manager,
            &status,
            Ok(Some(serde_json::to_value(&changed).unwrap())),
        );
        let status = status.read().unwrap();
        assert_eq!(status.active, changed);
        assert_eq!(status.last_outcome, ReloadOutcome::Applied);
        assert_eq!(status.last_error, None);
        assert_eq!(status.rejected_reloads, 4);
    }

    #[test]
    fn structural_changes_remain_visible_while_live_tuning_applies_and_can_be_reverted() {
        let directory = TempDir::new().unwrap();
        let initial = Config {
            data_dir: directory.path().join("sessions"),
            ..Config::default()
        };
        let manager = SessionManager::open(initial.clone()).unwrap();
        let status = RwLock::new(ConfigStatus::new(&manager, false));
        let changed = Config {
            data_dir: directory.path().join("next-start"),
            max_events: 5,
            max_inbox_messages: 50,
            max_sessions: 2,
            max_media_bytes: 2048,
            operation_timeout_secs: 10,
            shutdown_timeout_secs: 3,
        };
        apply_fetched(
            &manager,
            &status,
            Ok(Some(serde_json::to_value(&changed).unwrap())),
        );
        {
            let status = status.read().unwrap();
            assert_eq!(status.last_outcome, ReloadOutcome::RestartRequired);
            assert_eq!(status.restart_required_fields, ["data_dir", "max_events"]);
            assert_eq!(status.active.data_dir, initial.data_dir);
            assert_eq!(status.active.max_events, initial.max_events);
            assert_eq!(status.active.max_sessions, 2);
            assert_eq!(status.active.max_media_bytes, 2048);
            assert_eq!(status.active.operation_timeout_secs, 10);
            assert_eq!(status.active.shutdown_timeout_secs, 3);
            assert_eq!(status.last_error, None);
            assert_eq!(status.rejected_reloads, 0);
        }
        assert!(!changed.data_dir.exists());

        apply_fetched(
            &manager,
            &status,
            Ok(Some(serde_json::to_value(&initial).unwrap())),
        );
        let status = status.read().unwrap();
        assert_eq!(status.last_outcome, ReloadOutcome::Applied);
        assert!(status.restart_required_fields.is_empty());
        assert_eq!(status.active, initial);
    }
}
