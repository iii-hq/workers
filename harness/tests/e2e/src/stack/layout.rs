use std::path::{Path, PathBuf};

use anyhow::Context;

/// All filesystem locations owned by one isolated integration run.
#[derive(Debug, Clone)]
pub struct RunLayout {
    pub root: PathBuf,
    pub engine_dir: PathBuf,
    pub seeds_dir: PathBuf,
    pub logs_dir: PathBuf,
}

impl RunLayout {
    pub fn allocate(artifacts_dir: &Path, run_id: &str) -> anyhow::Result<Self> {
        validate_component("run id", run_id)?;
        let root = artifacts_dir.join(run_id);
        let layout = Self {
            engine_dir: root.join("engine"),
            seeds_dir: root.join("seeds"),
            logs_dir: root.join("logs"),
            root,
        };
        for dir in [
            &layout.root,
            &layout.engine_dir,
            &layout.seeds_dir,
            &layout.logs_dir,
        ] {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        Ok(layout)
    }

    pub fn scenario_dir(&self, scenario_id: &str) -> anyhow::Result<PathBuf> {
        crate::types::scenario::validate_scenario_id(scenario_id)?;
        let dir = self.root.join("scenarios").join(scenario_id);
        std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
        Ok(dir)
    }

    pub fn engine_config_path(&self) -> PathBuf {
        self.engine_dir.join("engine.yaml")
    }

    pub fn stack_manifest_path(&self) -> PathBuf {
        self.root.join("stack.json")
    }

    pub fn seed_path(&self, worker: &str) -> anyhow::Result<PathBuf> {
        validate_component("worker name", worker)?;
        Ok(self.seeds_dir.join(format!("{worker}.yaml")))
    }

    pub fn configuration_dir(&self) -> PathBuf {
        self.engine_dir.join("configuration")
    }

    pub fn session_data_dir(&self) -> PathBuf {
        self.root.join("session-data")
    }

    pub fn leases_dir(&self) -> PathBuf {
        self.root.join("leases")
    }

    pub fn queue_path(&self) -> PathBuf {
        self.root.join("queue")
    }

    pub fn skills_dir(&self) -> PathBuf {
        self.root.join("skills")
    }

    pub(crate) fn log_path(&self, log_name: &str, extension: &str) -> anyhow::Result<PathBuf> {
        validate_component("log name", log_name)?;
        validate_component("log extension", extension)?;
        Ok(self.logs_dir.join(format!("{log_name}.{extension}")))
    }
}

fn validate_component(kind: &str, value: &str) -> anyhow::Result<()> {
    let valid = !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    anyhow::ensure!(
        valid && value != "." && value != "..",
        "{kind} must be one safe path component"
    );
    Ok(())
}
