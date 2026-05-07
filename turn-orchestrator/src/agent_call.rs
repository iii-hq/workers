//! `agent::call` — single LLM-facing dispatcher.
//!
//! Replaces the per-function tool catalog. The model emits exactly one tool
//! name, `agent_call`, with `{function, payload}` arguments. This module owns
//! the schema, the schema-lookup cache, and the shared dispatch helper that
//! `states::tools::handle_execute` calls into.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use harness_types::{ContentBlock, TextContent, ToolResult};
use iii_sdk::{RegisterFunctionMessage, TriggerRequest, Value, III};
use serde_json::{json, Map};

/// LLM-facing tool name (regex-safe, no `::`).
pub const TOOL_NAME: &str = "agent_call";

/// iii function id under which the dispatcher is registered on the bus.
pub const FUNCTION_ID: &str = "agent::call";

/// Default schema-cache TTL. 30s absorbs back-to-back calls within a turn
/// while still picking up worker re-registrations within a few seconds.
pub const SCHEMA_CACHE_TTL: Duration = Duration::from_secs(30);

/// Default per-call dispatch timeout. None lets iii-sdk pick its default;
/// override per call site when a tighter ceiling is wanted.
pub const DEFAULT_DISPATCH_TIMEOUT_MS: Option<u64> = None;

/// The single tool schema sent to the provider.
///
/// Snapshot-tested below — any change here is a wire-format change for every
/// active session and must be intentional.
pub fn agent_call_tool() -> Value {
    json!({
        "name": TOOL_NAME,
        "description": "Call any iii function on the bus. The argument `function` is the function id (use `::` separators, e.g. `shell::filesystem::ls`). The argument `payload` is the function-specific JSON arguments. Skills loaded into your context tell you which functions exist and what arguments they take. The result is whatever that function returns.",
        "parameters": {
            "type": "object",
            "properties": {
                "function": {
                    "type": "string",
                    "description": "iii function id to dispatch, e.g. 'shell::filesystem::ls'."
                },
                "payload": {
                    "type": "object",
                    "description": "Arguments forwarded to the function. Shape depends on the target function."
                }
            },
            "required": ["function"]
        },
        "label": "agent_call",
        "execution_mode": "parallel",
        "prepare_arguments_supported": false,
    })
}

/// Build a `ToolResult` carrying a structured error envelope. The agent
/// loop must continue regardless of failure class — never throw from the
/// dispatcher.
fn error_result(envelope: Value) -> ToolResult {
    ToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: envelope.to_string(),
        })],
        details: envelope,
        terminate: false,
    }
}

/// Validate the `function` field. Returns `Err(ToolResult)` when the field
/// is missing, empty, or not a string. The caller short-circuits without
/// touching the schema cache or `iii.trigger`.
fn validate_function_field(function: &Value) -> Result<String, ToolResult> {
    match function.as_str() {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        _ => Err(error_result(json!({
            "error": "missing_function",
            "message": "agent_call requires a non-empty `function` string field"
        }))),
    }
}

/// Per-process function-id → schema cache with a short TTL. Absorbs back-
/// to-back calls within a turn without serializing every dispatch behind a
/// `engine::functions::list` round-trip. Stale entries within the TTL window
/// surface as `invalid_payload` rejections that resolve on the next refresh.
pub struct SchemaCache {
    ttl: Duration,
    inner: HashMap<String, (Value, Instant)>,
}

impl SchemaCache {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            inner: HashMap::new(),
        }
    }

    pub fn get(&self, function_id: &str) -> Option<Value> {
        let (schema, inserted) = self.inner.get(function_id)?;
        if inserted.elapsed() >= self.ttl {
            return None;
        }
        Some(schema.clone())
    }

    pub fn insert(&mut self, function_id: &str, schema: Value) {
        self.inner
            .insert(function_id.to_string(), (schema, Instant::now()));
    }
}

