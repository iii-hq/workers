use serde::Serialize;

use crate::config::McpConfig;

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
        description: "MCP 2025-06-18 Streamable HTTP server for the iii engine: iii_call / iii_observe gateway, behind rbac-proxy.".to_string(),
        default_config: {
            let cfg = McpConfig::default();
            serde_json::json!({
                "api_path": cfg.api_path,
                "allowed_origins": cfg.allowed_origins,
                "session_idle_ttl_secs": cfg.session_idle_ttl_secs,
                "max_buffered_events": cfg.max_buffered_events,
            })
        },
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m: serde_json::Value = serde_json::to_value(build_manifest()).unwrap();
        assert_eq!(m["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(m["version"], env!("CARGO_PKG_VERSION"));
        assert!(!m["description"].as_str().unwrap().is_empty());
        assert!(!m["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_is_exactly_the_live_settings() {
        let cfg = crate::config::McpConfig::default();
        assert_eq!(
            build_manifest().default_config,
            serde_json::json!({
                "api_path": cfg.api_path,
                "allowed_origins": [],
                "session_idle_ttl_secs": cfg.session_idle_ttl_secs,
                "max_buffered_events": cfg.max_buffered_events,
            })
        );
    }
}
