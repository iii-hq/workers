//! Manifest emitted by `codex --manifest` for the registry publish pipeline.

use serde::Serialize;

const DESCRIPTION: &str = "OpenAI Codex as an iii worker — codex::* run headless Codex turns, mirror raw thread events onto codex::events, and stream AgentEvent frames onto agent::events.";

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
                "sandbox_mode": "workspace-write",
                "approval_policy": "never",
                "reasoning_effort": "",
                "cwd": "",
                "skip_git_repo_check": true,
            },
            "events_stream": "agent::events",
            "raw_events_stream": "codex::events",
            "codex_executable": "",
            "base_url": "",
            "iii_context": true,
        }),
        supported_targets: vec![env!("TARGET").to_string()],
    }
}
