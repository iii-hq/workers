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
        description: "III engine image resize module".to_string(),
        default_config: serde_json::json!({
            "class": "modules::image_resize::ImageResizeModule",
            "config": {
                "width": 200,
                "height": 200,
                "strategy": "scale-to-fit",
                "quality": {
                    "jpeg": 85,
                    "webp": 80
                }
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
        assert_eq!(parsed["name"], "image-resize");
        assert_eq!(parsed["version"], "0.1.0");
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["default_config"]["class"].is_string());
        assert_eq!(parsed["default_config"]["config"]["width"], 200);
        assert_eq!(parsed["default_config"]["config"]["height"], 200);
        assert_eq!(
            parsed["default_config"]["config"]["strategy"],
            "scale-to-fit"
        );
        assert_eq!(parsed["default_config"]["config"]["quality"]["jpeg"], 85);
        assert_eq!(parsed["default_config"]["config"]["quality"]["webp"], 80);
        assert!(!manifest.supported_targets.is_empty());
    }
}
