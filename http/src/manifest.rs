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
        description: "Expose functions as HTTP endpoints.".to_string(),
        default_config: crate::config::RestApiConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn manifest_has_required_fields() {
        let v = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(v["name"], env!("CARGO_PKG_NAME"));
        assert!(v["default_config"]["port"].is_number());
        assert!(!v["supported_targets"].as_array().unwrap().is_empty());
    }
}
