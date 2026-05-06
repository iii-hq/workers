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
            "Agentic content registry worker. Hosts skills + prompts + the iii:// resource resolver that the mcp worker serves to harnesses."
                .to_string(),
        default_config: serde_json::json!({
            "scopes": {
                "skills": "skills",
                "prompts": "prompts"
            },
            "state_timeout_ms": 10_000
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

        assert_eq!(parsed["name"], "skills");
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|s| !s.is_empty()));
        assert_eq!(parsed["default_config"]["scopes"]["skills"], "skills");
        assert_eq!(parsed["default_config"]["scopes"]["prompts"], "prompts");
        assert_eq!(parsed["default_config"]["state_timeout_ms"], 10_000);
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }
}
