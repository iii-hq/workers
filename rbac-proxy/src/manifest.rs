//! Publish-time manifest emitted by `rbac-proxy --manifest`.
//!
//! Five required fields: `name`, `version`, `description`, `default_config`
//! (object), `supported_targets` (non-empty array). `default_config` is the
//! full `WorkerConfig::default()` — the same value seeded into the
//! `configuration` worker on first boot.

use serde::Serialize;

use crate::config::WorkerConfig;

pub const DESCRIPTION: &str = "RBAC boundary proxy for the iii worker protocol — auth, gating, namespacing, middleware, and engine:: result filtering on its own port.";

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
        description: DESCRIPTION.to_string(),
        default_config: WorkerConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let parsed: serde_json::Value =
            serde_json::from_str(&serde_json::to_string(&m).unwrap()).unwrap();

        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"].as_str().is_some_and(|s| !s.is_empty()));
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"]
            .as_array()
            .expect("supported_targets array")
            .is_empty());
    }

    #[test]
    fn default_config_mirrors_worker_config_default() {
        let m = build_manifest();
        assert_eq!(m.default_config, WorkerConfig::default().to_json());
        // Spot-check the public port surfaces.
        assert_eq!(m.default_config["port"], 49200);
    }
}
