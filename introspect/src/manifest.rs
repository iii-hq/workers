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
            "III engine introspection worker — registry discovery, topology maps, and health checks"
                .to_string(),
        default_config: serde_json::json!({
            "class": "modules::introspect::IntrospectModule",
            "config": {
                "cron_topology_refresh": "0 */5 * * * *",
                "cache_ttl_seconds": 30
            }
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_manifest_json_output() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed.is_object());
        assert_eq!(parsed["name"], "iii-introspect");
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["default_config"]["class"].is_string());
        assert_eq!(
            parsed["default_config"]["config"]["cron_topology_refresh"],
            "0 */5 * * * *"
        );
        assert_eq!(parsed["default_config"]["config"]["cache_ttl_seconds"], 30);
        assert!(!manifest.supported_targets.is_empty());
    }
}
