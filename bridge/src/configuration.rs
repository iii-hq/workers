//! Configuration structures and parsing for the bridge worker.

use std::sync::Arc;

use tokio::sync::RwLock;

use crate::config::BridgeConfig;

/// The live config cell, swappable on a config update.
pub type ConfigCell = Arc<RwLock<Arc<BridgeConfig>>>;

/// Apply lock to serialize overlapping configuration change runs.
pub type ApplyLock = Arc<tokio::sync::Mutex<()>>;
