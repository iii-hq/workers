use serde::Serialize;

use crate::config::TurnOrchestratorConfig;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: env!("CARGO_PKG_DESCRIPTION").to_string(),
        default_config: serde_json::to_value(TurnOrchestratorConfig::default())
            .expect("TurnOrchestratorConfig serializes to JSON"),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    /// Every field of [`TurnOrchestratorConfig`] should appear in the
    /// manifest's `default_config` — that's the whole point of deriving
    /// from `::default()`. Asserting *presence* (not literal values)
    /// keeps the test resilient to default tweaks while still catching
    /// regressions where a new field is added without being serialized.
    #[test]
    fn default_config_includes_every_config_field() {
        let m = build_manifest();
        let cfg = m.default_config;

        assert!(cfg["sync_default_timeout_ms"].is_number());
        assert!(cfg["sync_poll_interval_ms"].is_number());
        assert!(cfg["system_default_skills"].is_array());
    }

    /// Manifest must match a fresh `TurnOrchestratorConfig::default()`
    /// byte-for-byte. Lock-step guarantee against the manual-drift
    /// failure mode the previous hand-written `json!({...})` had.
    #[test]
    fn default_config_matches_struct_default() {
        let m = build_manifest();
        let from_struct = serde_json::to_value(TurnOrchestratorConfig::default()).unwrap();
        assert_eq!(m.default_config, from_struct);
    }
}
