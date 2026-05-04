//! Per-scenario World.
//!
//! Holds:
//!
//! - `iii` — `None` when the engine is unreachable so `@engine` steps
//!   can soft-skip without failing the run.
//! - `cfg` — the SkillsConfig the in-process registration used.
//! - `unique_id` — UUID-prefixed id used to namespace state writes
//!   per scenario (the `skills` / `prompts` state scopes are
//!   process-wide on the engine side).
//! - `stash` — typed outcome slots a `Then` step can read after the
//!   matching `When` step wrote.

use std::collections::HashMap;
use std::sync::Arc;

use cucumber::World;
use iii_sdk::III;
use serde_json::Value;
use uuid::Uuid;

use iii_skills::config::SkillsConfig;

#[derive(World)]
#[world(init = Self::new)]
pub struct IiiSkillsWorld {
    pub iii: Option<Arc<III>>,
    pub cfg: Arc<SkillsConfig>,
    pub unique_id: String,
    pub stash: HashMap<String, Value>,
}

impl std::fmt::Debug for IiiSkillsWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IiiSkillsWorld")
            .field("engine_connected", &self.iii.is_some())
            .field("unique_id", &self.unique_id)
            .finish()
    }
}

impl IiiSkillsWorld {
    pub fn new() -> Self {
        let unique_id = format!("bdd-{}", &Uuid::new_v4().simple().to_string()[..12]);
        Self {
            iii: None,
            cfg: Arc::new(SkillsConfig::default()),
            unique_id,
            stash: HashMap::new(),
        }
    }

    /// Scenario-scoped id prefix for state-scope isolation. Call this
    /// whenever a scenario needs an id / name that won't collide with a
    /// concurrent scenario or a leftover from a previous binary run.
    pub fn scoped_id(&self, base: &str) -> String {
        let id = format!("{}-{}", base, self.unique_id);
        // skills::validate_id caps at 64 chars; keep a comfortable margin.
        id.chars().take(64).collect()
    }
}
