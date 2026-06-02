//! `directory::engine::functions::info` — enriched detail for a single
//! engine function.
//!
//! This module restores the `directory::engine::functions::info`
//! function that was removed during the namespace consolidation. It
//! proxies the core lookup to `engine::functions::info` via
//! `iii.trigger` and returns a flat response with schemas, owning
//! worker, and registered triggers — WITHOUT the `how_guide` field
//! (removed: how-to enrichment is no longer shipped).

use std::sync::Arc;

use iii_sdk::{IIIError, RegisterFunction, TriggerRequest, III};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ────────────────── request / response shapes ─────────────────────────

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FunctionInfoInput {
    /// Fully-qualified function id on the bus (e.g. `sandbox::create`).
    pub function_id: String,
}

/// Trigger instance summary for the response envelope.
#[derive(Debug, Serialize, JsonSchema)]
pub struct RegisteredTriggerSummary {
    pub id: String,
    pub trigger_type: String,
    pub config: Value,
}

/// Response shape for `directory::engine::functions::info`.
///
/// Mirrors the shape of the old `directory::engine::functions::info`
/// but WITHOUT the `how_guide` and `related_skills` fields.
#[derive(Debug, Serialize, JsonSchema)]
pub struct FunctionInfoOutput {
    pub function_id: String,
    pub worker_name: Option<String>,
    pub description: Option<String>,
    pub request_schema: Option<Value>,
    pub response_schema: Option<Value>,
    pub metadata: Option<Value>,
    pub registered_triggers: Vec<RegisteredTriggerSummary>,
}

// ────────────────── registration ──────────────────────────────────────

pub fn register(iii: &Arc<III>) {
    let iii_inner = iii.clone();
    iii.register_function(
        "directory::engine::functions::info",
        RegisterFunction::new_async(move |req: FunctionInfoInput| {
            let iii = iii_inner.clone();
            async move { function_info(&iii, req).await.map_err(IIIError::Handler) }
        })
        .description(
            "Full detail for one engine function: schemas, owning worker, and \
             registered triggers that target it. Proxies to the engine's native \
             engine::functions::info for the core data.",
        ),
    );
}

// ────────────────── handler ──────────────────────────────────────────

async fn function_info(iii: &III, input: FunctionInfoInput) -> Result<FunctionInfoOutput, String> {
    let function_id = input.function_id.trim().to_string();
    if function_id.is_empty() {
        return Err("function_id must be non-empty".into());
    }

    // Proxy to the engine's native function info.
    let val = iii
        .trigger(TriggerRequest {
            function_id: "engine::functions::info".to_string(),
            payload: json!({ "function_id": function_id }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|e| format!("engine::functions::info proxy: {e}"))?;

    // Parse the engine response into our output shape.
    let worker_name = val
        .get("worker_name")
        .and_then(|v| v.as_str())
        .map(String::from)
        .or_else(|| id_worker_namespace(&function_id));

    let description = val
        .get("description")
        .and_then(|v| v.as_str())
        .map(String::from);

    let request_schema = val
        .get("request_format")
        .cloned()
        .or_else(|| val.get("request_schema").cloned())
        .filter(|v| !v.is_null());

    let response_schema = val
        .get("response_format")
        .cloned()
        .or_else(|| val.get("response_schema").cloned())
        .filter(|v| !v.is_null());

    let metadata = val.get("metadata").cloned().filter(|v| !v.is_null());

    // Parse registered triggers from the response if present.
    let registered_triggers = val
        .get("registered_triggers")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|t| {
                    let id = t.get("id")?.as_str()?.to_string();
                    let trigger_type = t.get("trigger_type")?.as_str()?.to_string();
                    let config = t.get("config").cloned().unwrap_or(json!({}));
                    Some(RegisteredTriggerSummary {
                        id,
                        trigger_type,
                        config,
                    })
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(FunctionInfoOutput {
        function_id,
        worker_name,
        description,
        request_schema,
        response_schema,
        metadata,
        registered_triggers,
    })
}

/// First `::` segment, used as a fallback worker-name attribution.
fn id_worker_namespace(id: &str) -> Option<String> {
    match id.split_once("::") {
        Some((ns, _)) if !ns.is_empty() => Some(ns.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_shape_has_no_how_guide_field() {
        let output = FunctionInfoOutput {
            function_id: "sandbox::create".into(),
            worker_name: Some("sandbox".into()),
            description: Some("Boot a sandbox.".into()),
            request_schema: None,
            response_schema: None,
            metadata: None,
            registered_triggers: Vec::new(),
        };
        let v = serde_json::to_value(&output).unwrap();
        assert!(
            v.get("how_guide").is_none(),
            "how_guide field must NOT be present in the response shape"
        );
        assert!(
            v.get("related_skills").is_none(),
            "related_skills field must NOT be present in the response shape"
        );
        assert_eq!(v["function_id"], "sandbox::create");
        assert_eq!(v["worker_name"], "sandbox");
    }

    #[test]
    fn id_worker_namespace_extracts_first_segment() {
        assert_eq!(
            id_worker_namespace("sandbox::create"),
            Some("sandbox".into())
        );
        assert_eq!(id_worker_namespace("mem::observe"), Some("mem".into()));
        assert_eq!(id_worker_namespace("bare"), None);
        assert_eq!(id_worker_namespace(""), None);
    }
}
