//! `--manifest` subcommand output for the registry publish pipeline.

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
            "Model-ready context assembly: token counting, function-result pruning, and history compaction over caller-supplied messages."
                .to_string(),
        // Mirrors config::WorkerConfig::default() field-for-field.
        default_config: serde_json::json!({
            "reserved_tokens_cap": 20_000,
            "reserved_pct": 10,
            "tail_turns": 2,
            "protect_recent_tokens": 40_000,
            "min_free_tokens": 20_000,
            "max_output_chars": 2_000,
            "lease_ttl_secs": 300,
            "allow_fallback_limits": true,
            "summarizer_timeout_ms": 320_000,
            "lease_dir": "~/.iii/data/context-manager",
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WorkerConfig;

    #[test]
    fn json_roundtrip_has_required_fields() {
        let m = build_manifest();
        let json = serde_json::to_string_pretty(&m).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["name"], env!("CARGO_PKG_NAME"));
        assert_eq!(parsed["version"], env!("CARGO_PKG_VERSION"));
        assert!(parsed["description"]
            .as_str()
            .is_some_and(|d| !d.is_empty()));
        assert!(parsed["default_config"].is_object());
        assert!(!parsed["supported_targets"].as_array().unwrap().is_empty());
    }

    #[test]
    fn default_config_mirrors_worker_config_default() {
        let m = build_manifest();
        let cfg = WorkerConfig::default();
        let from_manifest: WorkerConfig =
            serde_json::from_value(m.default_config.clone()).expect("default_config parses");
        assert_eq!(from_manifest, cfg);
    }
}
