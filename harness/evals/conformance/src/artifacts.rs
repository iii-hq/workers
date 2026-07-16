//! Artifact writing: every JSON file goes through the canonical writer so
//! report bytes are deterministic (spec § Verification: "deterministic
//! report bytes").

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde_json::Value;

use crate::canonical::canonical_json_pretty;

/// Write one canonical-JSON artifact and return its run-relative path.
pub fn write_json(run_root: &Path, path: &Path, value: &Value) -> anyhow::Result<String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, canonical_json_pretty(value))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path
        .strip_prefix(run_root)
        .unwrap_or(path)
        .to_string_lossy()
        .into_owned())
}

/// On a passing run without `--retain-success`, drop the heavyweight stack
/// state but keep the compact report (`result.json`, `stack.json`,
/// `scenarios/<id>/invariants.json`).
pub fn trim_passing_run(run_root: &Path) {
    for dir in [
        "engine",
        "logs",
        "seeds",
        "session-data",
        "leases",
        "skills",
    ] {
        let _ = std::fs::remove_dir_all(run_root.join(dir));
    }
    let _ = std::fs::remove_file(run_root.join("queue"));
    let _ = std::fs::remove_file(run_root.join("recorder.log.jsonl"));
}

pub fn scenario_file(scenario_dir: &Path, name: &str) -> PathBuf {
    scenario_dir.join(name)
}
