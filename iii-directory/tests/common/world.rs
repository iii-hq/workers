//! Per-scenario World.
//!
//! Holds:
//!
//! - `iii` — `None` when the engine is unreachable so `@engine` steps
//!   can soft-skip without failing the run.
//! - `cfg` — the `SkillsConfig` the in-process registration used.
//! - `skills_folder` — tempdir scenarios write fixture markdown into.
//! - `stash` — typed outcome slots a `Then` step can read after the
//!   matching `When` step wrote.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use cucumber::World;
use iii_sdk::III;
use serde_json::Value;

use iii_directory::config::SkillsConfig;

#[derive(World)]
#[world(init = Self::new)]
pub struct IiiSkillsWorld {
    pub iii: Option<Arc<III>>,
    pub cfg: Arc<SkillsConfig>,
    pub stash: HashMap<String, Value>,
    /// Path of the per-binary `skills_folder` tempdir. Mirrors what the
    /// shared in-process registration in `common::workers` configured
    /// the worker with. `None` when no engine is reachable.
    pub skills_folder: Option<PathBuf>,
    /// Optional registry origin (URL prefix) of a wiremock server set
    /// up by a download scenario. The download step uses this to point
    /// the worker at the mock instead of the real registry.
    pub registry_url: Option<String>,
}

impl std::fmt::Debug for IiiSkillsWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IiiSkillsWorld")
            .field("engine_connected", &self.iii.is_some())
            .field("skills_folder", &self.skills_folder)
            .finish()
    }
}

impl IiiSkillsWorld {
    pub fn new() -> Self {
        Self {
            iii: None,
            cfg: Arc::new(SkillsConfig::default()),
            stash: HashMap::new(),
            skills_folder: None,
            registry_url: None,
        }
    }
}
