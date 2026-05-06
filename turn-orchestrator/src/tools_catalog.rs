//! Build the LLM-callable tool catalog by enumerating every function
//! registered with the engine and filtering out control-plane prefixes.
//!
//! The harness uses this in place of a static, client-supplied tool array:
//! whatever workers are running on the bus determines what tools the model
//! sees on each fresh `run::start`. Mid-session adds aren't picked up
//! (intentional — schemas are frozen for the session, mirroring the
//! pre-existing behavior when callers passed `tools` in the run request).
//!
//! The two helpers `unwrap_function_list` and `format_to_input_schema` are
//! copied verbatim from `mcp/src/tools.rs:67-94` per a deliberate
//! plan-eng-review decision (D5) to keep the harness independent of the
//! mcp crate. Keep in sync until extracted to a shared crate (TODO-2).

use harness_types::{AgentTool, ExecutionMode};
use iii_sdk::{IIIError, TriggerRequest, III};
use serde_json::{json, Map, Value};

/// Built-in iii engine function id used to enumerate registered functions.
const ENGINE_LIST_FUNCTIONS: &str = "engine::functions::list";

/// Function-id prefixes the harness never advertises to the LLM. Keep the
/// list intentionally explicit — the principled answer is per-function
/// `metadata.mcp.expose` opt-in (tracked as TODO-1); this denylist is the
/// stopgap until we migrate.
const DENIED_PREFIXES: &[&str] = &[
    "engine::",
    "harness::",
    "bridge::",
    "skills::",
    "prompts::",
    "approval::",
    "router::",
    "sessions::",
    "session-tree::",
    "models::",
    "auth::",
    "oauth::",
    "llm-budget::",
    "policy::",
];

/// Some engines wrap the list under a `{ functions: [...] }` envelope; some
/// return the array directly. Accept both shapes. Copied verbatim from
/// `mcp/src/tools.rs:84-94`.
fn unwrap_function_list(raw: &Value) -> Vec<Map<String, Value>> {
    if let Some(arr) = raw.as_array() {
        return arr.iter().filter_map(|v| v.as_object().cloned()).collect();
    }
    if let Some(obj) = raw.as_object() {
        if let Some(arr) = obj.get("functions").and_then(Value::as_array) {
            return arr.iter().filter_map(|v| v.as_object().cloned()).collect();
        }
    }
    Vec::new()
}

/// Convert iii's `request_format` envelope into a JSON Schema usable as a
/// tool-parameters schema. Copied from `mcp/src/tools.rs:67-82`. The engine's
/// envelope wraps the raw schema with `name`/`description` keys; strip those
/// and force `type: "object"` since LLM tool calls always pass an object.
fn format_to_input_schema(fmt: Option<&Value>) -> Value {
    let Some(obj) = fmt.and_then(Value::as_object) else {
        return json!({"type": "object", "properties": {}});
    };
    let mut out = Map::new();
    out.insert("type".to_string(), Value::String("object".to_string()));
    for (k, v) in obj {
        if k == "name" || k == "description" {
            continue;
        }
        out.insert(k.clone(), v.clone());
    }
    Value::Object(out)
}

/// True iff `function_id` should be advertised to the LLM. A function-id is
/// included when (a) it contains a `::` namespace separator (worker-scoped)
/// and (b) it does not start with any [`DENIED_PREFIXES`] entry.
fn should_advertise(function_id: &str) -> bool {
    if !function_id.contains("::") {
        return false;
    }
    !DENIED_PREFIXES.iter().any(|p| function_id.starts_with(p))
}

/// Resolve the human-readable label for a tool. Workers set
/// `metadata.tool.label` at registration; if missing, fall back to the last
/// `::`-separated segment of the function id (e.g. `shell::filesystem::ls`
/// → `ls`). The fallback keeps brand-new tools usable, but a missing label
/// is a worker bug.
fn extract_label(function_id: &str, metadata: Option<&Value>) -> String {
    metadata
        .and_then(|m| m.get("tool"))
        .and_then(|t| t.get("label"))
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            function_id
                .rsplit("::")
                .next()
                .unwrap_or(function_id)
                .to_string()
        })
}

