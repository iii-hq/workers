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
        description: "III engine OTel-native evaluation worker".to_string(),
        default_config: serde_json::json!({
            "retention_hours": 24,
            "drift_threshold": 0.15,
            "cron_drift_check": "0 */10 * * * *",
            "max_spans_per_function": 1000,
            "baseline_window_minutes": 60
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
        assert_eq!(parsed["name"], "iii-eval");
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        let json = serde_json::to_string_pretty(&manifest).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["default_config"]["retention_hours"], 24);
        assert_eq!(parsed["default_config"]["drift_threshold"], 0.15);
        assert_eq!(parsed["default_config"]["max_spans_per_function"], 1000);
        assert!(!manifest.supported_targets.is_empty());
    }
}
