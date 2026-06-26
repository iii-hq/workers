//! Per-scenario World.
//!
//! Holds:
//!
//! - `iii` — `None` when the engine is unreachable so `@engine` steps
//!   can soft-skip without failing the run.
//! - `cfg` — the `CoderConfig` the in-process registration used.
//! - `base_path` — canonical path to the shared tempdir scenarios write
//!   fixtures into. Mirrors `cfg.base_path`.
//! - `stash` — JSON outcome slots a `Then` step reads after its
//!   matching `When` step wrote (see `common::helpers`).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use cucumber::World;
use iii_sdk::IIIClient;
use serde_json::Value;

use coder::config::CoderConfig;

#[derive(World)]
#[world(init = Self::new)]
pub struct CoderWorld {
    pub iii: Option<Arc<IIIClient>>,
    pub cfg: Arc<CoderConfig>,
    pub stash: HashMap<String, Value>,
    pub base_path: Option<PathBuf>,
}

impl std::fmt::Debug for CoderWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoderWorld")
            .field("engine_connected", &self.iii.is_some())
            .field("base_path", &self.base_path)
            .finish()
    }
}

impl CoderWorld {
    pub fn new() -> Self {
        Self {
            iii: None,
            cfg: Arc::new(CoderConfig::default()),
            stash: HashMap::new(),
            base_path: None,
        }
    }
}
