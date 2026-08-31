use serde::Serialize;

use crate::config::WorkerConfig;

pub const DESCRIPTION: &str =
    "Compose, validate, persist, and render A2UI v0.9.1 surfaces for iii Harness sessions in chat and on an injectable Console page.";

#[derive(Debug, Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: DESCRIPTION.into(),
        default_config: WorkerConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_carries_registry_fields() {
        let value = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(value["name"], "a2ui");
        assert!(value["version"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(value["default_config"].is_object());
        assert!(value["supported_targets"]
            .as_array()
            .is_some_and(|value| !value.is_empty()));
    }
}
