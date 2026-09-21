//! Registry manifest available without an engine.
use serde::Serialize;
pub const DESCRIPTION: &str = "Provider-neutral Noul, Choice and Score evaluations: judge::evaluate, judge::models::list and judge::cancel forwarded to the selected judge-<provider> worker.";
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
        name: "judge".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        description: DESCRIPTION.into(),
        // Provider selection is JUDGE_PROVIDER in the process environment; the
        // hub keeps no configuration entry of its own.
        default_config: serde_json::json!({}),
        supported_targets: vec![env!("TARGET").into()],
    }
}
