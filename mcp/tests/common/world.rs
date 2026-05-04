//! Per-scenario World.
//!
//! Holds:
//!
//! - `iii` — `None` when the engine is unreachable so `@engine` steps
//!   can soft-skip without failing the run.
//! - `cfg` — the McpConfig the in-process registration used.
//! - `unique_id` — UUID-prefixed id used to namespace JSON-RPC ids per
//!   scenario.
//! - `stash` — typed outcome slots a `Then` step can read after the
//!   matching `When` step wrote.

use std::collections::HashMap;
use std::sync::Arc;

use cucumber::World;
use iii_sdk::III;
use serde_json::Value;
use uuid::Uuid;

use iii_mcp::config::McpConfig;

#[derive(World)]
#[world(init = Self::new)]
pub struct IiiMcpWorld {
    pub iii: Option<Arc<III>>,
    #[allow(dead_code)]
    pub cfg: Arc<McpConfig>,
    pub unique_id: String,
    pub stash: HashMap<String, Value>,
}

impl std::fmt::Debug for IiiMcpWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IiiMcpWorld")
            .field("engine_connected", &self.iii.is_some())
            .field("unique_id", &self.unique_id)
            .finish()
    }
}

impl IiiMcpWorld {
    pub fn new() -> Self {
        let unique_id = format!("bdd-{}", &Uuid::new_v4().simple().to_string()[..12]);
        Self {
            iii: None,
            cfg: Arc::new(McpConfig::default()),
            unique_id,
            stash: HashMap::new(),
        }
    }
}
