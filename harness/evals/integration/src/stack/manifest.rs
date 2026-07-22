use std::path::Path;

use anyhow::Context;
use serde_json::{json, Value};

use super::config::{ENV_ALLOWLIST, WORKER_START_ORDER};
use super::{RunLayout, StackBins};

const STACK_PROFILE: &str = "harness-core-v1";

pub(crate) fn write_stack_manifest(
    bins: &StackBins,
    layout: &RunLayout,
    port: u16,
) -> anyhow::Result<()> {
    let info = stack_info(bins, layout, port)?;
    std::fs::write(
        layout.stack_manifest_path(),
        crate::canonical::canonical_json_pretty(&info),
    )?;
    Ok(())
}

pub(crate) fn stack_info(bins: &StackBins, layout: &RunLayout, port: u16) -> anyhow::Result<Value> {
    let mut binaries = serde_json::Map::new();
    let mut record = |name: &str, path: &Path| -> anyhow::Result<()> {
        let absolute = path
            .canonicalize()
            .with_context(|| format!("resolve {name} binary {}", path.display()))?;
        let digest = std::fs::read(&absolute)
            .map(|bytes| crate::canonical::sha256_of_bytes(&bytes))
            .with_context(|| format!("read {name} binary {}", absolute.display()))?;
        binaries.insert(
            name.to_string(),
            json!({
                "path": absolute.to_string_lossy(),
                "sha256": digest,
                "version": Value::Null
            }),
        );
        Ok(())
    };
    record("engine", &bins.engine)?;
    record("harness", &bins.harness)?;
    for (name, path) in &bins.workers {
        record(name, path)?;
    }
    let external_workers = WORKER_START_ORDER
        .iter()
        .copied()
        .chain(std::iter::once("harness"))
        .collect::<Vec<_>>();
    Ok(json!({
        "profile": STACK_PROFILE,
        "components": {
            "engine_builtins": ["configuration", "state", "stream", "cron"],
            "external_workers": external_workers,
            "controlled_services": ["scripted-router", "integration-recorder"]
        },
        "port": port,
        "run_root": layout.root.to_string_lossy(),
        "binaries": Value::Object(binaries),
        "provenance": {
            "engine_lock": include_str!(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/engine.lock"
            )).trim()
        },
        "env_allowlist": ENV_ALLOWLIST,
        "start_order": WORKER_START_ORDER,
    }))
}