/// Convert iii's `request_format` envelope into a JSON Schema usable for
/// payload validation. The envelope wraps the raw schema with `name`/
/// `description` keys; strip those and force `type: "object"`. Mirrors
/// `mcp/src/tools.rs:67-82` (kept in sync via `TODOS.md` until extracted).
pub(crate) fn format_to_input_schema(fmt: Option<&Value>) -> Value {
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

/// Fetch list from engine, warm the shared cache with every entry, return
/// schema for `function_id` if present.
async fn fetch_schemas_from_engine(
    iii: &III,
    function_id: &str,
) -> Result<Option<Value>, iii_sdk::IIIError> {
    let raw = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(5_000),
        })
        .await?;
    let entries = raw
        .as_array()
        .cloned()
        .or_else(|| {
            raw.as_object()
                .and_then(|o| o.get("functions").and_then(Value::as_array).cloned())
        })
        .unwrap_or_default();

    let mut found: Option<Value> = None;
    let mut batch: Vec<(String, Value)> = Vec::with_capacity(entries.len());
    for entry in entries {
        let id = entry
            .get("function_id")
            .or_else(|| entry.get("id"))
            .and_then(Value::as_str);
        let Some(id) = id else {
            continue;
        };
        let schema = format_to_input_schema(entry.get("request_format"));
        if id == function_id {
            found = Some(schema.clone());
        }
        batch.push((id.to_string(), schema));
    }
    {
        let mut cache = shared_cache().lock().expect("schema cache mutex poisoned");
        for (id, sch) in batch {
            cache.insert(&id, sch);
        }
    }
    Ok(found)
}

/// Fetch the target function's request schema from `engine::functions::list`,
/// caching the result. Returns `None` only when the function id is genuinely
/// not registered (vs. a transient bus error, which propagates as
/// `IIIError`). The caller maps `None` → `function_not_found` envelope.
async fn resolve_schema(iii: &III, function_id: &str) -> Result<Option<Value>, iii_sdk::IIIError> {
    {
        let cache = shared_cache().lock().expect("schema cache mutex poisoned");
        if let Some(hit) = cache.get(function_id) {
            return Ok(Some(hit));
        }
    }
    fetch_schemas_from_engine(iii, function_id).await
}

