// motia/workers/iii-mcp-engine/src/tools.rs
use serde_json::{json, Value};

pub struct ToolEntry {
    /// MCP-facing tool name (also the bus function id we register).
    /// Namespaced under registering worker per skills::register validator.
    pub mcp_name: &'static str,
    /// Bus function id we proxy to.
    pub function_id: &'static str,
    pub description: &'static str,
    pub max_bytes: Option<usize>,
}

/// Allowlist of engine reads exposed via MCP. Matches iii repo's
/// console-frontend/src/chat/agent/engine-tools.json (resolved entries only).
/// Update both files together when the engine grows new read functions.
pub const ENGINE_TOOLS: &[ToolEntry] = &[
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::traces::list",
        function_id: "engine::traces::list",
        description: "List recent traces.",
        max_bytes: Some(32768),
    },
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::traces::tree",
        function_id: "engine::traces::tree",
        description: "Get a trace tree by id.",
        max_bytes: Some(65536),
    },
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::logs::list",
        function_id: "engine::logs::list",
        description: "List logs (with filters).",
        max_bytes: Some(65536),
    },
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::workers::list",
        function_id: "engine::workers::list",
        description: "List active workers.",
        max_bytes: Some(16384),
    },
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::functions::list",
        function_id: "engine::functions::list",
        description: "List registered functions.",
        max_bytes: Some(16384),
    },
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::triggers::list",
        function_id: "engine::triggers::list",
        description: "List registered triggers.",
        max_bytes: Some(16384),
    },
    ToolEntry {
        mcp_name: "iii-mcp-engine::engine::queue::dlq_messages",
        function_id: "engine::queue::dlq_messages",
        description: "List dead-letter messages.",
        max_bytes: Some(32768),
    },
];

/// If `v` serialises to more than `max` bytes, return a truncation marker
/// instead. Used to enforce per-tool max-bytes contracts at the MCP layer
/// so a runaway result can't blow the LLM context.
pub fn truncate_result(v: Value, max: usize) -> Value {
    let s = v.to_string();
    if s.len() <= max {
        return v;
    }
    let preview_len = max.saturating_sub(64);
    json!({
        "_truncated": true,
        "_original_bytes": s.len(),
        "_max_bytes": max,
        "preview": s.get(..preview_len).unwrap_or("")
    })
}
