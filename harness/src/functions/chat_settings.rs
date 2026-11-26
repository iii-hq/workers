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

use crate::config::{AutocompactConfig, DescriptionsConfig, WorkerConfig};
use crate::deps::Deps;
use crate::error::HarnessError;

pub const GET_ID: &str = "harness::chat-settings::get";
pub const GET_DESC: &str = "Internal control-plane: the Chat settings, { descriptions: { agent_writes }, \
     autocompact: { mode: \"off\" | \"tokens\", tokens?, instructions? } }.";
pub const SET_ID: &str = "harness::chat-settings::set";
pub const SET_DESC: &str = "Internal control-plane: change the Chat settings with a patch, { patch: { descriptions: \
     { agent_writes }, autocompact: { mode, tokens, instructions } } }, merged into the harness configuration \
     on the server (an autocompact field set to null is removed). Refused with `bad_setting` (an unknown \
     mode, or `tokens` mode without tokens), `bad_tokens` (outside 20,000-2,000,000) or `bad_instructions` \
     (over 1,000 characters). Answers { settings }.";
pub const ON_CHANGED_FN: &str = "harness::chat-settings::on-changed";
pub const ON_CHANGED_DESC: &str =
    "The Chat settings changed (from any device, or the harness configuration): { settings }.";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ChatSettings {
    pub descriptions: DescriptionsSettings,
    /// The global autocompact setting (chat-projects phase 3): `off` (the
    /// default) or `tokens`; `tokens` and `instructions` stay while off.
    #[serde(default)]
    pub autocompact: AutocompactConfig,
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
            autocompact: cfg.autocompact.clone(),
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
            "autocompact" => {
                let changes = value.as_object().ok_or_else(|| {
                    HarnessError::rejected(
                        "bad_setting",
                        "Choose a size in tokens, or off.",
                        json!({ "field": "autocompact", "autocompact": value }),
                    )
                })?;
                let mut autocompact = merged
                    .get("autocompact")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                for (field, value) in changes {
                    if value.is_null() {
                        autocompact.remove(field);
                    } else {
                        autocompact.insert(field.clone(), value.clone());
                    }
                }
                let checked = crate::autocompact::validate_global(&Value::Object(autocompact))?;
                // The default (off, no size, no instructions) is left out of
                // the stored entry, so a harness older than 1.13 still boots.
                if checked.is_default() {
                    merged.remove("autocompact");
                } else {
                    let stored = serde_json::to_value(checked).map_err(|error| {
                        HarnessError::Internal(format!("autocompact setting: {error}"))
                    })?;
                    merged.insert("autocompact".into(), stored);
                }
            }
            other => {
                return Err(HarnessError::InvalidRequest(format!(
                    "unknown chat setting `{other}`; this harness has: descriptions, autocompact"
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
    let autocompact: AutocompactConfig = match merged.get("autocompact") {
        Some(value) => crate::autocompact::validate_global(value)?,
        None => AutocompactConfig::default(),
    };
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
            autocompact,
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
            json!({ "descriptions": { "agent_writes": true }, "autocompact": { "mode": "off" } })
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

    /// The global autocompact setting merges field by field, `null` removes
    /// a field, and what is stored is the checked value.
    #[test]
    fn set_merges_and_checks_the_autocompact_setting() {
        let stored = json!({ "default_max_turns": 9 });
        let on = merge_patch(
            &stored,
            &json!({ "autocompact": { "mode": "tokens", "tokens": 160_000, "instructions": "  keep the plan  " } }),
        )
        .unwrap();
        assert_eq!(
            on,
            json!({
                "default_max_turns": 9,
                "autocompact": { "mode": "tokens", "tokens": 160_000, "instructions": "keep the plan" },
            })
        );
        // Off keeps the size and the instructions for turning it back on.
        let off = merge_patch(&on, &json!({ "autocompact": { "mode": "off" } })).unwrap();
        assert_eq!(
            off["autocompact"],
            json!({ "mode": "off", "tokens": 160_000, "instructions": "keep the plan" })
        );
        let cleared =
            merge_patch(&off, &json!({ "autocompact": { "instructions": null } })).unwrap();
        assert_eq!(
            cleared["autocompact"],
            json!({ "mode": "off", "tokens": 160_000 })
        );
        // The default (off, no size, no instructions) is not stored, so a
        // harness older than 1.13 still boots on the entry.
        let default = merge_patch(&cleared, &json!({ "autocompact": { "tokens": null } })).unwrap();
        assert_eq!(default, stored, "the default leaves the entry as it was");
        let never_set = merge_patch(
            &stored,
            &json!({ "autocompact": { "mode": "off", "instructions": "" } }),
        )
        .unwrap();
        assert_eq!(never_set, stored);
        // The ADE sends the whole object: `tokens` stays with `mode: "off"`
        // (inert until it is turned back on), `instructions: ""` clears.
        let whole = merge_patch(
            &on,
            &json!({ "autocompact": { "mode": "off", "tokens": 200_000, "instructions": "" } }),
        )
        .unwrap();
        assert_eq!(
            whole["autocompact"],
            json!({ "mode": "off", "tokens": 200_000 })
        );
        let parsed = WorkerConfig::from_json(&whole).unwrap();
        assert_eq!(
            crate::autocompact::global_level(&parsed.autocompact),
            crate::autocompact::Level::Off
        );
        assert_eq!(
            crate::autocompact::global_instructions(&parsed.autocompact),
            None
        );
        let back_on = merge_patch(
            &whole,
            &json!({ "autocompact": { "mode": "tokens", "tokens": 200_000 } }),
        )
        .unwrap();
        assert_eq!(
            crate::autocompact::global_level(
                &WorkerConfig::from_json(&back_on).unwrap().autocompact
            ),
            crate::autocompact::Level::Tokens(200_000)
        );
        let parsed = WorkerConfig::from_json(&on).unwrap();
        assert_eq!(parsed.autocompact.tokens, Some(160_000));
        assert_eq!(
            ChatSettings::of(&parsed).autocompact.mode,
            crate::config::AutocompactMode::Tokens
        );
        for (patch, code) in [
            (
                json!({ "autocompact": { "mode": "tokens" } }),
                "bad_setting",
            ),
            (
                json!({ "autocompact": { "mode": "inherit" } }),
                "bad_setting",
            ),
            (
                json!({ "autocompact": { "mode": "sometimes" } }),
                "bad_setting",
            ),
            (json!({ "autocompact": true }), "bad_setting"),
            (json!({ "autocompact": { "colour": "red" } }), "bad_setting"),
            (
                json!({ "autocompact": { "mode": "tokens", "tokens": 19_999 } }),
                "bad_tokens",
            ),
            (
                json!({ "autocompact": { "mode": "tokens", "tokens": 2_000_001 } }),
                "bad_tokens",
            ),
            (
                json!({ "autocompact": { "mode": "tokens", "tokens": "160k" } }),
                "bad_tokens",
            ),
            (
                json!({ "autocompact": { "instructions": "x".repeat(1_001) } }),
                "bad_instructions",
            ),
        ] {
            let error = merge_patch(&stored, &patch).unwrap_err();
            assert_eq!(error.rejected_code().as_deref(), Some(code), "{patch}");
        }
        // Exactly 1,000 characters is allowed.
        merge_patch(
            &stored,
            &json!({ "autocompact": { "instructions": "é".repeat(1_000) } }),
        )
        .unwrap();
    }
}
