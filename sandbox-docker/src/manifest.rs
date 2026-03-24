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
        description: "III engine Docker sandbox worker".to_string(),
        default_config: serde_json::json!({
            "class": "modules::sandbox_docker::SandboxDockerModule",
            "config": {
                "default_image": "python:3.12-slim",
                "default_timeout": 3600,
                "default_memory": 512,
                "default_cpu": 1.0,
                "max_sandboxes": 50,
                "max_cmd_timeout": 300,
                "workspace_dir": "/workspace",
                "pool_size": 0
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
        assert!(parsed.is_object(), "Manifest must be valid JSON object");
        assert_eq!(parsed["name"], "sandbox-docker");
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["default_config"]["class"].is_string());
        assert_eq!(
            parsed["default_config"]["config"]["default_image"],
            "python:3.12-slim"
        );
        assert_eq!(parsed["default_config"]["config"]["default_timeout"], 3600);
        assert_eq!(parsed["default_config"]["config"]["default_memory"], 512);
        assert_eq!(parsed["default_config"]["config"]["max_sandboxes"], 50);
        assert_eq!(
            parsed["default_config"]["config"]["workspace_dir"],
            "/workspace"
        );
        assert!(!manifest.supported_targets.is_empty());
    }
}
