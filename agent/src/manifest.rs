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
        description: "III engine AI agent — chat orchestrator with dynamic function discovery"
            .to_string(),
        default_config: serde_json::json!({
            "anthropic_model": "claude-sonnet-4-20250514",
            "max_tokens": 4096,
            "max_iterations": 10,
            "session_ttl_hours": 24,
            "cron_session_cleanup": "0 0 * * * *"
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
        assert_eq!(parsed["name"], "iii-agent");
    }

    #[test]
    fn test_manifest_has_required_fields() {
        let manifest = build_manifest();
        assert!(!manifest.name.is_empty());
        assert!(!manifest.version.is_empty());
        assert!(!manifest.description.is_empty());
        assert!(!manifest.supported_targets.is_empty());
    }
}
