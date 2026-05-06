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
        description: "Durable run::start state machine driving each agent turn through provisioning, assistant, tools, steering, and tearing-down.".to_string(),
        default_config: serde_json::json!({
            "sync_default_timeout_ms": 120_000,
            "sync_poll_interval_ms": 50,
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
        assert!(parsed["default_config"].is_object());
        assert_eq!(parsed["default_config"]["sync_default_timeout_ms"], 120_000);
        assert_eq!(parsed["default_config"]["sync_poll_interval_ms"], 50);
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }
}
