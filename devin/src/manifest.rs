//! Manifest emitted by `devin --manifest` for the registry publish pipeline.

use serde::Serialize;

const DESCRIPTION: &str = "Devin CLI + API as an iii worker — devin::run drives the local devin CLI and streams AgentEvent frames onto agent::events; devin::session::* wrap the Devin cloud session lifecycle and devin::api reaches any v3 endpoint.";

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
        description: DESCRIPTION.to_string(),
        default_config: crate::config::Config::default().to_json(),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
