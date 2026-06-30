use serde::{Deserialize, Serialize};
use serde_json::Value;

/// HTTP server configuration — minimal for Phase 0.
/// Expanded in Phase 1 with full REST API config schema.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct RestApiConfig {
    pub port: u16,
}

impl Default for RestApiConfig {
    fn default() -> Self {
        Self { port: 3111 }
    }
}

impl RestApiConfig {
    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("RestApiConfig serializes")
    }
}
