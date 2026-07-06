//! Manifest parsing and validation for the bridge worker.

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
        description: "Bridge functions to and from a remote iii instance.".to_string(),
        default_config: crate::config::BridgeConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_has_required_fields() {
        let v = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(v["name"], "iii-bridge");
        assert!(v["default_config"].is_object());
        assert!(!v["supported_targets"].as_array().unwrap().is_empty());
    }
}
