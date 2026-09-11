//! Publish-time manifest emitted by `console --manifest`.
//!
//! Five required fields: `name`, `version`, `description`,
//! `default_config` (object), `supported_targets` (non-empty array).
//! `default_config` mirrors YAML-backed defaults (`http_port` only);
//! the engine WebSocket URL is a CLI flag (`--url`), not config file.

use serde::Serialize;

use crate::config::ConsoleConfig;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    let cfg = ConsoleConfig::default();
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description:
            "Web console for iii — bundles the React UI and proxies the engine WebSocket on a single port."
                .to_string(),
        default_config: serde_json::json!({
            "http_port": cfg.http_port,
        }),
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
        assert!(
            parsed["description"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "description must be a non-empty string"
        );
        assert!(
            parsed["default_config"].is_object(),
            "default_config must be an object"
        );
        assert!(
            !parsed["supported_targets"]
                .as_array()
                .expect("supported_targets must be an array")
                .is_empty(),
            "supported_targets must not be empty"
        );
    }

    #[test]
    fn default_config_mirrors_runtime_defaults() {
        let m = build_manifest();
        let cfg = ConsoleConfig::default();
        assert_eq!(m.default_config["http_port"], cfg.http_port);
    }
}
