//! `--manifest` subcommand output. Same contract as every binary worker.

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
        description:
            "Model Context Protocol bridge. Exposes iii functions as MCP tools and the skills worker as MCP resources/prompts over POST /mcp."
                .to_string(),
        default_config: serde_json::json!({
            "api_path": "mcp",
            "state_timeout_ms": 30_000,
            "hidden_prefixes": [
                "engine::",
                "state::",
                "stream::",
                "iii.",
                "iii::",
                "mcp::",
                "a2a::",
                "skills::",
                "prompts::"
            ]
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
}
