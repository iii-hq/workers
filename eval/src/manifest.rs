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
        description: "Harness E2E improvement loop: one cycle started from an objective or a finished E2E execution, with E2E coverage selection, a frozen plan, a per-cycle budget, durable report evidence, a deterministic first pass and a decision computed in code.".to_string(),
        default_config: serde_json::json!({}),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
