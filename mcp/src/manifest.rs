//! `--manifest` subcommand output. Same contract as every binary worker.

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
    // Serialize `McpConfig::default()` rather than hand-rolling the JSON
    // here so adding a field to the config can never silently drift the
    // manifest's advertised defaults.
    let default_config =
        serde_json::to_value(McpConfig::default()).expect("McpConfig is always serializable");

    ModuleManifest {
        name: env!("CARGO_PKG_NAME").to_string(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        description:
            "Model Context Protocol bridge. Exposes iii functions as MCP tools and the skills worker as MCP resources/prompts over POST /mcp."
                .to_string(),
        default_config,
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

        assert_eq!(parsed["name"], "mcp");
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert_eq!(parsed["default_config"]["api_path"], "mcp");
        assert_eq!(parsed["default_config"]["state_timeout_ms"], 30_000);
        assert!(parsed["default_config"]["hidden_prefixes"]
            .as_array()
            .is_some_and(|a| !a.is_empty()));
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_includes_every_mcpconfig_field() {
        // Regression: prior hand-rolled JSON drifted off McpConfig and
        // omitted `require_expose`. Using McpConfig::default() ensures
        // every field is present going forward.
        let m = build_manifest();
        let cfg = &m.default_config;
        assert!(cfg.get("api_path").is_some());
        assert!(cfg.get("state_timeout_ms").is_some());
        assert!(cfg.get("hidden_prefixes").is_some());
        assert_eq!(
            cfg.get("require_expose"),
            Some(&serde_json::Value::Bool(false))
        );
    }
}
