//! Register the `devin::*` surface. `devin::run` drives the local `devin` CLI;
//! `devin::api` is a raw pass-through to any v3 endpoint; the typed
//! `devin::session::*` wrappers cover the Devin cloud session lifecycle.
//! Schemas are published via request_format so `iii trigger <fn> --help` prints
//! the parameter table.

pub mod types;

use iii_sdk::errors::Error;
use iii_sdk::{IIIClient, RegisterFunction};
use reqwest::Client;
use serde_json::{json, Value};

use crate::api;
use crate::cli;
use crate::configuration::ConfigCell;
use crate::state::{list_sessions, load_session};
use types::{
    extract_create_prompt, ApiRequest, CodeScanFindingsRequest, CodeScanMetricsRequest,
    CodeScanRemediateRequest, PrReviewStatusRequest, PrReviewTriggerRequest, RunRequest,
    SessionCreateRequest, SessionIdRequest, SessionListRequest, SessionMessageRequest,
};

fn schema_value<T: schemars::JsonSchema>() -> Value {
    let root = schemars::gen::SchemaGenerator::default().into_root_schema_for::<T>();
    let mut v = serde_json::to_value(root).expect("schema serializes");
    // The registry publish validator has no meta-schema registered, so the
    // `$schema` key schemars stamps at the root fails publish. Strip it.
    if let Some(obj) = v.as_object_mut() {
        obj.remove("$schema");
    }
    v
}

fn passthrough_schema() -> Value {
    json!({ "type": "object", "additionalProperties": true })
}

fn err_value(e: impl std::fmt::Display) -> Value {
    json!({ "is_error": true, "error": e.to_string() })
}

pub fn register_all(iii: &IIIClient, cell: ConfigCell, http: Client) {
    register_run(iii, &cell);
    register_stop(iii);
    register_status(iii);
    register_runs_list(iii);
    register_api(iii, &cell, &http);
    register_session_create(iii, &cell, &http);
    register_session_get(iii, &cell, &http);
    register_session_list(iii, &cell, &http);
    register_session_message(iii, &cell, &http);
    register_pr_review_trigger(iii, &cell, &http);
    register_pr_review_status(iii, &cell, &http);
    register_code_scan_findings(iii, &cell, &http);
    register_code_scan_metrics(iii, &cell, &http);
    register_code_scan_remediate(iii, &cell, &http);
}

// devin::run — drive the local devin CLI and wait for the result.
fn register_run(iii: &IIIClient, cell: &ConfigCell) {
    let iii_h = iii.clone();
    let cell_h = cell.clone();
    iii.register_function(
        "devin::run",
        RegisterFunction::new_async(move |req: RunRequest| {
            let iii_h = iii_h.clone();
            let cell_h = cell_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                Ok::<Value, Error>(cli::run(iii_h, cfg, req).await)
            }
        })
        .request_format(schema_value::<RunRequest>())
        .response_format(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "devin_session_id": { "type": ["string", "null"] },
                "result": { "type": "string" },
                "stop_reason": { "type": "string" },
                "is_error": { "type": "boolean" },
                "num_turns": { "type": "integer" },
                "busy": { "type": "boolean" },
                "reason": { "type": "string" },
            },
        }))
        .description(
            "Run one Devin CLI turn and wait for the result. Accepts `prompt` or a `messages` \
             array; streams raw CLI stdout onto devin::events, terminal AgentEvent frames onto \
             agent::events, and returns {session_id, result, stop_reason, is_error}.",
        ),
    );
}

// devin::stop — interrupt a live CLI run.
fn register_stop(iii: &IIIClient) {
    iii.register_function(
        "devin::stop",
        RegisterFunction::new_async(move |req: SessionIdRequest| async move {
            let stopped = cli::stop(&req.session_id).await;
            Ok::<Value, Error>(json!({
                "session_id": req.session_id,
                "stopped": stopped,
                "reason": if stopped { Value::Null } else { json!("no live run") },
            }))
        })
        .request_format(schema_value::<SessionIdRequest>())
        .response_format(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "stopped": { "type": "boolean" },
                "reason": { "type": ["string", "null"] },
            },
        }))
        .description("Interrupt a live devin::run CLI turn for a session."),
    );
}

