//! `--manifest` subcommand output for the registry publish pipeline.

use serde::Serialize;

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
        description:
            "Durable, reactive, branching store of typed conversation entries with six emitted trigger types."
                .to_string(),
        // Mirrors config::WorkerConfig::default() field-for-field,
        // with the adapter spelled out in its resolved fs shape.
        default_config: serde_json::json!({
            "adapter": {
                "name": "fs",
                "config": {
                    "data_dir": "~/.iii/data/session-manager",
                },
            },
            "default_list_limit": 50,
            "max_list_limit": 500,
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()));
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_mirrors_worker_config_default() {
        let m = build_manifest();
        let cfg = WorkerConfig::default();
        assert_eq!(m.default_config["adapter"]["name"], serde_json::json!("fs"));
        assert_eq!(
            m.default_config["adapter"]["config"]["data_dir"],
            serde_json::json!(crate::config::default_data_dir())
        );
        assert_eq!(
            m.default_config["default_list_limit"],
            serde_json::json!(cfg.default_list_limit)
        );
        assert_eq!(
            m.default_config["max_list_limit"],
            serde_json::json!(cfg.max_list_limit)
        );
        // The manifest's hand-written default must stay byte-for-byte the
        // serialized default config (catches adapter-shape drift).
        assert_eq!(m.default_config, cfg.to_json());
    }
}
