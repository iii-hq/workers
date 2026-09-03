use std::sync::Arc;

use iii_sdk::{IIIClient, RegisterFunction};
use schemars::{schema::RootSchema, JsonSchema};
use serde_json::json;

use crate::action::{ACTION_COMMIT_ID, ACTION_PUSH_ID};
use crate::{
    ActionCommitRequestV1, ActionCommitResponseV1, ActionEnqueueRequestV1, ActionExecuteResponseV1,
    ActionPushRequestV1, ActionPushResponseV1, EnqueueRequest, ExecuteResponseV1, IiiRuntime,
    SecurityScanActionReadRequestV1, SecurityScanActionReadResponseV1, SecurityScanActionRequestV1,
    SecurityScanActionResponseV1, SecurityScanAnalysisChatRequestV1,
    SecurityScanAnalysisChatResponseV1, SecurityScanCancelRequestV1, SecurityScanCancelResponseV1,
    SecurityScanExecutor, SecurityScanListRequestV1, SecurityScanListResponseV1,
    SecurityScanReadRequestV1, SecurityScanReadResponseV1, SecurityScanReconciliationRequestV1,
    SecurityScanReconciliationResponseV1, SecurityScanRequestV1, SecurityScanResponseV1,
    SecurityScanScheduleEventV1, SecurityScanScheduleResponseV1, SecurityScanService,
    TurnCompletedEventV1, TurnCompletedResponseV1,
};

pub const REQUEST_ID: &str = "security-scan::request";
pub const REQUEST_DESC: &str = "Scan a repository for security issues, report-only, at one commit. Pass an exact 40-character target_sha, or omit it to review HEAD. Duplicate repository, commit, mode and model requests return the same run id.";
pub const LIST_ID: &str = "security-scan::list";
pub const LIST_DESC: &str = "List security-scan runs as sanitized lightweight summaries, newest update first. Optional repository and status filters are applied before the bounded result limit.";
pub const READ_ID: &str = "security-scan::read";
pub const READ_DESC: &str = "Read a security-scan run and its validated report without exposing internal checkout paths or Harness session identifiers.";
pub const ANALYSIS_CHAT_ID: &str = "security-scan::analysis-chat";
pub const ANALYSIS_CHAT_DESC: &str = "Make a run's Harness review discoverable through session metadata and report whether it is available, without returning the private session identifier.";
pub const CANCEL_ID: &str = "security-scan::cancel";
pub const CANCEL_DESC: &str = "Stop an in-flight security-scan run. Queued and materializing runs are marked cancelled; analyzing runs stop the Harness turn and clean up the isolated checkout.";
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
pub const ACTION_ID: &str = "security-scan::action";
pub const ACTION_DESC: &str = "Start an approval-gated GitHub issue or draft fix PR for one validated Harness finding. Duplicate run, finding, and action requests return the same action id.";
pub const ACTION_READ_ID: &str = "security-scan::action-read";
pub const ACTION_READ_DESC: &str = "Read a durable security-scan GitHub action without exposing internal checkout paths or Harness session identifiers.";
pub const ACTION_EXECUTE_ID: &str = "security-scan::action-execute";
pub const ACTION_EXECUTE_DESC: &str =
    "Internal durable queue step for approval-gated GitHub issue and draft PR publication.";

pub struct Deps {
    pub runtime: Arc<IiiRuntime>,
    pub service: Arc<SecurityScanService<IiiRuntime>>,
    pub executor: Arc<SecurityScanExecutor<IiiRuntime>>,
    pub action_executor: Arc<crate::action_executor::SecurityActionExecutor<IiiRuntime>>,
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

    let current = deps.runtime.clone();
    iii.register_function(
        ACTION_COMMIT_ID,
        RegisterFunction::new_async(move |request: ActionCommitRequestV1| {
            let runtime = current.clone();
            async move { runtime.commit_action(request).await.map_err(Into::into) }
        })
        .description("Commit the current fix action through its checkout-bound capability.")
        .metadata(json!({ "internal": true })),
    );

