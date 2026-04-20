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
        description: "III engine code quality sensor — scans, scores, baselines, and gates"
            .to_string(),
        default_config: serde_json::json!({
            "scan_extensions": ["rs", "ts", "py", "js", "go"],
            "max_file_size_kb": 512,
            "score_weights": {
                "complexity": 0.25,
                "coupling": 0.25,
                "cohesion": 0.20,
                "size": 0.15,
                "duplication": 0.15
            },
            "thresholds": {
                "degradation_pct": 10.0,
                "min_score": 60.0
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
        assert_eq!(parsed["name"], "iii-sensor");
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["default_config"]["scan_extensions"].is_array());
        assert_eq!(parsed["default_config"]["max_file_size_kb"], 512);
        assert_eq!(parsed["default_config"]["score_weights"]["complexity"], 0.25);
        assert_eq!(parsed["default_config"]["thresholds"]["min_score"], 60.0);
        assert!(!manifest.supported_targets.is_empty());
    }
}
