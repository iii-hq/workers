//! Artifact writing: every JSON file goes through the canonical writer so
//! report bytes are deterministic (spec § Verification: "deterministic
//! report bytes").

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::Serialize;

use crate::canonical::canonical_json_pretty;

/// Canonical artifact persistence plus a single run-relative registry.
#[derive(Debug)]
pub struct ArtifactSink {
    run_root: PathBuf,
    paths: Vec<String>,
}

impl ArtifactSink {
    pub fn new(run_root: impl Into<PathBuf>) -> Self {
        Self {
            run_root: run_root.into(),
            paths: Vec::new(),
        }
    }

    pub fn run_root(&self) -> &Path {
        &self.run_root
    }

    /// Persist canonical JSON at a run-relative path and register it once.
    pub fn write_json<T>(
        &mut self,
        relative_path: impl AsRef<Path>,
        value: &T,
    ) -> anyhow::Result<String>
    where
        T: Serialize + ?Sized,
    {
        let relative_path = validated_relative_path(relative_path.as_ref())?;
        let path = self.run_root.join(relative_path);
        let recorded = write_json(&self.run_root, &path, value)?;
        if !self.paths.contains(&recorded) {
            self.paths.push(recorded.clone());
        }
        Ok(recorded)
    }

    pub fn write_scenario_json<T>(
        &mut self,
        scenario_id: &str,
        name: &str,
        value: &T,
    ) -> anyhow::Result<String>
    where
        T: Serialize + ?Sized,
    {
        self.write_json(Path::new("scenarios").join(scenario_id).join(name), value)
    }

    /// Persist UTF-8 text and register it through the same path policy as
    /// JSON evidence.
    pub fn write_scenario_text(
        &mut self,
        scenario_id: &str,
        name: &str,
        text: &str,
    ) -> anyhow::Result<String> {
        let candidate = Path::new("scenarios").join(scenario_id).join(name);
        let relative_path = validated_relative_path(&candidate)?.to_path_buf();
        let path = self.run_root.join(&relative_path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
        let recorded = relative_path.to_string_lossy().into_owned();
        if !self.paths.contains(&recorded) {
            self.paths.push(recorded.clone());
        }
        Ok(recorded)
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn trim_passing_run(&self) {
        trim_passing_run(&self.run_root);
    }
}

fn validated_relative_path(path: &Path) -> anyhow::Result<&Path> {
    use std::path::Component;

    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        anyhow::bail!(
            "artifact path must be a non-empty run-relative path: {}",
            path.display()
        );
    }
    Ok(path)
}

/// Write one canonical-JSON artifact and return its run-relative path.
pub fn write_json<T>(run_root: &Path, path: &Path, value: &T) -> anyhow::Result<String>
where
    T: Serialize + ?Sized,
{
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let value =
        serde_json::to_value(value).with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(path, canonical_json_pretty(&value))
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn sink_writes_canonical_json_and_registers_relative_path_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = ArtifactSink::new(dir.path());
        let path = sink
            .write_scenario_json("001", "evidence.json", &json!({"z": 1, "a": 2}))
            .unwrap();
        sink.write_scenario_json("001", "evidence.json", &json!({"a": 2, "z": 1}))
            .unwrap();

        assert_eq!(path, "scenarios/001/evidence.json");
        assert_eq!(sink.paths(), ["scenarios/001/evidence.json"]);
        assert_eq!(
            std::fs::read_to_string(dir.path().join(&path)).unwrap(),
            "{\n  \"a\": 2,\n  \"z\": 1\n}\n"
        );
    }

    #[test]
    fn sink_rejects_paths_outside_the_run_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut sink = ArtifactSink::new(dir.path());
        assert!(sink.write_json("../escape.json", &json!({})).is_err());
        assert!(sink.write_json("/tmp/escape.json", &json!({})).is_err());
    }
}
