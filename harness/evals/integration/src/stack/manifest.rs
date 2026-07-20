use std::path::Path;

use serde_json::{json, Value};

use super::config::{ENV_ALLOWLIST, WORKER_START_ORDER};
use super::{RunLayout, StackBins};

pub(crate) fn write_stack_manifest(
    bins: &StackBins,
    layout: &RunLayout,
    port: u16,
) -> anyhow::Result<()> {
    let info = stack_info(bins, layout, port);
    std::fs::write(
        layout.stack_manifest_path(),
        crate::canonical::canonical_json_pretty(&info),
    )?;
    Ok(())
}

pub(crate) fn stack_info(bins: &StackBins, layout: &RunLayout, port: u16) -> Value {
    let mut binaries = serde_json::Map::new();
    let mut record = |name: &str, path: &Path| {
        let absolute = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let digest = std::fs::read(&absolute)
            .map(|bytes| crate::canonical::sha256_of_bytes(&bytes))
            .unwrap_or_else(|error| format!("unreadable: {error}"));
        binaries.insert(
            name.to_string(),
            json!({ "path": absolute.to_string_lossy(), "sha256": digest }),
        );
    };
    record("engine", &bins.engine);
    record("harness", &bins.harness);
    for (name, path) in &bins.workers {
        record(name, path);
    }
    json!({
        "port": port,
        "run_root": layout.root.to_string_lossy(),
        "binaries": Value::Object(binaries),
        "env_allowlist": ENV_ALLOWLIST,
        "start_order": WORKER_START_ORDER,
    })
}
