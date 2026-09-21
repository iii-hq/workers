//! Registry manifest available without configuration, credentials or an engine.
use serde::Serialize;
pub const DESCRIPTION: &str = "Generic typed JEV evaluations over arbitrary JSON state with atomic results and known usage statistics.";
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
        name: "jev".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: DESCRIPTION.into(),
        default_config: crate::config::JevConfig::default().to_json(),
        supported_targets: vec![env!("TARGET").into()],
    }
}
