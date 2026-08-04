use serde::Serialize;

use crate::config::CodeRunnerConfig;

#[derive(Serialize)]
pub struct ModuleManifest {
    pub name: String,
    pub version: String,
    pub description: String,
    pub default_config: serde_json::Value,
    pub supported_targets: Vec<String>,
}

pub fn build_manifest() -> ModuleManifest {
    let d = CodeRunnerConfig::default();
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Run Node.js and Python in iii-sandbox microVMs: eval code, register \
                      bus functions whose handlers execute inside the VM, tear down."
            .to_string(),
        default_config: serde_json::json!({
            "default_timeout_ms": d.default_timeout_ms,
            "max_timeout_ms": d.max_timeout_ms,
            "idle_ttl_secs": d.idle_ttl_secs,
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let json = serde_json::to_string_pretty(&build_manifest()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(!parsed["description"].as_str().unwrap().is_empty());
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_mirrors_struct_defaults() {
        let m = build_manifest();
        let d = CodeRunnerConfig::default();
        assert_eq!(m.default_config["default_timeout_ms"], d.default_timeout_ms);
        assert_eq!(m.default_config["idle_ttl_secs"], d.idle_ttl_secs);
    }
}
