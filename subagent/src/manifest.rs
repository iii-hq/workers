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
    let cfg = crate::config::SubagentConfig::default();
    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description: "Spawn child agent sessions under subagent::start via run::start_and_wait."
            .to_string(),
        default_config: serde_json::json!({
            "default_system_prompt": cfg.default_system_prompt,
            "trigger_timeout_ms": cfg.trigger_timeout_ms,
            "default_max_subagent_depth": cfg.default_max_subagent_depth,
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
        assert!(parsed["default_config"].is_object());
        let targets = parsed["supported_targets"].as_array().expect("array");
        assert!(!targets.is_empty());
    }
}
