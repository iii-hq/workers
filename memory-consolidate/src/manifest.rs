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
            "Scheduled hygiene for the memory worker: deterministic dedup of near-duplicate memories, supersede-only through the public memory functions, pinned untouchable, catch-up-on-boot scheduling."
                .to_string(),
        // Mirrors config::WorkerConfig::default() field-for-field.
        default_config: serde_json::json!({
            "enabled": true,
            "interval_hours": 24,
            "dry_run": false,
            "banks": [],
            "max_supersedes_per_run": 200,
            "llm_assist_enabled": false,
            "llm_model": "",
            "promote_corroboration_threshold": 4,
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