    let current = deps.runtime.clone();
    iii.register_function(
        ACTION_PUSH_ID,
        RegisterFunction::new_async(move |request: ActionPushRequestV1| {
            let runtime = current.clone();
            async move { runtime.push_action(request).await.map_err(Into::into) }
        })
        .description("Push the current fix action through its checkout-bound capability.")
        .metadata(json!({ "internal": true })),
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

    let current = deps.service.clone();
    iii.register_function(
        ANALYSIS_CHAT_ID,
        RegisterFunction::new_async(move |request: SecurityScanAnalysisChatRequestV1| {
            let service = current.clone();
            async move { service.analysis_chat(request).await.map_err(Into::into) }
        })
        .description(ANALYSIS_CHAT_DESC),
    );

    let current = deps.service.clone();
    iii.register_function(
        CANCEL_ID,
        RegisterFunction::new_async(move |request: SecurityScanCancelRequestV1| {
            let service = current.clone();
            async move { service.cancel(request).await.map_err(Into::into) }
        })
        .description(CANCEL_DESC),
    );

    let current = deps.service.clone();
    iii.register_function(
        ACTION_ID,
        RegisterFunction::new_async(move |request: SecurityScanActionRequestV1| {
            let service = current.clone();
            async move { service.action(request).await.map_err(Into::into) }
        })
        .description(ACTION_DESC),
    );

    let current = deps.service.clone();
    iii.register_function(
        ACTION_READ_ID,
        RegisterFunction::new_async(move |request: SecurityScanActionReadRequestV1| {
            let service = current.clone();
            async move { service.action_read(request).await.map_err(Into::into) }
        })
        .description(ACTION_READ_DESC),
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

    let scan_executor = deps.executor.clone();
    let action_executor = deps.action_executor.clone();
    iii.register_function(
        TURN_COMPLETED_ID,
        RegisterFunction::new_async(move |event: TurnCompletedEventV1| {
            let scan_executor = scan_executor.clone();
            let action_executor = action_executor.clone();
            async move {
                let scan = scan_executor
                    .on_turn_completed(event.clone())
                    .await
                    .map_err(iii_sdk::errors::Error::from)?;
                if scan.woke {
                    return Ok(scan);
                }
                action_executor
                    .on_turn_completed(event)
                    .await
                    .map_err(iii_sdk::errors::Error::from)
            }
        })
        .description(TURN_COMPLETED_DESC)
        .metadata(json!({ "internal": true, "trace_hidden": true })),
    );

    let current = deps.action_executor.clone();
    iii.register_function(
        ACTION_EXECUTE_ID,
        RegisterFunction::new_async(move |request: ActionEnqueueRequestV1| {
            let executor = current.clone();
            async move { executor.execute(request).await.map_err(Into::into) }
        })
        .description(ACTION_EXECUTE_DESC)
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
        spec::<SecurityScanAnalysisChatRequestV1, SecurityScanAnalysisChatResponseV1>(
            ANALYSIS_CHAT_ID,
            ANALYSIS_CHAT_DESC,
        ),
        spec::<SecurityScanCancelRequestV1, SecurityScanCancelResponseV1>(CANCEL_ID, CANCEL_DESC),
        spec::<SecurityScanActionRequestV1, SecurityScanActionResponseV1>(ACTION_ID, ACTION_DESC),
        spec::<SecurityScanActionReadRequestV1, SecurityScanActionReadResponseV1>(
            ACTION_READ_ID,
            ACTION_READ_DESC,
        ),
        spec::<EnqueueRequest, ExecuteResponseV1>(EXECUTE_ID, EXECUTE_DESC),
        spec::<TurnCompletedEventV1, TurnCompletedResponseV1>(
            TURN_COMPLETED_ID,
            TURN_COMPLETED_DESC,
        ),
        spec::<SecurityScanScheduleEventV1, SecurityScanScheduleResponseV1>(
            ON_SCHEDULE_ID,
            ON_SCHEDULE_DESC,
        ),
        spec::<ActionEnqueueRequestV1, ActionExecuteResponseV1>(
            ACTION_EXECUTE_ID,
            ACTION_EXECUTE_DESC,
        ),
        spec::<ActionCommitRequestV1, ActionCommitResponseV1>(
            ACTION_COMMIT_ID,
            "Commit the current fix action through its checkout-bound capability.",
        ),
        spec::<ActionPushRequestV1, ActionPushResponseV1>(
            ACTION_PUSH_ID,
            "Push the current fix action through its checkout-bound capability.",
        ),
    ]
}
