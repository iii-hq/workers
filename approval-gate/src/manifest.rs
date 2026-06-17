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
            "Policy and decision surface for human-held function calls — pre_trigger gate, pending inbox, per-session permission settings, and two notification trigger types."
                .to_string(),
        // The shipped configuration defaults, straight from the source of
        // truth so the published manifest can never drift from the worker.
        default_config: crate::config::WorkerConfig::default().to_json(),
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
        let from_manifest: WorkerConfig = serde_json::from_value(m.default_config.clone()).unwrap();
        assert_eq!(from_manifest, WorkerConfig::default());
    }
}
