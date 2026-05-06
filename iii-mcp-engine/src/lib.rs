//! MCP-shaped engine-tools layer. Exposes the iii engine read allowlist as
//! MCP tools so external agents (Cursor, Claude Code) can consume the same
//! surface the iii-console chat panel uses. Function ids are wired in
//! Plan Task 7.2.

use anyhow::Result;
use iii_sdk::{FunctionRef, III};

pub struct McpRefs(pub Vec<FunctionRef>);

impl McpRefs {
    pub fn unregister_all(self) {
        for r in self.0 {
            r.unregister();
        }
    }
}

pub async fn register_with_iii(_iii: &III) -> Result<McpRefs> {
    // Tools registered in Plan Task 7.2.
    Ok(McpRefs(Vec::new()))
}
