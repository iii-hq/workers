use serde::Serialize;

const DESCRIPTION: &str = "Command Code dual-protocol provider worker behind llm-router.";

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
        default_config: serde_json::json!({}),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_the_registry_fields() {
        let value = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(value["name"], "provider-command-code");
        assert!(!value["version"].as_str().unwrap().is_empty());
        assert!(!value["description"].as_str().unwrap().is_empty());
        assert!(value["default_config"].is_object());
        assert!(!value["supported_targets"].as_array().unwrap().is_empty());
    }
}
