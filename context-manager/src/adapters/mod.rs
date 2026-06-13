//! Production adapters behind the ports: `llm-router` calls over the
//! iii bus and `state::*`-backed lease storage.

pub mod router;
pub mod state_lease;