/// Validate `payload` against a JSON Schema. Returns the violations as
/// `[{path, message}, ...]` so the model can read them in the
/// `invalid_payload` envelope and self-correct.
pub(crate) fn validate_payload(payload: &Value, schema: &Value) -> Result<(), Vec<Value>> {
    let validator = match jsonschema::validator_for(schema) {
        Ok(v) => v,
        Err(e) => {
            return Err(vec![json!({
                "path": "$",
                "message": format!("schema build failed: {e}")
            })]);
        }
    };
    let errors: Vec<Value> = validator
        .iter_errors(payload)
        .map(|e| {
            json!({
                "path": e.instance_path.to_string(),
                "message": e.to_string(),
            })
        })
        .collect();
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// True iff `resp` (output of `sandbox::list`) advertises an alive sandbox
/// with `sandbox_id`. Tolerates both the bare-array and `{items: [...]}`
/// or `{sandboxes: [...]}` envelopes the engine has returned across versions.
#[must_use]
pub(crate) fn sandbox_list_contains_id(resp: &Value, sandbox_id: &str) -> bool {
    let items = resp
        .as_array()
        .or_else(|| resp.get("items").and_then(Value::as_array))
        .or_else(|| resp.get("sandboxes").and_then(Value::as_array));

    items.is_some_and(|items| {
        items.iter().any(|item| {
            if item.get("stopped").and_then(Value::as_bool) == Some(true) {
                return false;
            }

            item.get("sandbox_id")
                .or_else(|| item.get("id"))
                .and_then(Value::as_str)
                == Some(sandbox_id)
        })
    })
}

pub(crate) async fn sandbox_alive(iii: &III, sandbox_id: &str) -> bool {
    let Ok(resp) = iii
        .trigger(TriggerRequest {
            function_id: "sandbox::list".into(),
            payload: json!({}),
            action: None,
            timeout_ms: Some(30_000),
        })
        .await
    else {
        return false;
    };
    sandbox_list_contains_id(&resp, sandbox_id)
}

#[must_use]
pub(crate) fn is_function_not_found<E: std::fmt::Display>(err: &E) -> bool {
    let msg = err.to_string();
    msg.contains("function_not_found") || msg.contains("Function not found")
}

const SHELL_PREFIX: &str = "shell::";

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SandboxAction {
    Skip,
    Reuse(String),
    Create,
}

/// Pure helper — given the function id, persisted sandbox id (if any), and
/// whether that persisted sandbox is currently alive, return the action.
/// Tested in isolation; the async wrapper `ensure_sandbox` does the IO.
pub(crate) fn sandbox_decision(
    function: &str,
    persisted: Option<String>,
    alive: bool,
) -> SandboxAction {
    if !function.starts_with(SHELL_PREFIX) {
        return SandboxAction::Skip;
    }
    match persisted {
        Some(id) if alive => SandboxAction::Reuse(id),
        _ => SandboxAction::Create,
    }
}

/// Lazy sandbox provisioning. Fired only on shell::* dispatches. Mirrors
/// the load-id → alive-check → recreate-if-stale pattern from
/// `provisioning.rs` so resume-after-engine-restart keeps working.
pub(crate) async fn ensure_sandbox(iii: &III, session_id: &str, function: &str) {
    use crate::persistence;

    let persisted = persistence::load_sandbox_id(iii, session_id).await;
    let alive = match &persisted {
        Some(id) => sandbox_alive(iii, id).await,
        None => false,
    };

    match sandbox_decision(function, persisted, alive) {
        SandboxAction::Skip | SandboxAction::Reuse(_) => {}
        SandboxAction::Create => {
            let payload = json!({
                "image": "python",
                "name": format!("session-{session_id}"),
                "idle_timeout_secs": 300,
            });
            match iii
                .trigger(TriggerRequest {
                    function_id: "sandbox::create".into(),
                    payload,
                    action: None,
                    timeout_ms: Some(300_000),
                })
                .await
            {
                Ok(resp) => {
                    let new_id = resp.get("sandbox_id").and_then(Value::as_str);
                    persistence::save_sandbox_id(iii, session_id, new_id).await;
                }
                Err(ref e) if is_function_not_found(e) => {
                    tracing::warn!(
                        %session_id,
                        error = %e,
                        "sandbox::create not registered; shell::* dispatches will fail until a sandbox worker is added"
                    );
                }
                Err(e) => {
                    tracing::warn!(%session_id, error = %e, "sandbox::create failed");
                }
            }
        }
    }
}

/// Process-global schema cache shared across all dispatches.
fn shared_cache() -> &'static Mutex<SchemaCache> {
    static CACHE: OnceLock<Mutex<SchemaCache>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(SchemaCache::new(SCHEMA_CACHE_TTL)))
}

#[must_use]
pub(crate) fn is_timeout<E: std::fmt::Display>(err: &E) -> bool {
    let s = err.to_string().to_ascii_lowercase();
    s.contains("timeout") || s.contains("timed out")
}

/// If the inner function returned a `ToolResult`-shaped value, deserialize
/// it. Otherwise wrap the value as the tool's `details` so function-level
/// envelopes (`{ok: false, error}`) pass through verbatim per the spec.
pub(crate) fn decode_or_passthrough(value: Value) -> ToolResult {
    if let Ok(tr) = serde_json::from_value::<ToolResult>(value.clone()) {
        return tr;
    }
    ToolResult {
        content: vec![ContentBlock::Text(TextContent {
            text: value.to_string(),
        })],
        details: value,
        terminate: false,
    }
}

/// Shared dispatch helper. Both `agent::call`'s registered iii handler and
/// `states::tools::handle_execute` call this so policy → schema → sandbox →
/// trigger ordering has one source of truth.
pub async fn dispatch(iii: &III, session_id: &str, function: &Value, payload: Value) -> ToolResult {
    let function_id = match validate_function_field(function) {
        Ok(id) => id,
        Err(result) => return result,
    };

    let schema = match resolve_schema(iii, &function_id).await {
        Ok(Some(s)) => s,
        Ok(None) => {
            return error_result(json!({
                "error": "function_not_found",
                "function": function_id,
                "hint": "load the relevant skill via skill::fetch, or check the function id"
            }));
        }
        Err(e) => {
            return error_result(json!({
                "error": "registry_unavailable",
                "function": function_id,
                "message": e.to_string()
            }));
        }
    };

    if let Err(violations) = validate_payload(&payload, &schema) {
        return error_result(json!({
            "error": "invalid_payload",
            "function": function_id,
            "validation_errors": violations
        }));
    }

    ensure_sandbox(iii, session_id, &function_id).await;

    let response = iii
        .trigger(TriggerRequest {
            function_id: function_id.clone(),
            payload,
            action: None,
            timeout_ms: DEFAULT_DISPATCH_TIMEOUT_MS,
        })
        .await;

    match response {
        Ok(value) => decode_or_passthrough(value),
        Err(ref e) if is_timeout(e) => error_result(json!({
            "error": "timeout",
            "function": function_id,
            "message": e.to_string()
        })),
        Err(e) => error_result(json!({
            "error": "trigger_failed",
            "function": function_id,
            "message": e.to_string()
        })),
    }
}