// devin::status — point-in-time view of a local run record.
fn register_status(iii: &IIIClient) {
    let iii_h = iii.clone();
    iii.register_function(
        "devin::status",
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let iii_h = iii_h.clone();
            async move {
                let record = load_session(&iii_h, &req.session_id).await.ok().flatten();
                let live = cli::is_live(&req.session_id).await;
                Ok::<Value, Error>(json!({
                    "session_id": req.session_id,
                    "live": live,
                    "record": record,
                }))
            }
        })
        .request_format(schema_value::<SessionIdRequest>())
        .response_format(json!({
            "type": "object",
            "properties": {
                "session_id": { "type": "string" },
                "live": { "type": "boolean" },
                "record": { "type": ["object", "null"] },
            },
        }))
        .description("Point-in-time status of a local devin::run session."),
    );
}

// devin::runs::list — every local CLI run this worker has recorded.
fn register_runs_list(iii: &IIIClient) {
    let iii_h = iii.clone();
    iii.register_function(
        "devin::runs::list",
        RegisterFunction::new_async(move |_req: Value| {
            let iii_h = iii_h.clone();
            async move {
                let sessions = list_sessions(&iii_h).await.unwrap_or_default();
                Ok::<Value, Error>(json!({ "sessions": sessions }))
            }
        })
        .request_format(json!({ "type": "object", "properties": {} }))
        .response_format(json!({
            "type": "object",
            "properties": { "sessions": { "type": "array", "items": { "type": "object" } } },
        }))
        .description("List every local devin::run session this worker has recorded."),
    );
}

// devin::api — raw pass-through to any Devin v3 endpoint.
fn register_api(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::api",
        RegisterFunction::new_async(move |req: ApiRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let out = api::request(
                    &http_h,
                    &cfg,
                    &req.method,
                    &req.path,
                    req.query.as_ref(),
                    req.body.as_ref(),
                )
                .await;
                Ok::<Value, Error>(match out {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<ApiRequest>())
        .response_format(passthrough_schema())
        .description(
            "Raw authenticated call to any Devin v3 endpoint: {method, path, query?, body?}. \
             Use for anything the typed devin::session::* wrappers do not cover.",
        ),
    );
}

// devin::session::create — start a Devin cloud session.
fn register_session_create(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::session::create",
        RegisterFunction::new_async(move |req: SessionCreateRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let prompt = match extract_create_prompt(&req) {
                    Ok(p) => p,
                    Err(e) => return Ok::<Value, Error>(err_value(e)),
                };
                let body = req.to_body(&prompt);
                Ok::<Value, Error>(match api::create_session(&http_h, &cfg, &body).await {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<SessionCreateRequest>())
        .response_format(passthrough_schema())
        .description(
            "Create a Devin cloud session from a prompt. Optional fields (title, tags, \
             playbook_id, snapshot_id, idempotent, max_acu_limit, secret_ids, knowledge_ids, \
             unlisted) map to POST /sessions.",
        ),
    );
}

// devin::session::get — fetch one Devin cloud session by id.
fn register_session_get(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::session::get",
        RegisterFunction::new_async(move |req: SessionIdRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                Ok::<Value, Error>(
                    match api::get_session(&http_h, &cfg, &req.session_id).await {
                        Ok(v) => v,
                        Err(e) => err_value(e),
                    },
                )
            }
        })
        .request_format(schema_value::<SessionIdRequest>())
        .response_format(passthrough_schema())
        .description("Fetch one Devin cloud session (status, messages, output) by session id."),
    );
}

