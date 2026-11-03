//! `directory::engine::functions::info` — enriched detail for a single
//! engine function.
//!
//! This module restores the `directory::engine::functions::info`
//! function that was removed during the namespace consolidation. It
//! proxies the core lookup to `engine::functions::info` via
//! `iii.call` and returns a flat response with schemas, owning
//! worker, its observers' args format and observer count, and whether it
//! is private or observable — WITHOUT the `how_guide` field (how-to
//! enrichment is not shipped).

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ────────────────── request / response shapes ─────────────────────────

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct FunctionInfoInput {
    /// Fully-qualified function id on the bus (e.g. `sandbox::create`).
    pub function_id: String,
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
    /// JSON Schema of the args its observers pass; `None` when it takes none.
    pub args_format: Option<Value>,
    /// How many observers the function has.
    pub observer_count: u64,
    /// Only its own worker may call it; others may observe it.
    pub private: bool,
    /// `false` when nobody may observe it.
    pub observable: bool,
}

// ────────────────── registration ──────────────────────────────────────

pub fn register(iii: &Arc<IIIClient>) {
    let iii_inner = iii.clone();
    iii.register_function(
        "directory::engine::functions::info",
        RegisterFunction::new_async(move |req: FunctionInfoInput| {
            let iii = iii_inner.clone();
            async move { function_info(&iii, req).await.map_err(Error::Handler) }
        })
        .description(
            "Read full detail for one engine function: schemas, owning worker, the args \
             its observers pass (args_format), its observer count, and whether it is \
             private or observable. Proxies to the engine's native \
             engine::functions::info.",
        ),
    );
}

// ────────────────── handler ──────────────────────────────────────────

async fn function_info(
    iii: &IIIClient,
    input: FunctionInfoInput,
) -> Result<FunctionInfoOutput, String> {
    let function_id = input.function_id.trim().to_string();
    if function_id.is_empty() {
        return Err("function_id must be non-empty".into());
    }

    // Proxy to the engine's native function info.
    let val = iii
        .call(CallRequest {
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

    let args_format = val.get("args_format").cloned().filter(|v| !v.is_null());
    let observer_count = val
        .get("observer_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    // The engine sends `private` only when true and `observable` only when
    // false.
    let private = val.get("private").and_then(Value::as_bool).unwrap_or(false);
    let observable = val
        .get("observable")
        .and_then(Value::as_bool)
        .unwrap_or(true);

    Ok(FunctionInfoOutput {
        function_id,
        worker_name,
        description,
        request_schema,
        response_schema,
        metadata,
        args_format,
        observer_count,
        private,
        observable,
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
            args_format: None,
            observer_count: 0,
            private: false,
            observable: true,
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
