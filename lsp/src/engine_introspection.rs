use iii_sdk::errors::Error;
use iii_sdk::{CallRequest, IIIClient};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Function information returned by `engine::functions::list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FunctionInfo {
    pub function_id: String,
    pub description: Option<String>,
    pub request_format: Option<Value>,
    pub response_format: Option<Value>,
    pub metadata: Option<Value>,
    /// JSON Schema of an observer's `args`; absent when the function takes
    /// none, and its observers select calls with filters.
    #[serde(default)]
    pub args_format: Option<Value>,
    /// Only the worker that hosts the function calls it. Anyone may observe
    /// it; it cannot be an observer's target.
    #[serde(default)]
    pub private: bool,
    /// `false`: nobody may observe the function.
    #[serde(default = "observable_default")]
    pub observable: bool,
    /// Live observers of the function.
    #[serde(default)]
    pub observer_count: u64,
}

fn observable_default() -> bool {
    true
}

impl Default for FunctionInfo {
    /// A public, observable function without formats, args or observers.
    fn default() -> Self {
        Self {
            function_id: String::new(),
            description: None,
            request_format: None,
            response_format: None,
            metadata: None,
            args_format: None,
            private: false,
            observable: true,
            observer_count: 0,
        }
    }
}

/// The function an observer delivers to, as `engine::observers::list`
/// reports it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObserverTargetInfo {
    pub function_id: String,
    #[serde(default)]
    pub namespace: Option<String>,
}

/// Observer information returned by `engine::observers::list`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObserverInfo {
    pub id: String,
    /// The observed function.
    pub function_id: String,
    #[serde(default)]
    pub namespace: Option<String>,
    /// The observer's args, read by the observed function's worker.
    #[serde(default)]
    pub args: Value,
    /// The observer's filters, in the engine's normal form.
    #[serde(default)]
    pub filters: Value,
    pub target: ObserverTargetInfo,
    #[serde(default)]
    pub state: Option<String>,
}

/// Worker information returned by `engine::workers::list`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkerInfo {
    pub id: String,
    pub name: Option<String>,
    pub runtime: Option<String>,
    pub version: Option<String>,
    pub os: Option<String>,
    pub ip_address: Option<String>,
    pub status: String,
    pub connected_at_ms: u64,
    pub function_count: usize,
    pub functions: Vec<String>,
    pub active_invocations: usize,
    #[serde(default)]
    pub isolation: Option<String>,
}

/// Call an engine builtin with an empty payload and read the list under `key`.
async fn list<T: serde::de::DeserializeOwned>(
    iii: &IIIClient,
    function_id: &str,
    key: &str,
) -> Result<Vec<T>, Error> {
    let result = iii
        .call(CallRequest::new(function_id, serde_json::json!({})))
        .await?;
    Ok(result
        .get(key)
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default())
}

pub async fn list_functions(iii: &IIIClient) -> Result<Vec<FunctionInfo>, Error> {
    list(iii, "engine::functions::list", "functions").await
}

pub async fn list_workers(iii: &IIIClient) -> Result<Vec<WorkerInfo>, Error> {
    list(iii, "engine::workers::list", "workers").await
}

pub async fn list_observers(iii: &IIIClient) -> Result<Vec<ObserverInfo>, Error> {
    list(iii, "engine::observers::list", "observers").await
}
