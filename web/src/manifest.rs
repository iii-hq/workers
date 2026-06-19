//! `--manifest` subcommand output. Same contract as every binary worker.

use serde::Serialize;

use crate::config::WebConfig;

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
        description: "Outbound HTTP client on the iii bus (web::fetch).".to_string(),
        default_config: WebConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_has_required_fields() {
        let m = build_manifest();
        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
        assert!(v["description"].as_str().is_some_and(|s| !s.is_empty()));
        assert_eq!(v["default_config"]["user_agent"], "iii-harness/0.1 (+web::fetch)");
        assert!(!v["supported_targets"].as_array().unwrap().is_empty());
    }
}
