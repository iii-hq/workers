//! Production adapters behind the ports: `llm-router` calls over the
//! iii bus and filesystem-backed lease storage.

pub mod fs_lease;
pub mod router;
