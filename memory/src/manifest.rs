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
            "Durable cross-session agent memory: named banks of always-injected markdown rules and auto-extracted memories, hybrid BM25 + entity recall, pinning, and supersede-never-delete history."
                .to_string(),
        // Mirrors config::WorkerConfig::default() field-for-field.
        default_config: serde_json::json!({
            "data_dir": "~/.iii/data/memory",
            "default_bank": "main",
            "inject_rules": true,
            "inject_memories": true,
            "recall_limit": 6,
            "recall_budget_tokens": 1_200,
            "extraction_enabled": true,
            "extraction_model": "",
            "extraction_window": 12,
            "extraction_timeout_ms": 60_000,
            "max_memories_per_turn": 8,
            "rule_learning_enabled": true,
            "max_rule_chars": 6_000,
            "decay_half_life_days": 30,
            "embeddings_enabled": true,
            "embedding_model": "",
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