/// Build a single [`AgentTool`] from one engine function descriptor.
fn function_to_agent_tool(fn_obj: &Map<String, Value>) -> Option<AgentTool> {
    let name = fn_obj
        .get("function_id")
        .and_then(Value::as_str)?
        .to_string();
    let description = fn_obj
        .get("description")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let parameters = format_to_input_schema(fn_obj.get("request_format"));
    let label = extract_label(&name, fn_obj.get("metadata"));
    Some(AgentTool {
        name,
        description,
        parameters,
        label,
        execution_mode: ExecutionMode::default(),
        prepare_arguments_supported: false,
    })
}

/// Empty-catalog error. Surfaces as `agent::error` in the SSE stream so the
/// user sees a clear "no tools registered" instead of a model that won't
/// call anything (per plan-eng-review D4).
fn empty_catalog_error() -> IIIError {
    IIIError::Handler(
        "tool catalog is empty — engine::functions::list returned no advertise-able \
         functions; the engine may not have finished registering workers yet"
            .into(),
    )
}

/// Pure helper: filter and convert a raw `engine::functions::list` response
/// into the LLM tool catalog. Split out so the filter, conversion, and
/// empty-catalog error path are testable without a live engine.
pub fn build_catalog_from_response(raw: &Value) -> Result<Vec<AgentTool>, IIIError> {
    let functions = unwrap_function_list(raw);
    let catalog: Vec<AgentTool> = functions
        .into_iter()
        .filter(|f| {
            f.get("function_id")
                .and_then(Value::as_str)
                .is_some_and(should_advertise)
        })
        .filter_map(|f| function_to_agent_tool(&f))
        .collect();
    if catalog.is_empty() {
        return Err(empty_catalog_error());
    }
    Ok(catalog)
}

