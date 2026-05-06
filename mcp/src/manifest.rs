use serde::Serialize;

use crate::config::default_hidden_prefixes;

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
        description: "MCP 2025-06-18 Streamable HTTP bridge for the iii engine.".to_string(),
        default_config: serde_json::json!({
            "hidden_prefixes": default_hidden_prefixes(),
            "require_expose": false,
            "api_path": "/mcp",
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
        assert!(parsed["description"].is_string());
        assert!(!parsed["description"].as_str().unwrap().is_empty());
        assert!(parsed["default_config"].is_object());
        assert!(parsed["default_config"]["hidden_prefixes"].is_array());
        assert!(!parsed["supported_targets"]
            .as_array()
            .expect("supported_targets must be an array")
            .is_empty());
    }

    #[test]
    fn default_config_matches_mcp_config_default() {
        let m = build_manifest();
        let cfg = crate::config::McpConfig::default();
        assert_eq!(
            m.default_config["require_expose"].as_bool().unwrap(),
            cfg.require_expose
        );
        assert_eq!(m.default_config["api_path"].as_str().unwrap(), cfg.api_path);
        let arr: Vec<String> = m.default_config["hidden_prefixes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_string())
            .collect();
        assert_eq!(arr, cfg.hidden_prefixes);
    }
}
