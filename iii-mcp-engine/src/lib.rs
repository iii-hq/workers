//! MCP-shaped engine-tools layer. Exposes the iii engine read allowlist as
//! MCP tools so external agents (Cursor, Claude Code) can consume the same
//! surface the iii-console chat panel uses.
//!
//! Each tool is registered twice:
//!   1. As an iii bus function `iii-mcp-engine::<engine_fn_id>` that proxies
//!      to the underlying engine read and applies the per-tool max-bytes
//!      contract.
//!   2. As a tool descriptor in `skills::register` so the iii-console
//!      `/skills` route + chat panel discover it.

use anyhow::Result;
use iii_sdk::{FunctionRef, IIIError, RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::json;

mod tools;
pub use tools::truncate_result;
use tools::{ToolEntry, ENGINE_TOOLS};

pub struct McpRefs(pub Vec<FunctionRef>);

impl McpRefs {
    pub fn unregister_all(self) {
        for r in self.0 {
            r.unregister();
        }
    }
}

pub async fn register_with_iii(iii: &III) -> Result<McpRefs> {
    let mut refs = Vec::new();

    for entry in ENGINE_TOOLS {
        let iii_for_handler = iii.clone();
        let entry_static: &'static ToolEntry = entry;

        let func = iii.register_function((
            RegisterFunctionMessage::with_id(entry_static.mcp_name.to_string()).with_description(
                format!(
                    "MCP proxy: {} (max {} bytes).",
                    entry_static.description,
                    entry_static.max_bytes.unwrap_or(0)
                ),
            ),
            move |payload: Value| {
                let iii = iii_for_handler.clone();
                async move {
                    let outcome = iii
                        .trigger(TriggerRequest {
                            function_id: entry_static.function_id.to_string(),
                            payload,
                            action: None,
                            timeout_ms: Some(30_000),
                        })
                        .await
                        .map_err(|e| IIIError::Handler(e.to_string()))?;
                    let max = entry_static.max_bytes.unwrap_or(usize::MAX);
                    Ok::<_, IIIError>(truncate_result(outcome, max))
                }
            },
        ));
        refs.push(func);
    }

    // Register the worker's skill so the chat panel / /skills route lists it.
    // Best-effort: a missing or older `skills` worker shouldn't keep us from booting.
    let descriptors: Vec<Value> = ENGINE_TOOLS
        .iter()
        .map(|t| {
            json!({
                "name": t.mcp_name,
                "description": t.description,
                "input_schema": { "type": "object", "additionalProperties": true },
                "max_bytes": t.max_bytes,
            })
        })
        .collect();

    let _ = iii
        .trigger(TriggerRequest {
            function_id: "skills::register".into(),
            payload: json!({
                "id": "iii-mcp-engine",
                "skill_version": env!("CARGO_PKG_VERSION"),
                "min_console_version": "0.1.0",
                "body": "MCP-shaped engine-tools layer. Read-only proxies to engine state.",
                "tools": descriptors,
            }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await;

    Ok(McpRefs(refs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_marks_oversized() {
        let big = json!({ "x": "y".repeat(1024) });
        let out = truncate_result(big, 100);
        assert_eq!(out["_truncated"], true);
        assert!(out["_original_bytes"].as_u64().unwrap() > 100);
    }

    #[test]
    fn small_passes_through() {
        let small = json!({ "ok": true });
        assert_eq!(truncate_result(small.clone(), 1024), small);
    }
}