/// Trigger `engine::functions::list` and convert the result into a catalog
/// of [`AgentTool`]s the harness can advertise to the model. Returns the
/// empty-catalog error when filtering leaves zero tools.
pub async fn load_tool_catalog(iii: &III) -> Result<Vec<AgentTool>, IIIError> {
    let raw = iii
        .trigger(TriggerRequest {
            function_id: ENGINE_LIST_FUNCTIONS.to_string(),
            payload: json!({}),
            action: None,
            timeout_ms: None,
        })
        .await?;
    build_catalog_from_response(&raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fn_entry(id: &str) -> Value {
        json!({ "function_id": id, "description": "", "request_format": null })
    }

    fn fn_entry_full(id: &str, label: &str, schema: Value) -> Value {
        json!({
            "function_id": id,
            "description": format!("desc for {id}"),
            "request_format": schema,
            "metadata": { "tool": { "label": label } },
        })
    }

    // ---- unwrap_function_list (mirror mcp/src/tools.rs:271-278) ----

    #[test]
    fn unwrap_function_list_handles_array_envelope() {
        let raw = json!([{"function_id": "a"}, {"function_id": "b"}]);
        assert_eq!(unwrap_function_list(&raw).len(), 2);
    }

    #[test]
    fn unwrap_function_list_handles_object_envelope() {
        let raw = json!({"functions": [{"function_id": "a"}]});
        assert_eq!(unwrap_function_list(&raw).len(), 1);
    }

    #[test]
    fn unwrap_function_list_returns_empty_on_null_or_unrelated() {
        assert!(unwrap_function_list(&json!(null)).is_empty());
        assert!(unwrap_function_list(&json!({"unrelated": 1})).is_empty());
    }

    // ---- should_advertise filter ----

    #[test]
    fn filter_keeps_namespaced_worker_tools() {
        assert!(should_advertise("shell::bash::exec"));
        assert!(should_advertise("shell::filesystem::ls"));
        assert!(should_advertise("subagent::start"));
        assert!(should_advertise("skill::fetch"));
    }

    #[test]
    fn filter_drops_engine_namespace() {
        assert!(!should_advertise("engine::functions::list"));
        assert!(!should_advertise("engine::status"));
    }

    #[test]
    fn filter_drops_each_denylist_prefix() {
        // Every prefix in DENIED_PREFIXES must reject. Pinning each one
        // keeps a denylist regression visible at PR time.
        for prefix in DENIED_PREFIXES {
            let id = format!("{prefix}some_fn");
            assert!(
                !should_advertise(&id),
                "expected '{id}' to be filtered out via prefix '{prefix}'"
            );
        }
    }

    #[test]
    fn filter_drops_unnamespaced_function_ids() {
        // Anything without `::` is a bare name (engine builtin or test fixture);
        // never advertise it.
        assert!(!should_advertise("noprefix"));
        assert!(!should_advertise(""));
    }

    // ---- format_to_input_schema ----

    #[test]
    fn format_to_input_schema_round_trips_well_formed_request_format() {
        let fmt = json!({
            "name": "ls_input",
            "description": "envelope desc to drop",
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        let schema = format_to_input_schema(Some(&fmt));
        assert_eq!(schema["type"], "object");
        assert!(schema.get("name").is_none());
        assert!(schema.get("description").is_none());
        assert_eq!(schema["properties"]["path"]["type"], "string");
        assert_eq!(schema["required"][0], "path");
    }

    #[test]
    fn format_to_input_schema_returns_empty_object_when_missing() {
        let schema = format_to_input_schema(None);
        assert_eq!(schema, json!({"type": "object", "properties": {}}));
    }

    // ---- label resolution ----

    #[test]
    fn extract_label_uses_metadata_tool_label_when_present() {
        let metadata = json!({ "tool": { "label": "List directory" } });
        assert_eq!(
            extract_label("shell::filesystem::ls", Some(&metadata)),
            "List directory"
        );
    }

    #[test]
    fn extract_label_falls_back_to_last_segment_when_metadata_missing() {
        assert_eq!(extract_label("shell::filesystem::ls", None), "ls");
        let metadata = json!({ "tool": { "other": "x" } });
        assert_eq!(extract_label("shell::bash::exec", Some(&metadata)), "exec");
    }

    // ---- AgentTool construction ----

    #[test]
    fn function_to_agent_tool_populates_all_fields_on_happy_path() {
        let entry = fn_entry_full(
            "shell::filesystem::ls",
            "List directory",
            json!({"type": "object", "properties": {"path": {"type": "string"}}}),
        );
        let obj = entry.as_object().unwrap();
        let tool = function_to_agent_tool(obj).expect("tool");
        assert_eq!(tool.name, "shell::filesystem::ls");
        assert_eq!(tool.label, "List directory");
        assert!(tool.description.contains("desc for"));
        assert_eq!(tool.parameters["type"], "object");
        assert_eq!(tool.execution_mode, ExecutionMode::Parallel);
        assert!(!tool.prepare_arguments_supported);
    }

    // ---- end-to-end via build_catalog_from_response ----

    #[test]
    fn build_catalog_filters_and_returns_only_advertise_able_tools() {
        let raw = json!({
            "functions": [
                fn_entry_full(
                    "shell::filesystem::ls",
                    "List directory",
                    json!({"type": "object", "properties": {"path": {"type": "string"}}}),
                ),
                fn_entry("engine::functions::list"),
                fn_entry("harness::status"),
                fn_entry("skills::register"),
                fn_entry_full(
                    "skill::fetch",
                    "Fetch skill",
                    json!({"type": "object", "properties": {}}),
                ),
            ]
        });
        let catalog = build_catalog_from_response(&raw).expect("non-empty");
        let names: Vec<&str> = catalog.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, vec!["shell::filesystem::ls", "skill::fetch"]);
        assert_eq!(catalog[0].label, "List directory");
        assert_eq!(catalog[1].label, "Fetch skill");
    }

    #[test]
    fn build_catalog_returns_empty_catalog_error_when_filter_leaves_nothing() {
        let raw = json!({
            "functions": [
                fn_entry("engine::functions::list"),
                fn_entry("harness::status"),
                fn_entry("approval::resolve"),
            ]
        });
        let err = build_catalog_from_response(&raw).expect_err("empty catalog");
        let msg = err.to_string();
        assert!(msg.contains("tool catalog is empty"), "got: {msg}");
    }

    #[test]
    fn build_catalog_returns_empty_catalog_error_when_engine_returns_empty_array() {
        let raw = json!([]);
        let err = build_catalog_from_response(&raw).expect_err("empty catalog");
        assert!(err.to_string().contains("tool catalog is empty"));
    }

    #[test]
    fn build_catalog_returns_empty_catalog_error_when_engine_returns_null() {
        let raw = json!(null);
        let err = build_catalog_from_response(&raw).expect_err("empty catalog");
        assert!(err.to_string().contains("tool catalog is empty"));
    }

    #[test]
    fn function_to_agent_tool_skips_entries_without_function_id() {
        let entry = json!({"description": "missing id"});
        assert!(function_to_agent_tool(entry.as_object().unwrap()).is_none());
    }
}
