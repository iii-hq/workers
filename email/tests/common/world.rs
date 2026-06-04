//! Per-scenario `World` shared across steps. Holds the engine handle
//! (`None` when the engine isn't reachable so `@engine` scenarios can
//! soft-skip without failing) plus a typed stash that `When` steps
//! write and `Then` steps read.

use std::collections::HashMap;
use std::sync::Arc;

use cucumber::World;
use iii_sdk::III;
use serde_json::Value;
use uuid::Uuid;

#[derive(World)]
#[world(init = Self::new)]
pub struct EmailWorld {
    pub iii: Option<Arc<III>>,
    pub unique_id: String,
    pub stash: HashMap<String, Value>,
}

impl std::fmt::Debug for EmailWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmailWorld")
            .field("engine_connected", &self.iii.is_some())
            .field("unique_id", &self.unique_id)
            .finish()
    }
}

impl EmailWorld {
    pub fn new() -> Self {
        let unique_id = format!("bdd-{}", &Uuid::new_v4().simple().to_string()[..12]);
        Self {
            iii: None,
            unique_id,
            stash: HashMap::new(),
        }
    }
}
