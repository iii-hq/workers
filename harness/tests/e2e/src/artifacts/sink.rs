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
        Ok(self.register(recorded))
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
        Ok(self.register(relative_path.to_string_lossy().into_owned()))
    }

    pub fn paths(&self) -> &[String] {
        &self.paths
    }

    pub fn trim_passing_run(&self) {
        trim_passing_run(&self.run_root);
    }

    fn register(&mut self, path: String) -> String {
        if !self.paths.contains(&path) {
            self.paths.push(path.clone());
        }
        path
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
    let relative_path = path.strip_prefix(run_root).with_context(|| {
        format!(
            "artifact {} is outside run root {}",
            path.display(),
            run_root.display()
        )
    })?;
    let relative_path = validated_relative_path(relative_path)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let value =
        serde_json::to_value(value).with_context(|| format!("serializing {}", path.display()))?;
    std::fs::write(path, canonical_json_pretty(&value))
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(relative_path.to_string_lossy().into_owned())
}

/// On a passing run without `--retain-success`, drop heavyweight stack state
/// but keep the compact reports and collected evidence.
pub(crate) fn trim_passing_run(run_root: &Path) {
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
    let queue = run_root.join("queue");
    let _ = std::fs::remove_dir_all(&queue);
    let _ = std::fs::remove_file(queue);
    let _ = std::fs::remove_file(run_root.join("recorder.log.jsonl"));
}
