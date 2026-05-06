//! `skills` — agentic content registry worker. The binary in
//! `src/main.rs` is a thin wrapper that wires the modules below to the iii
//! engine.
//!
//! This worker owns two surfaces that were previously bundled inside
//! `iii-mcp`:
//!
//!   * **Skills** (`skills::*`): a state-backed markdown registry keyed
//!     by short skill ids. Skills are surfaced through the `iii://`
//!     resource URI scheme and read by the `mcp` worker when it serves
//!     `resources/list` / `resources/read`.
//!   * **Prompts** (`prompts::*`): a state-backed registry of
//!     slash-command templates. The `mcp` worker reads them via
//!     `prompts::mcp-list` / `prompts::mcp-get` when answering
//!     `prompts/list` and `prompts/get`.
//!
//! Mutations of either scope fan out through custom trigger types
//! `skills::on-change` and `prompts::on-change` so that the `mcp` worker
//! (or any other interested subscriber) can forward MCP
//! `notifications/*_list_changed` to its clients.

pub mod config;
pub mod functions;
pub mod manifest;
pub mod state;
pub mod trigger_types;

/// A single LLM-callable tool exported by a worker alongside its skill body.
///
/// Chat clients (e.g. the console chat panel) read these descriptors from
/// `skills::list` and build the LLM tool array dynamically — they don't
/// need to hard-code knowledge of each worker's function catalogue.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub struct ToolDescriptor {
    /// Full namespaced tool name. **Must** start with `{worker_id}::`.
    /// The server enforces this constraint at registration time via
    /// [`validate_tool_namespace`].
    pub name: String,
    /// Human-readable purpose. Chat clients show this in the UI.
    /// **Do not embed LLM prompting instructions here** — clients sanitize
    /// this field before passing it to the model.
    pub description: String,
    /// Standard JSON Schema object describing the tool's arguments. The LLM
    /// uses this to construct valid calls.
    pub input_schema: serde_json::Value,
    /// Optional per-call max-bytes contract. Chat clients truncate tool
    /// results that exceed this length before including them in context.
    #[serde(default)]
    pub max_bytes: Option<usize>,
}

/// Validate that every tool in `tools` is namespaced under `worker_id`.
///
/// Each tool name must begin with `"{worker_id}::"`. This mirrors the
/// hard-floor convention in the engine and prevents one worker from
/// squatting on another worker's namespace.
///
/// Returns `Ok(())` when all names pass or the slice is empty; otherwise
/// returns the first violation as an error string.
pub fn validate_tool_namespace(worker_id: &str, tools: &[ToolDescriptor]) -> Result<(), String> {
    let prefix = format!("{worker_id}::");
    for t in tools {
        if !t.name.starts_with(&prefix) {
            return Err(format!(
                "tool '{}' must be namespaced under registering worker '{worker_id}' \
                 (expected prefix '{prefix}')",
                t.name
            ));
        }
    }
    Ok(())
}
