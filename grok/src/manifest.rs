//! Manifest emitted by `grok --manifest` for the registry publish pipeline.

use serde::Serialize;

const DESCRIPTION: &str = "xAI Grok CLI as an iii worker — grok::* run headless Grok turns, mirror raw streaming-json events onto grok::events, and stream AgentEvent frames onto agent::events.";

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
        default_config: serde_json::json!({
            "defaults": {
                "model": "",
                "cwd": "",
                "always_approve": true,
            },
            "events_stream": "agent::events",
            "raw_events_stream": "grok::events",
            "grok_executable": "",
            "iii_context": true,
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
