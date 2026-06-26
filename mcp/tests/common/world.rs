//! Per-scenario `World` cucumber threads through every step.
//!
//! Holds:
//!
//! - `iii` — `None` when the engine is unreachable so `@engine` steps
//!   can soft-skip without failing the run.
//! - `http` — reusable reqwest client posting to `http_url`.
//! - `http_url` — engine HTTP path bound by mcp's HTTP trigger
//!   (`http://127.0.0.1:3000/mcp` by default; override via
//!   `III_ENGINE_HTTP_URL`).
//! - `unique_id` — UUID-prefixed id used by step defs that need to
//!   namespace per-scenario state.
//! - `stash` — typed outcome slots a `Then` step can read after the
//!   matching `When` step wrote (HTTP status, parsed JSON-RPC, etc.).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use cucumber::World;
use iii_sdk::IIIClient;
use serde_json::Value;
use uuid::Uuid;

use crate::common::engine::http_url;

#[derive(World)]
#[world(init = Self::new)]
pub struct McpWorld {
    pub iii: Option<Arc<IIIClient>>,
    pub http: reqwest::Client,
    pub http_url: String,
    pub unique_id: String,
    pub stash: HashMap<String, Value>,
}

impl std::fmt::Debug for McpWorld {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("McpWorld")
            .field("engine_connected", &self.iii.is_some())
            .field("http_url", &self.http_url)
            .field("unique_id", &self.unique_id)
            .finish()
    }
}

impl McpWorld {
    pub fn new() -> Self {
        let unique_id = format!("bdd-{}", &Uuid::new_v4().simple().to_string()[..12]);
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .expect("build reqwest client");
        Self {
            iii: None,
            http,
            http_url: http_url(),
            unique_id,
            stash: HashMap::new(),
        }
    }
}
