use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize)]
pub struct WorkerManifest {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub trigger_types: Vec<TriggerType>,
}

#[derive(Debug, Deserialize)]
pub struct TriggerType {
    pub name: String,
    pub fires_when: String,
    pub payload: String,
}

#[derive(Debug)]
pub struct RegisteredFunction {
    pub id: String,
    pub description: Option<String>,
}

/// Parse `<dir>/iii.worker.yaml`.
pub fn read_manifest(_dir: &Path) -> anyhow::Result<WorkerManifest> {
    anyhow::bail!("introspect::read_manifest not yet implemented")
}

/// Recursively scan `<src_dir>` for `RegisterFunction::new("...", ...).description("...")` calls
/// and return the function ids + descriptions in source order.
pub fn list_functions(_src_dir: &Path) -> anyhow::Result<Vec<RegisteredFunction>> {
    anyhow::bail!("introspect::list_functions not yet implemented")
}
