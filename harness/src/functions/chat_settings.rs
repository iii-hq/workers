//! `harness::chat-settings::get|set|on-changed` (chat-projects §1.2.5): the
//! Chat settings devices show, kept in the harness's own configuration entry
//! (`configuration::get|set {id: "harness"}`). `set` reads the stored entry,
//! merges the patch and writes it on the server, so a device never sends the
//! whole harness configuration. The configuration change then reaches every
//! harness through `configuration::on-updated`, which calls
//! `harness::chat-settings::on-changed` when these settings changed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::config::{DescriptionsConfig, WorkerConfig};
use crate::deps::Deps;
use crate::error::HarnessError;

pub const GET_ID: &str = "harness::chat-settings::get";
pub const GET_DESC: &str =
    "Internal control-plane: the Chat settings, { descriptions: { agent_writes } }.";
pub const SET_ID: &str = "harness::chat-settings::set";
pub const SET_DESC: &str = "Internal control-plane: change the Chat settings with a patch, { patch: { descriptions: \
     { agent_writes } } }, merged into the harness configuration on the server. Answers { settings }.";
pub const ON_CHANGED_FN: &str = "harness::chat-settings::on-changed";
pub const ON_CHANGED_DESC: &str =
    "The Chat settings changed (from any device, or the harness configuration): { settings }.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatSettings {
    pub descriptions: DescriptionsSettings,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DescriptionsSettings {
    /// Agents keep conversation descriptions up to date
    /// (`harness::set-description` is offered to the model).
    pub agent_writes: bool,
}

impl ChatSettings {
    pub fn of(cfg: &WorkerConfig) -> Self {
        Self {
            descriptions: DescriptionsSettings {
                agent_writes: cfg.descriptions.agent_writes,
            },
        }
    }
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct ChatSettingsGetRequest {}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct ChatSettingsSetRequest {
    /// The settings to change; keys not named keep their value.
    #[schemars(with = "Map<String, Value>")]
    pub patch: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatSettingsSetResponse {
    pub settings: ChatSettings,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ChatSettingsChangedEvent {
    pub settings: ChatSettings,
}

pub async fn get(
    deps: &Deps,
    _request: ChatSettingsGetRequest,
) -> Result<ChatSettings, HarnessError> {
    Ok(ChatSettings::of(&*deps.cfg().await))
}

/// Merge `patch` into the stored configuration value (raw, so `${VAR}`
/// placeholders stay as written). Only the Chat settings' keys are accepted.
pub fn merge_patch(stored: &Value, patch: &Value) -> Result<Value, HarnessError> {
    let patch = patch.as_object().ok_or_else(|| {
        HarnessError::InvalidRequest("chat-settings patch must be an object".into())
    })?;
    let mut merged = match stored {
        Value::Object(map) => map.clone(),
        Value::Null => Map::new(),
        _ => {
            return Err(HarnessError::Internal(
                "the stored harness configuration is not an object".into(),
            ));
        }
    };
    for (key, value) in patch {
        match key.as_str() {
            "descriptions" => {
                let changes = value.as_object().ok_or_else(|| {
                    HarnessError::InvalidRequest("descriptions must be an object".into())
                })?;
                let mut descriptions = merged
                    .get("descriptions")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for (field, value) in changes {
                    match field.as_str() {
                        "agent_writes" if value.is_boolean() => {
                            descriptions.insert(field.clone(), value.clone());
                        }
                        "agent_writes" => {
                            return Err(HarnessError::InvalidRequest(
                                "descriptions.agent_writes must be true or false".into(),
                            ));
                        }
                        other => {
                            return Err(HarnessError::InvalidRequest(format!(
                                "unknown chat setting descriptions.{other}"
                            )));
                        }
                    }
                }
                merged.insert("descriptions".into(), Value::Object(descriptions));
            }
            other => {
                return Err(HarnessError::InvalidRequest(format!(
                    "unknown chat setting `{other}`; this harness has: descriptions"
                )));
            }
        }
    }
    Ok(Value::Object(merged))
}

/// One `set` at a time in this process: read, merge and write the entry.
static SET_GATE: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

pub async fn set(
    deps: &Deps,
    request: ChatSettingsSetRequest,
) -> Result<ChatSettingsSetResponse, HarnessError> {
    let _gate = SET_GATE.lock().await;
    let stored = crate::configuration::read_stored(&deps.iii)
        .await
        .map_err(HarnessError::Dependency)?;
    let merged = merge_patch(&stored, &request.patch)?;
    let descriptions: DescriptionsConfig = serde_json::from_value(
        merged
            .get("descriptions")
            .cloned()
            .unwrap_or_else(|| json!({})),
    )
    .map_err(|error| HarnessError::InvalidRequest(format!("descriptions: {error}")))?;
    if merged != stored {
        crate::configuration::write_stored(&deps.iii, merged)
            .await
            .map_err(HarnessError::Dependency)?;
    }
    Ok(ChatSettingsSetResponse {
        settings: ChatSettings {
            descriptions: DescriptionsSettings {
                agent_writes: descriptions.agent_writes,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_lets_agents_write_descriptions() {
        let settings = ChatSettings::of(&WorkerConfig::default());
        assert_eq!(
            serde_json::to_value(settings).unwrap(),
            json!({ "descriptions": { "agent_writes": true } })
        );
        let stored: WorkerConfig = serde_json::from_value(json!({})).unwrap();
        assert!(stored.descriptions.agent_writes);
    }

    /// The merge happens on the server: every other key of the entry stays,
    /// placeholders included.
    #[test]
    fn set_merges_only_the_named_settings() {
        let stored = json!({
            "default_max_turns": 9,
            "folders_file_path": "${III_DATA}/folders.json",
            "descriptions": { "agent_writes": true },
        });
        let merged = merge_patch(
            &stored,
            &json!({ "descriptions": { "agent_writes": false } }),
        )
        .unwrap();
        assert_eq!(
            merged,
            json!({
                "default_max_turns": 9,
                "folders_file_path": "${III_DATA}/folders.json",
                "descriptions": { "agent_writes": false },
            })
        );
        assert_eq!(
            merge_patch(
                &Value::Null,
                &json!({ "descriptions": { "agent_writes": false } })
            )
            .unwrap(),
            json!({ "descriptions": { "agent_writes": false } })
        );
        assert_eq!(merge_patch(&stored, &json!({})).unwrap(), stored);
        for bad in [
            json!({ "default_max_turns": 1 }),
            json!({ "descriptions": { "agent_writes": "no" } }),
            json!({ "descriptions": { "other": true } }),
            json!({ "descriptions": true }),
            json!([]),
        ] {
            assert!(merge_patch(&stored, &bad).is_err(), "{bad}");
        }
        // The merged value still parses as the harness configuration.
        let parsed =
            WorkerConfig::from_json(&json!({ "descriptions": { "agent_writes": false } })).unwrap();
        assert!(!parsed.descriptions.agent_writes);
    }
}
