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
        description: "III engine KVM CoW fork sandbox for sub-millisecond VM spawning".to_string(),
        default_config: serde_json::json!({
            "class": "modules::sandbox_firecracker::SandboxFirecrackerModule",
            "config": {
                "vmstate_path": "./template/vmstate",
                "memfile_path": "./template/mem",
                "mem_size_mb": 256,
                "default_timeout": 30,
                "max_sandboxes": 1000,
                "max_cmd_timeout": 60,
                "max_output_bytes": 1048576,
                "default_language": "python"
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
        assert_eq!(parsed["name"], "sandbox-firecracker");
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["default_config"]["class"].is_string());
        assert_eq!(
            parsed["default_config"]["config"]["vmstate_path"],
            "./template/vmstate"
        );
        assert_eq!(parsed["default_config"]["config"]["mem_size_mb"], 256);
        assert_eq!(parsed["default_config"]["config"]["default_timeout"], 30);
        assert_eq!(parsed["default_config"]["config"]["max_sandboxes"], 1000);
        assert_eq!(
            parsed["default_config"]["config"]["default_language"],
            "python"
        );
        assert!(!manifest.supported_targets.is_empty());
    }
}
