use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::config::WORKER_START_ORDER;

#[derive(Debug, Clone)]
pub struct StackBins {
    pub engine: PathBuf,
    pub harness: PathBuf,
    /// queue, iii-directory, session-manager, context-manager.
    pub workers: BTreeMap<String, PathBuf>,
}

impl StackBins {
    /// Every binary the scenario spawns, in declared start order.
    pub fn resolve(&self, name: &str) -> Option<&Path> {
        match name {
            "engine" => Some(&self.engine),
            "harness" => Some(&self.harness),
            other => self.workers.get(other).map(PathBuf::as_path),
        }
    }

    pub fn missing_workers(&self) -> Vec<&'static str> {
        WORKER_START_ORDER
            .iter()
            .filter(|name| !self.workers.contains_key(**name))
            .copied()
            .collect()
    }
}
