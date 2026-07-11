//! `harness::function::trigger` — invoke a single iii function and return a
//! normalised result (harness.md § `harness::function::trigger`). The loop
//! unwraps `agent_trigger` before calling this; it can also be called directly
//! with an already-unwrapped target.

use std::time::Instant;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::hooks::runner::PreTriggerOutcome;
use crate::policy::CompiledPolicy;
use crate::trigger;
use crate::types::content::ContentBlock;

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct TriggerCall {
    /// function_call id, echoed into the result.
    pub id: String,
    /// The iii function to invoke (already unwrapped from `agent_trigger`).
    pub function_id: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub struct FunctionTriggerRequest {
    pub session_id: String,
    pub call: TriggerCall,
}

impl Default for TriggerCall {
    fn default() -> Self {
        Self {
            id: String::new(),
            function_id: String::new(),
            arguments: Value::Null,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TriggerResultResponse {
    pub function_call_id: String,
    pub function_id: String,
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub details: Value,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TriggerPendingResponse {
    pub function_call_id: String,
    pub function_id: String,
    pub pending: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pending_timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum FunctionTriggerResponse {
    Result(TriggerResultResponse),
    Pending(TriggerPendingResponse),
}

pub async fn handle(
    deps: &Deps,
    req: FunctionTriggerRequest,
) -> Result<FunctionTriggerResponse, HarnessError> {
    let cfg = deps.cfg().await;
    // Honour the calling turn's dispatch policy when one exists; absent a turn
    // record this is a direct trusted call, which still fails closed.
    let record = crate::state::get_turn(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?;
    let policy = CompiledPolicy::from(record.as_ref().and_then(|r| r.options.functions.as_ref()));
    let engine = deps.engine().await;
    let started = Instant::now();

    // Trigger is a pipeline: fail-closed globs, then the pre_trigger chain,
    // then the target, then the post_trigger chain (harness.md §
    // `harness::function::trigger`).
    if !policy.allows(&req.call.function_id) {
        let data = trigger::denied_result(&req.call.function_id);
        return Ok(result_response(
            &req.call.id,
            &req.call.function_id,
            data,
            elapsed(started),
        ));
    }

    // Stamp the turn's filesystem scope onto scoped shell/coder args BEFORE the
    // hook chain (an approver must see the fs_scope the call will run under)
    // and re-apply it before invocation so neither a direct caller nor a hook
    // rewrite can widen the session scope. No turn record → no scope: any
    // caller-supplied fs_scope is stripped.
    let session_grants = if record.is_some() {
        crate::filesystem_grants::roots(&deps.iii, &req.session_id, cfg.session_timeout_ms).await?
    } else {
        Vec::new()
    };
    let filesystem_root = record
        .as_ref()
        .and_then(|rec| rec.options.filesystem_root())
        .map(str::to_string);
    let filesystem_boundary = deps.hooks.filesystem_boundary(&req.call.function_id);
    let mut arguments = crate::filesystem_scope::inject(
        &req.call.function_id,
        req.call.arguments.clone(),
        filesystem_root.as_deref(),
        &session_grants,
        filesystem_boundary,
    );
    if let Some(rec) = &record {
        match deps
            .hooks
            .run_pre_trigger(
                rec,
                rec.step,
                &req.call.id,
                &req.call.function_id,
                &arguments,
            )
            .await
        {
            PreTriggerOutcome::Continue { arguments: a, .. } => {
                arguments = crate::filesystem_scope::inject(
                    &req.call.function_id,
                    a,
                    filesystem_root.as_deref(),
                    &session_grants,
                    filesystem_boundary,
                );
            }
            PreTriggerOutcome::Deny(reason) => {
                let data = trigger::ResultData {
                    content: vec![ContentBlock::text(reason.clone())],
                    is_error: true,
                    details: serde_json::json!({ "error": "hook_denied", "message": reason }),
                };
                return Ok(result_response(
                    &req.call.id,
                    &req.call.function_id,
                    data,
                    elapsed(started),
                ));
            }
            PreTriggerOutcome::Hold { .. } => {
                return Ok(FunctionTriggerResponse::Pending(TriggerPendingResponse {
                    function_call_id: req.call.id,
                    function_id: req.call.function_id,
                    pending: true,
                    pending_timeout_ms: None,
                }));
            }
        }
    }

    // Re-stamp after the hook chain (idempotent) — a hook rewrite must not
    // widen or drop the session scope.
    let arguments = crate::filesystem_scope::inject(
        &req.call.function_id,
        arguments,
        filesystem_root.as_deref(),
        &session_grants,
        filesystem_boundary,
    );

    // Single invocation chokepoint: subscription control calls are intercepted
    // (trusted session injected); everything else invokes the target.
    let raw = crate::functions::subscribe::invoke(
        deps,
        &engine,
        &policy,
        &req.call.function_id,
        &arguments,
        &req.session_id,
    )
    .await;
    let data = if let Some(rec) = &record {
        match deps
            .hooks
            .run_post_trigger(
                rec,
                rec.step,
                &req.call.id,
                &req.call.function_id,
                &arguments,
                raw,
            )
            .await
        {
            crate::hooks::runner::PostTriggerOutcome::Result { result, .. } => result,
            crate::hooks::runner::PostTriggerOutcome::Hold { .. } => {
                return Ok(FunctionTriggerResponse::Pending(TriggerPendingResponse {
                    function_call_id: req.call.id,
                    function_id: req.call.function_id,
                    pending: true,
                    pending_timeout_ms: None,
                }));
            }
        }
    } else {
        raw
    };

    Ok(result_response(
        &req.call.id,
        &req.call.function_id,
        data,
        elapsed(started),
    ))
}

fn elapsed(started: Instant) -> u64 {
    started.elapsed().as_millis() as u64
}

fn result_response(
    call_id: &str,
    function_id: &str,
    data: trigger::ResultData,
    duration_ms: u64,
) -> FunctionTriggerResponse {
    FunctionTriggerResponse::Result(TriggerResultResponse {
        function_call_id: call_id.to_string(),
        function_id: function_id.to_string(),
        content: data.content,
        is_error: data.is_error,
        details: data.details,
        duration_ms,
    })
}
