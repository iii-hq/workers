use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterFunction};
use schemars::{schema::RootSchema, JsonSchema};
use serde_json::json;

use crate::{
    EnqueueRequest, ExecuteResponseV1, IiiRuntime, SecurityScanExecutor, SecurityScanListRequestV1,
    SecurityScanListResponseV1, SecurityScanReadRequestV1, SecurityScanReadResponseV1,
    SecurityScanReconciliationRequestV1, SecurityScanReconciliationResponseV1,
    SecurityScanRequestV1, SecurityScanResponseV1, SecurityScanScheduleEventV1,
    SecurityScanScheduleResponseV1, SecurityScanService, TurnCompletedEventV1,
    TurnCompletedResponseV1,
};

pub const REQUEST_ID: &str = "security-scan::request";
pub const REQUEST_DESC: &str = "Queue a report-only security review for an operator-configured repository at an exact 40-character Git commit SHA. Duplicate repository, commit, and mode requests return the same run id.";
pub const LIST_ID: &str = "security-scan::list";
pub const LIST_DESC: &str = "List security-scan runs as sanitized lightweight summaries, newest update first. Optional repository and status filters are applied before the bounded result limit.";
pub const READ_ID: &str = "security-scan::read";
pub const READ_DESC: &str = "Read a security-scan run and its validated report without exposing internal checkout paths or Harness session identifiers.";
pub const RECONCILIATION_ID: &str = "security-scan::reconciliation";
pub const RECONCILIATION_DESC: &str = "Read or refresh a persisted, sanitized comparison of one Harness report with separately counted Dependabot and code-scanning snapshots. Supports bounded source, severity, lifecycle, and cursor filters; never reports a combined unique total.";
pub const EXECUTE_ID: &str = "security-scan::execute";
pub const EXECUTE_DESC: &str =
    "Internal durable queue step for target materialization and read-only Harness dispatch.";
pub const TURN_COMPLETED_ID: &str = "security-scan::on-turn-completed";
pub const TURN_COMPLETED_DESC: &str =
    "Internal Harness completion doorbell that validates and checkpoints a structured report.";
pub const ON_SCHEDULE_ID: &str = "security-scan::on-schedule";
pub const ON_SCHEDULE_DESC: &str = "Internal UTC cron target that uses invocation metadata only to look up an operator-configured repository schedule, resolves its local Git ref at fire time, and queues the exact commit through security-scan::request.";

pub struct Deps {
    pub service: Arc<SecurityScanService<IiiRuntime>>,
    pub executor: Arc<SecurityScanExecutor<IiiRuntime>>,
}

pub fn register_all(iii: &IIIClient, deps: &Arc<Deps>) {
    let current = deps.service.clone();
    iii.register_function(
        REQUEST_ID,
        RegisterFunction::new_async(move |request: SecurityScanRequestV1| {
            let service = current.clone();
            async move { service.request(request).await.map_err(Into::into) }
        })
        .description(REQUEST_DESC),
    );

    let current = deps.service.clone();
    iii.register_function(
        LIST_ID,
        RegisterFunction::new_async(move |request: SecurityScanListRequestV1| {
            let service = current.clone();
            async move { service.list(request).await.map_err(Into::into) }
        })
        .description(LIST_DESC),
    );

    let current = deps.service.clone();
    iii.register_function(
        RECONCILIATION_ID,
        RegisterFunction::new_async(move |request: SecurityScanReconciliationRequestV1| {
            let service = current.clone();
            async move { service.reconciliation(request).await.map_err(Into::into) }
        })
        .description(RECONCILIATION_DESC),
    );

    let current = deps.service.clone();
    iii.register_function(
        READ_ID,
        RegisterFunction::new_async(move |request: SecurityScanReadRequestV1| {
            let service = current.clone();
            async move { service.read(request).await.map_err(Into::into) }
        })
        .description(READ_DESC),
    );

    let current = deps.executor.clone();
    iii.register_function(
        EXECUTE_ID,
        RegisterFunction::new_async(move |request: EnqueueRequest| {
            let executor = current.clone();
            async move { executor.execute(request).await.map_err(Into::into) }
        })
        .description(EXECUTE_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.executor.clone();
    iii.register_function(
        TURN_COMPLETED_ID,
        RegisterFunction::new_async(move |event: TurnCompletedEventV1| {
            let executor = current.clone();
            async move { executor.on_turn_completed(event).await.map_err(Into::into) }
        })
        .description(TURN_COMPLETED_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );
}

pub struct FunctionSpec {
    pub function_id: &'static str,
    pub description: &'static str,
    pub request_schema: RootSchema,
    pub response_schema: RootSchema,
}

fn schema_of<T: JsonSchema>() -> RootSchema {
    schemars::r#gen::SchemaSettings::draft07()
        .into_generator()
        .into_root_schema_for::<T>()
}

fn spec<Req: JsonSchema, Resp: JsonSchema>(
    function_id: &'static str,
    description: &'static str,
) -> FunctionSpec {
    FunctionSpec {
        function_id,
        description,
        request_schema: schema_of::<Req>(),
        response_schema: schema_of::<Resp>(),
    }
}

pub fn catalog() -> Vec<FunctionSpec> {
    vec![
        spec::<SecurityScanRequestV1, SecurityScanResponseV1>(REQUEST_ID, REQUEST_DESC),
        spec::<SecurityScanListRequestV1, SecurityScanListResponseV1>(LIST_ID, LIST_DESC),
        spec::<SecurityScanReconciliationRequestV1, SecurityScanReconciliationResponseV1>(
            RECONCILIATION_ID,
            RECONCILIATION_DESC,
        ),
        spec::<SecurityScanReadRequestV1, SecurityScanReadResponseV1>(READ_ID, READ_DESC),
        spec::<EnqueueRequest, ExecuteResponseV1>(EXECUTE_ID, EXECUTE_DESC),
        spec::<TurnCompletedEventV1, TurnCompletedResponseV1>(
            TURN_COMPLETED_ID,
            TURN_COMPLETED_DESC,
        ),
        spec::<SecurityScanScheduleEventV1, SecurityScanScheduleResponseV1>(
            ON_SCHEDULE_ID,
            ON_SCHEDULE_DESC,
        ),
    ]
}
