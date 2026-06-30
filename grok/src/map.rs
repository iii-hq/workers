//! Translate Grok thread items to the AgentEvent wire subset emitted on
//! `agent::events`. Command/patch/mcp/web items become function_execution
//! frames; agent_message/reasoning become message_complete.

use serde_json::{json, Value};

use crate::grok::events_types::{CommandExecutionItem, McpToolCallItem, ThreadItemDetails};
use crate::wire::Usage as WireUsage;

/// Bus-style function id for an execution-type item.
pub fn function_id(details: &ThreadItemDetails) -> String {
    match details {
        ThreadItemDetails::CommandExecution(_) => "grok::shell".to_string(),
        ThreadItemDetails::FileChange(_) => "grok::apply_patch".to_string(),
        ThreadItemDetails::WebSearch(_) => "grok::web_search".to_string(),
        ThreadItemDetails::McpToolCall(m) => format!("{}::{}", server_or(m, "mcp"), tool_or(m)),
        _ => "grok::item".to_string(),
    }
}

fn server_or(m: &McpToolCallItem, fallback: &str) -> String {
    if m.server.is_empty() {
        fallback.to_string()
    } else {
        m.server.clone()
    }
}

fn tool_or(m: &McpToolCallItem) -> String {
    if m.tool.is_empty() {
        "tool".to_string()
    } else {
        m.tool.clone()
    }
}

/// True for the item types that map to function_execution_start/end.
pub fn is_exec_item(details: &ThreadItemDetails) -> bool {
    matches!(
        details,
        ThreadItemDetails::CommandExecution(_)
            | ThreadItemDetails::FileChange(_)
            | ThreadItemDetails::McpToolCall(_)
            | ThreadItemDetails::WebSearch(_)
    )
}

pub fn args_for(details: &ThreadItemDetails) -> Value {
    match details {
        ThreadItemDetails::CommandExecution(c) => json!({ "command": c.command }),
        ThreadItemDetails::FileChange(f) => json!({ "changes": f.changes }),
        ThreadItemDetails::WebSearch(w) => json!({ "query": w.query }),
        ThreadItemDetails::McpToolCall(m) => m.arguments.clone(),
        _ => json!({}),
    }
}

pub fn result_content(details: &ThreadItemDetails) -> Value {
    let text = match details {
        ThreadItemDetails::CommandExecution(c) => c.aggregated_output.clone(),
        ThreadItemDetails::FileChange(f) => f.changes.to_string(),
        ThreadItemDetails::WebSearch(w) => w.query.clone(),
        ThreadItemDetails::McpToolCall(m) => match &m.error {
            Some(e) => e.message.clone(),
            None => m.result.to_string(),
        },
        _ => String::new(),
    };
    json!([{ "type": "text", "text": text }])
}

pub fn is_error_item(details: &ThreadItemDetails) -> bool {
    match details {
        ThreadItemDetails::CommandExecution(CommandExecutionItem {
            status, exit_code, ..
        }) => status.is_failure() || matches!(exit_code, Some(code) if *code != 0),
        ThreadItemDetails::FileChange(f) => f.status.is_failure(),
        ThreadItemDetails::McpToolCall(m) => m.status.is_failure() || m.error.is_some(),
        _ => false,
    }
}

pub fn map_usage(u: &crate::grok::events_types::Usage) -> WireUsage {
    WireUsage {
        input_tokens: u.input_tokens,
        output_tokens: u.output_tokens,
        cache_read_tokens: u.cached_input_tokens,
        reasoning_tokens: u.reasoning_output_tokens,
    }
}
