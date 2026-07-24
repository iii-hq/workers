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
            "Durable same-model A/B evaluation for prompts and system prompts over harness turns."
                .to_string(),
        default_config: serde_json::json!({}),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
