pub mod create;
pub mod decide;
pub mod loop_run;
pub mod propose;
pub mod run;
pub mod status;
pub mod stop;

use iii_sdk::{IIIError, RegisterFunctionMessage, III};
use serde_json::{json, Value};
use std::sync::Arc;

use crate::config::ExperimentConfig;

pub fn register_all(iii: &Arc<III>, config: &Arc<ExperimentConfig>) {
    register_create(iii, config);
    register_propose(iii);
    register_run(iii, config);
    register_decide(iii);
    register_loop(iii, config);
    register_status(iii);
    register_stop(iii);

    tracing::info!("all 7 experiment functions registered");
}

fn register_create(iii: &Arc<III>, config: &Arc<ExperimentConfig>) {
    let iii_clone = iii.clone();
    let config_clone = config.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::create".to_string(),
            description: Some(
                "Create a new optimization experiment with target and metric functions".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "target_function": { "type": "string", "description": "Function ID to optimize" },
                    "metric_function": { "type": "string", "description": "Function ID that returns a measurable score" },
                    "metric_path": { "type": "string", "description": "JSON path to extract score from metric result" },
                    "direction": { "type": "string", "enum": ["minimize", "maximize"] },
                    "budget": { "type": "integer", "description": "Max iterations to run" },
                    "description": { "type": "string" },
                    "target_payload": { "type": "object", "description": "Base payload for target function" },
                    "metric_payload": { "type": "object", "description": "Payload for metric function" }
                },
                "required": ["target_function", "metric_function", "metric_path", "direction"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" },
                    "baseline_score": { "type": "number" },
                    "status": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let cfg = config_clone.clone();
            Box::pin(async move { create::handle(iii, cfg, payload).await })
        },
    );
}

fn register_propose(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::propose".to_string(),
            description: Some(
                "Generate a hypothesis with modified parameters for the target function".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" }
                },
                "required": ["experiment_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "proposal_id": { "type": "string" },
                    "hypothesis": { "type": "string" },
                    "modified_payload": { "type": "object" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { propose::handle(iii, payload).await })
        },
    );
}

fn register_run(iii: &Arc<III>, config: &Arc<ExperimentConfig>) {
    let iii_clone = iii.clone();
    let config_clone = config.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::run".to_string(),
            description: Some(
                "Run a single experiment iteration: propose, execute target, measure metric, decide"
                    .to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" },
                    "proposal_id": { "type": "string", "description": "Optional pre-existing proposal" }
                },
                "required": ["experiment_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "iteration": { "type": "integer" },
                    "score": { "type": "number" },
                    "baseline_score": { "type": "number" },
                    "best_score": { "type": "number" },
                    "kept": { "type": "boolean" },
                    "improvement_pct": { "type": "number" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let cfg = config_clone.clone();
            Box::pin(async move { run::handle(iii, cfg, payload).await })
        },
    );
}

fn register_decide(iii: &Arc<III>) {
    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::decide".to_string(),
            description: Some(
                "Compare a score against current best and decide keep or discard".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" },
                    "score": { "type": "number" },
                    "iteration": { "type": "integer" },
                    "current_best": { "type": "number" },
                    "direction": { "type": "string", "enum": ["minimize", "maximize"] }
                },
                "required": ["score"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" },
                    "iteration": { "type": "integer" },
                    "kept": { "type": "boolean" },
                    "reason": { "type": "string" },
                    "improvement_pct": { "type": "number" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            Box::pin(async move { decide::handle(payload).await })
        },
    );
}

fn register_loop(iii: &Arc<III>, config: &Arc<ExperimentConfig>) {
    let iii_clone = iii.clone();
    let config_clone = config.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::loop".to_string(),
            description: Some(
                "Run the full optimization loop: propose, run, measure, decide for N iterations"
                    .to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" }
                },
                "required": ["experiment_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" },
                    "total_runs": { "type": "integer" },
                    "kept_count": { "type": "integer" },
                    "best_score": { "type": "number" },
                    "baseline_score": { "type": "number" },
                    "total_improvement_pct": { "type": "number" },
                    "status": { "type": "string" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            let cfg = config_clone.clone();
            Box::pin(async move { loop_run::handle(iii, cfg, payload).await })
        },
    );
}

fn register_status(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::status".to_string(),
            description: Some(
                "Get current status, progress, and history of an experiment".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" }
                },
                "required": ["experiment_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" },
                    "status": { "type": "string" },
                    "iterations_completed": { "type": "integer" },
                    "budget": { "type": "integer" },
                    "best_score": { "type": "number" },
                    "baseline_score": { "type": "number" },
                    "improvement_pct": { "type": "number" },
                    "history": { "type": "array" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { status::handle(iii, payload).await })
        },
    );
}

fn register_stop(iii: &Arc<III>) {
    let iii_clone = iii.clone();

    iii.register_function_with(
        RegisterFunctionMessage {
            id: "experiment::stop".to_string(),
            description: Some(
                "Stop a running experiment after the current iteration completes".to_string(),
            ),
            request_format: Some(json!({
                "type": "object",
                "properties": {
                    "experiment_id": { "type": "string" }
                },
                "required": ["experiment_id"]
            })),
            response_format: Some(json!({
                "type": "object",
                "properties": {
                    "stopped": { "type": "boolean" },
                    "experiment_id": { "type": "string" },
                    "iterations_completed": { "type": "integer" }
                }
            })),
            metadata: None,
            invocation: None,
        },
        move |payload: Value| -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Value, IIIError>> + Send>> {
            let iii = iii_clone.clone();
            Box::pin(async move { stop::handle(iii, payload).await })
        },
    );
}
