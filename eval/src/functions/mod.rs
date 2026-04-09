pub mod analyze;
pub mod baseline;
pub mod drift;
pub mod ingest;
pub mod metrics;
pub mod report;
pub mod score;
pub mod state;

use iii_sdk::{IIIError, RegisterFunctionMessage, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::EvalConfig;

pub fn register_all(iii: &Arc<III>, config: &Arc<EvalConfig>) {
    register_ingest(iii, config);
    register_metrics(iii);
    register_score(iii);
    register_drift(iii, config);
    register_baseline(iii);
    register_report(iii, config);
    register_analyze_traces(iii);

    tracing::info!("all 7 eval functions registered");
}

fn register_ingest(iii: &Arc<III>, config: &Arc<EvalConfig>) {
    let iii_clone = iii.clone();
    let config_clone = config.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::ingest".to_string(),
            description: Some("Ingest function execution span data".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" },
                    "duration_ms": { "type": "integer" },
                    "success": { "type": "boolean" },
                    "error": { "type": "string" },
                    "input_hash": { "type": "string" },
                    "output_hash": { "type": "string" },
                    "timestamp": { "type": "string", "format": "date-time" },
                    "trace_id": { "type": "string" },
                    "worker_id": { "type": "string" }
                },
                "required": ["function_id", "duration_ms", "success"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "ingested": { "type": "boolean" },
                    "function_id": { "type": "string" },
                    "total_spans": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let cfg = config_clone.clone();
            Box::pin(async move {
                let result = ingest::handle(&iii, &cfg, payload).await?;

                let fid = result
                    .get("function_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();

                if !fid.is_empty() {
                    let index_val = state::state_get(&iii, ingest::SCOPE_INDEX, ingest::INDEX_KEY)
                        .await
                        .unwrap_or(json!(null));
                    let mut index: Vec<String> = if index_val.is_array() {
                        serde_json::from_value(index_val).unwrap_or_else(|e| {
                            tracing::warn!(error = %e, "failed to deserialize function index");
                            Vec::new()
                        })
                    } else {
                        Vec::new()
                    };

                    if !index.contains(&fid) {
                        index.push(fid);
                        if let Err(e) = state::state_set(&iii, ingest::SCOPE_INDEX, ingest::INDEX_KEY, json!(index)).await {
                            tracing::warn!(error = %e, "failed to update function index");
                        }
                    }
                }

                Ok(result)
            })
        },
    );
}

fn register_metrics(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::metrics".to_string(),
            description: Some("Calculate metrics for a tracked function".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" }
                },
                "required": ["function_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" },
                    "p50_ms": { "type": "integer" },
                    "p95_ms": { "type": "integer" },
                    "p99_ms": { "type": "integer" },
                    "success_rate": { "type": "number" },
                    "total_invocations": { "type": "integer" },
                    "avg_duration_ms": { "type": "number" },
                    "error_count": { "type": "integer" },
                    "throughput_per_min": { "type": "number" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { metrics::handle(&iii, payload).await })
        },
    );
}

fn register_score(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::score".to_string(),
            description: Some("Score overall system health across all tracked functions".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "overall_score": { "type": "integer", "minimum": 0, "maximum": 100 },
                    "issues": { "type": "array" },
                    "suggestions": { "type": "array" },
                    "functions_evaluated": { "type": "integer" },
                    "timestamp": { "type": "string", "format": "date-time" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { score::handle(&iii, payload).await })
        },
    );
}

fn register_drift(iii: &Arc<III>, config: &Arc<EvalConfig>) {
    let iii_clone = iii.clone();
    let config_clone = config.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::drift".to_string(),
            description: Some("Detect metric drift against saved baselines".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" }
                }
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "results": { "type": "array" },
                    "threshold": { "type": "number" },
                    "timestamp": { "type": "string", "format": "date-time" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let cfg = config_clone.clone();
            Box::pin(async move { drift::handle(&iii, &cfg, payload).await })
        },
    );
}

fn register_baseline(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::baseline".to_string(),
            description: Some("Save current metrics as baseline snapshot".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "function_id": { "type": "string" }
                },
                "required": ["function_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "saved": { "type": "boolean" },
                    "function_id": { "type": "string" },
                    "baseline": { "type": "object" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { baseline::handle(&iii, payload).await })
        },
    );
}

fn register_report(iii: &Arc<III>, config: &Arc<EvalConfig>) {
    let iii_clone = iii.clone();
    let config_clone = config.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::report".to_string(),
            description: Some("Generate full evaluation report with metrics, scores, and drift".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {}
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "functions": { "type": "array" },
                    "score": { "type": "object" },
                    "total_functions": { "type": "integer" },
                    "timestamp": { "type": "string", "format": "date-time" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let cfg = config_clone.clone();
            Box::pin(async move { report::handle(&iii, &cfg, payload).await })
        },
    );
}

fn register_analyze_traces(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "eval::analyze_traces".to_string(),
            description: Some("Analyze production OTel traces — top functions, slowest, error rates, insights".to_string()),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "limit": { "type": "integer", "description": "Max traces to fetch (default 100)" },
                    "function_filter": { "type": "string", "description": "Filter traces by function_id substring" }
                }
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "summary": {
                        "type": "object",
                        "properties": {
                            "total_spans": { "type": "integer" },
                            "unique_functions": { "type": "integer" },
                            "time_range": { "type": "object" },
                            "error_rate": { "type": "number" }
                        }
                    },
                    "slowest_functions": { "type": "array" },
                    "most_active": { "type": "array" },
                    "errors": { "type": "array" },
                    "insights": { "type": "array", "items": { "type": "string" } }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { analyze::handle(&iii, payload).await })
        },
    );
}