/// Register `agent::call` as a regular iii function so it appears in
/// `engine::functions::list` and can be invoked through `bridge::trigger`
/// for testing. The browser does not call this directly — `bridge::trigger`
/// remains the browser path. The LLM reaches it via the `agent_call` tool.
pub fn register(iii: &Arc<III>) {
    let iii_clone = iii.clone();
    iii.register_function((
        RegisterFunctionMessage::with_id(FUNCTION_ID.to_string()).with_description(
            "LLM-facing dispatcher: dispatches an iii function and returns a ToolResult. \
             Validates payload against the target's request_format and provisions a \
             sandbox lazily for shell::* calls."
                .to_string(),
        ),
        move |payload: Value| {
            let iii = iii_clone.clone();
            async move {
                let session_id = payload
                    .get("session_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                let function = payload.get("function").cloned().unwrap_or(Value::Null);
                let inner_payload = payload
                    .get("payload")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                let result = dispatch(&iii, &session_id, &function, inner_payload).await;
                serde_json::to_value(&result).map_err(|e| iii_sdk::IIIError::Handler(e.to_string()))
            }
        },
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_call_tool_schema_is_stable() {
        let tool = agent_call_tool();
        assert_eq!(tool["name"], "agent_call");
        assert_eq!(tool["parameters"]["type"], "object");
        assert_eq!(tool["parameters"]["required"], json!(["function"]));
        assert_eq!(
            tool["parameters"]["properties"]["function"]["type"],
            "string"
        );
        assert_eq!(
            tool["parameters"]["properties"]["payload"]["type"],
            "object"
        );
        let desc = tool["description"].as_str().unwrap();
        assert!(desc.contains("::"), "description should mention `::`");
        assert!(
            desc.contains("function id"),
            "description should explain what `function` is"
        );
    }
}

#[cfg(test)]
mod dispatch_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn missing_function_returns_missing_function_error() {
        let result = validate_function_field(&Value::Null).unwrap_err();
        let env = &result.details;
        assert_eq!(env["error"], "missing_function");
    }

    #[test]
    fn empty_function_returns_missing_function_error() {
        let result = validate_function_field(&json!("")).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn non_string_function_returns_missing_function_error() {
        let result = validate_function_field(&json!(42)).unwrap_err();
        assert_eq!(result.details["error"], "missing_function");
    }

    #[test]
    fn valid_function_returns_owned_string() {
        let result = validate_function_field(&json!("shell::filesystem::ls")).unwrap();
        assert_eq!(result, "shell::filesystem::ls");
    }

    #[test]
    fn schema_cache_returns_value_for_known_function() {
        let mut cache = SchemaCache::new(Duration::from_secs(30));
        cache.insert("shell::filesystem::ls", json!({"type": "object"}));
        let hit = cache.get("shell::filesystem::ls").unwrap();
        assert_eq!(hit, json!({"type": "object"}));
    }

    #[test]
    fn schema_cache_returns_none_for_unknown_function() {
        let cache = SchemaCache::new(Duration::from_secs(30));
        assert!(cache.get("nope::nope").is_none());
    }

    #[test]
    fn schema_cache_expires_after_ttl() {
        let mut cache = SchemaCache::new(Duration::from_millis(0));
        cache.insert("foo::bar", json!({}));
        assert!(cache.get("foo::bar").is_none());
    }

    #[test]
    fn validate_payload_passes_when_schema_matches() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        let payload = json!({ "path": "/tmp" });
        assert!(validate_payload(&payload, &schema).is_ok());
    }

    #[test]
    fn validate_payload_collects_violations() {
        let schema = json!({
            "type": "object",
            "properties": { "path": { "type": "string" } },
            "required": ["path"]
        });
        let payload = json!({ "file": "/tmp" });
        let errors = validate_payload(&payload, &schema).unwrap_err();
        assert!(!errors.is_empty(), "should report at least one violation");
        let first = &errors[0];
        assert!(first["message"].is_string());
        assert!(first["path"].is_string());
    }

    #[test]
    fn validate_payload_rejects_wrong_type() {
        let schema = json!({
            "type": "object",
            "properties": { "count": { "type": "integer" } }
        });
        let payload = json!({ "count": "not-a-number" });
        assert!(validate_payload(&payload, &schema).is_err());
    }

    #[test]
    fn sandbox_decision_skips_non_shell_functions() {
        assert_eq!(
            sandbox_decision("skills::list", None, false),
            SandboxAction::Skip
        );
        assert_eq!(
            sandbox_decision("agent::call", Some("s1".into()), true),
            SandboxAction::Skip
        );
    }

    #[test]
    fn sandbox_decision_creates_when_no_id_persisted() {
        assert_eq!(
            sandbox_decision("shell::filesystem::ls", None, false),
            SandboxAction::Create
        );
    }

    #[test]
    fn sandbox_decision_reuses_alive_sandbox() {
        assert_eq!(
            sandbox_decision("shell::bash::exec", Some("s1".into()), true),
            SandboxAction::Reuse("s1".into())
        );
    }

    #[test]
    fn sandbox_decision_recreates_when_persisted_id_is_stale() {
        assert_eq!(
            sandbox_decision("shell::filesystem::ls", Some("dead".into()), false),
            SandboxAction::Create
        );
    }

    #[test]
    fn decode_or_passthrough_preserves_function_error_envelopes() {
        let inner = json!({ "ok": false, "error": "thing went wrong", "code": 42 });
        let tr = decode_or_passthrough(inner.clone());
        assert_eq!(tr.details, inner);
        assert!(!tr.terminate);
    }

    #[test]
    fn is_timeout_recognizes_engine_timeout_strings() {
        struct E(&'static str);
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        assert!(is_timeout(&E("trigger timed out after 240s")));
        assert!(is_timeout(&E("Timeout waiting for reply")));
        assert!(!is_timeout(&E("function_not_found")));
    }

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
    fn sandbox_list_contains_id_accepts_bare_array() {
        let resp = json!([{ "sandbox_id": "s1" }, { "id": "s2" }]);
        assert!(sandbox_list_contains_id(&resp, "s1"));
        assert!(sandbox_list_contains_id(&resp, "s2"));
    }

    #[test]
    fn sandbox_list_contains_id_accepts_items_envelope() {
        let resp = json!({ "items": [{ "sandbox_id": "s1" }] });
        assert!(sandbox_list_contains_id(&resp, "s1"));
    }

    #[test]
    fn sandbox_list_contains_id_accepts_sandboxes_envelope() {
        let resp = json!({ "sandboxes": [{ "sandbox_id": "s1" }] });
        assert!(sandbox_list_contains_id(&resp, "s1"));
    }

    #[test]
    fn sandbox_list_contains_id_rejects_stopped_sandbox() {
        let resp = json!({ "sandboxes": [{ "sandbox_id": "s1", "stopped": true }] });
        assert!(!sandbox_list_contains_id(&resp, "s1"));
    }

    #[test]
    fn sandbox_list_contains_id_rejects_missing_id() {
        let resp = json!([{ "sandbox_id": "s1" }]);
        assert!(!sandbox_list_contains_id(&resp, "s3"));
    }

    #[test]
    fn function_not_found_detector_matches_engine_error_codes() {
        struct E(&'static str);
        impl std::fmt::Display for E {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(self.0)
            }
        }
        assert!(is_function_not_found(&E(
            "remote error (function_not_found): Function sandbox::create not found"
        )));
        assert!(is_function_not_found(&E("Function not found")));
        assert!(!is_function_not_found(&E("timeout waiting for reply")));
        assert!(!is_function_not_found(&E("invocation_failed: bad payload")));
    }
}
