use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

const STATE_DIR: &str = "iii_workers";
const STATE_FILE: &str = "iii_workers/launcher-state.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManagedWorker {
    pub image: String,
    pub container_id: String,
    pub runtime: String,
    pub started_at: DateTime<Utc>,
    pub status: String,
    pub config: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LauncherState {
    pub managed_workers: HashMap<String, ManagedWorker>,
}

impl LauncherState {
    pub fn load() -> Result<Self> {
        let path = Path::new(STATE_FILE);
        if !path.exists() {
            return Ok(Self::default());
        }
        let data = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read state file: {}", STATE_FILE))?;
        let state: LauncherState = serde_json::from_str(&data)
            .with_context(|| "failed to parse launcher state JSON")?;
        Ok(state)
    }

    pub fn save(&self) -> Result<()> {
        let dir = Path::new(STATE_DIR);
        if !dir.exists() {
            std::fs::create_dir_all(dir)
                .with_context(|| format!("failed to create state directory: {}", STATE_DIR))?;
        }
        let data = serde_json::to_string_pretty(self)
            .with_context(|| "failed to serialize launcher state")?;
        std::fs::write(STATE_FILE, data)
            .with_context(|| format!("failed to write state file: {}", STATE_FILE))?;
        Ok(())
    }

    pub fn add_worker(&mut self, name: String, worker: ManagedWorker) {
        self.managed_workers.insert(name, worker);
    }

    pub fn remove_worker(&mut self, name: &str) -> Option<ManagedWorker> {
        self.managed_workers.remove(name)
    }

    pub fn get_worker(&self, name: &str) -> Option<&ManagedWorker> {
        self.managed_workers.get(name)
    }
}
