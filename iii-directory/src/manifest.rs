//! `--manifest` subcommand output. Same contract as every binary worker.

use serde::Serialize;

use crate::config::{DEFAULT_LOCAL_SKILLS_FOLDER, DEFAULT_REGISTRY_URL, DEFAULT_SKILLS_FOLDER};

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
        description: "Engine introspection (functions / triggers / workers), workers \
             registry proxy, and filesystem-backed skill + prompt reader."
            .to_string(),
        default_config: serde_json::json!({
            "skills_folder": DEFAULT_SKILLS_FOLDER,
            "local_skills_folder": DEFAULT_LOCAL_SKILLS_FOLDER,
            "registry_url": DEFAULT_REGISTRY_URL,
            "download_timeout_ms": 60_000,
            "registry_cache_ttl_ms": 60_000,
            "filter_unregistered": false,
            "auto_download": true,
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
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert_eq!(
            parsed["default_config"]["skills_folder"],
            DEFAULT_SKILLS_FOLDER
        );
        assert_eq!(
            parsed["default_config"]["registry_url"],
            DEFAULT_REGISTRY_URL
        );
        assert_eq!(parsed["default_config"]["download_timeout_ms"], 60_000);
        assert_eq!(parsed["default_config"]["registry_cache_ttl_ms"], 60_000);
        assert_eq!(parsed["default_config"]["filter_unregistered"], false);
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }
}
