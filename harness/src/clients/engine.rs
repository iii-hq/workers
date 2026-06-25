//! iii engine client: function registry reads (`engine::functions::list` /
//! `engine::functions::info`) for runtime discovery post-filtering and native
//! exposure, and generic function dispatch (`iii.trigger`) for the target of
//! an `agent_trigger` call.

use std::sync::Arc;

use iii_sdk::protocol::TriggerRequest;
use iii_sdk::IIIClient;
use serde_json::{json, Value};

use crate::error::HarnessError;

/// A registry function descriptor (id + optional schemas/description). Only
/// the fields the harness reads are typed; the rest pass through as `extra`.
#[derive(Debug, Clone)]
pub struct FunctionDescriptor {
    pub function_id: String,
    pub description: Option<String>,
    pub parameters: Option<Value>,
}

#[derive(Clone)]
pub struct EngineClient {
    iii: Arc<IIIClient>,
    timeout_ms: u64,
}

impl EngineClient {
    pub fn new(iii: Arc<IIIClient>, timeout_ms: u64) -> Self {
        Self { iii, timeout_ms }
    }

    /// Dispatch an arbitrary iii function and return its raw result. This is
    /// the target invocation of an unwrapped `agent_trigger` call.
    pub async fn dispatch(&self, function_id: &str, payload: Value) -> Result<Value, HarnessError> {
        self.iii
            .trigger(TriggerRequest {
                function_id: function_id.to_string(),
                payload,
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .map_err(|e| HarnessError::Dependency(format!("{function_id}: {e}")))
    }

    /// List registry function descriptors (best-effort; empty on failure).
    pub async fn functions_list(&self) -> Vec<FunctionDescriptor> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::list".into(),
                payload: json!({}),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await;
        match resp {
            Ok(v) => parse_descriptor_list(&v),
            Err(e) => {
                tracing::warn!(error = %e, "engine::functions::list failed");
                Vec::new()
            }
        }
    }

    /// Read one function's descriptor (`None` when unknown).
    pub async fn functions_info(&self, function_id: &str) -> Option<FunctionDescriptor> {
        let resp = self
            .iii
            .trigger(TriggerRequest {
                function_id: "engine::functions::info".into(),
                payload: json!({ "function_id": function_id }),
                action: None,
                timeout_ms: Some(self.timeout_ms),
            })
            .await
            .ok()?;
        if resp.is_null() {
            return None;
        }
        descriptor_of(&resp)
    }
}

fn parse_descriptor_list(v: &Value) -> Vec<FunctionDescriptor> {
    let items: Vec<&Value> = match v {
        Value::Array(items) => items.iter().collect(),
        Value::Object(map) => map
            .get("functions")
            .or_else(|| map.get("items"))
            .and_then(Value::as_array)
            .map(|a| a.iter().collect())
            .unwrap_or_else(|| map.values().collect()),
        _ => return Vec::new(),
    };
    items.iter().filter_map(|d| descriptor_of(d)).collect()
}

/// Extract id + description + parameters schema from a descriptor in the
/// shapes engines return (`function_id` or `id`; parameters at the top level,
/// under `request_format`, or `request_schema`).
fn descriptor_of(v: &Value) -> Option<FunctionDescriptor> {
    let function_id = v
        .get("function_id")
        .or_else(|| v.get("id"))
        .or_else(|| v.get("name"))
        .and_then(Value::as_str)?
        .to_string();
    let description = v
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string);
    let parameters = v
        .get("parameters")
        .or_else(|| v.get("request_format"))
        .or_else(|| v.get("request_schema"))
        .cloned();
    Some(FunctionDescriptor {
        function_id,
        description,
        parameters,
    })
}