// devin::session::list — list Devin cloud sessions.
fn register_session_list(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::session::list",
        RegisterFunction::new_async(move |req: SessionListRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let query = req.to_query();
                Ok::<Value, Error>(match api::list_sessions(&http_h, &cfg, &query).await {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<SessionListRequest>())
        .response_format(passthrough_schema())
        .description("List Devin cloud sessions with optional limit, offset, and tag filters."),
    );
}

// devin::session::message — send a follow-up to a running Devin session.
fn register_session_message(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::session::message",
        RegisterFunction::new_async(move |req: SessionMessageRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let body = json!({ "message": req.message });
                Ok::<Value, Error>(
                    match api::send_message(&http_h, &cfg, &req.session_id, &body).await {
                        Ok(v) => v,
                        Err(e) => err_value(e),
                    },
                )
            }
        })
        .request_format(schema_value::<SessionMessageRequest>())
        .response_format(passthrough_schema())
        .description("Send a follow-up message to a running Devin cloud session."),
    );
}

// devin::pr-review::trigger — start a Devin review for a pull/merge request.
fn register_pr_review_trigger(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::pr-review::trigger",
        RegisterFunction::new_async(move |req: PrReviewTriggerRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let body = json!({ "pr_url": req.pr_url });
                Ok::<Value, Error>(match api::pr_review_trigger(&http_h, &cfg, &body).await {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<PrReviewTriggerRequest>())
        .response_format(passthrough_schema())
        .description("Trigger a Devin review for a pull/merge request by its URL."),
    );
}

// devin::pr-review::status — latest Devin review for a pull/merge request.
fn register_pr_review_status(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::pr-review::status",
        RegisterFunction::new_async(move |req: PrReviewStatusRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let query = req.to_query();
                Ok::<Value, Error>(match api::pr_review_status(&http_h, &cfg, &query).await {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<PrReviewStatusRequest>())
        .response_format(passthrough_schema())
        .description("Get the latest Devin review status for a pull/merge request."),
    );
}

// devin::code-scan::findings — list enterprise code scan findings.
fn register_code_scan_findings(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::code-scan::findings",
        RegisterFunction::new_async(move |req: CodeScanFindingsRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let query = req.to_query();
                Ok::<Value, Error>(match api::code_scan_findings(&http_h, &cfg, &query).await {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<CodeScanFindingsRequest>())
        .response_format(passthrough_schema())
        .description(
            "List enterprise code scan findings (enterprise-gated). Filter by severity, status, \
             scan_id, repo_name, org_ids; paginate with after/first.",
        ),
    );
}

// devin::code-scan::metrics — enterprise code scan metrics.
fn register_code_scan_metrics(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::code-scan::metrics",
        RegisterFunction::new_async(move |req: CodeScanMetricsRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                let query = req.to_query();
                Ok::<Value, Error>(match api::code_scan_metrics(&http_h, &cfg, &query).await {
                    Ok(v) => v,
                    Err(e) => err_value(e),
                })
            }
        })
        .request_format(schema_value::<CodeScanMetricsRequest>())
        .response_format(passthrough_schema())
        .description("Get metrics for enterprise code scans (enterprise-gated)."),
    );
}

// devin::code-scan::remediate — launch a session to fix a finding.
fn register_code_scan_remediate(iii: &IIIClient, cell: &ConfigCell, http: &Client) {
    let cell_h = cell.clone();
    let http_h = http.clone();
    iii.register_function(
        "devin::code-scan::remediate",
        RegisterFunction::new_async(move |req: CodeScanRemediateRequest| {
            let cell_h = cell_h.clone();
            let http_h = http_h.clone();
            async move {
                let cfg = { cell_h.read().await.clone() };
                Ok::<Value, Error>(
                    match api::code_scan_remediate(&http_h, &cfg, &req.scan_id, &req.finding_id)
                        .await
                    {
                        Ok(v) => v,
                        Err(e) => err_value(e),
                    },
                )
            }
        })
        .request_format(schema_value::<CodeScanRemediateRequest>())
        .response_format(passthrough_schema())
        .description(
            "Launch a Devin session to remediate a code scan finding (enterprise-gated); \
             opens a PR with the fix.",
        ),
    );
}
