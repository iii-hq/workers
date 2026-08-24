use std::sync::Arc;

use async_trait::async_trait;

use crate::action::{build_fix_plan, build_issue_plan, result_from_output, ActionHarnessOutputV1};
use crate::{
    ids, ActionEnqueueRequestV1, ActionExecuteResponseV1, AnalysisHandle, AnalysisPlan,
    ExecutionRuntime, MaterializedTargetV1, RepositoryConfigV1, RunRecordV1, RunStatusV1,
    ScanModeV1, SecurityActionKindV1, SecurityActionRecordV1, SecurityActionStatusV1,
    SecurityFindingV1, SecurityRuntime, SecurityScanError, TurnCompletedEventV1,
    TurnCompletedResponseV1, WorkerConfig,
};

const MAX_STEP_FAILURES: u32 = 3;

#[async_trait]
pub trait ActionRuntime: SecurityRuntime + ExecutionRuntime {
    async fn materialize_action_target(
        &self,
        repository: &RepositoryConfigV1,
        action: &SecurityActionRecordV1,
    ) -> Result<MaterializedTargetV1, SecurityScanError>;

    async fn cleanup_action_target(
        &self,
        target: &MaterializedTargetV1,
    ) -> Result<(), SecurityScanError> {
        self.cleanup_target(target).await
    }

    async fn start_action_session(
        &self,
        plan: AnalysisPlan,
    ) -> Result<AnalysisHandle, SecurityScanError> {
        self.start_analysis(plan).await
    }

    async fn completed_action(
        &self,
        action: &SecurityActionRecordV1,
    ) -> Result<Option<TurnCompletedEventV1>, SecurityScanError>;

    async fn get_action_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<SecurityActionRecordV1>, SecurityScanError>;
}

pub struct SecurityActionExecutor<R> {
    runtime: Arc<R>,
    config: WorkerConfig,
}

impl<R> SecurityActionExecutor<R>
where
    R: ActionRuntime,
{
    pub fn new(runtime: Arc<R>, config: WorkerConfig) -> Self {
        Self { runtime, config }
    }

    pub async fn recover_actions(&self) -> Result<(), SecurityScanError> {
        let actions = self.runtime.list_actions().await?;
        for action in actions {
            if action.status.is_terminal() {
                if action.cleanup_completed_at.is_none() {
                    if let Err(error) = self.cleanup(&action).await {
                        tracing::warn!(
                            action_id = %action.action_id,
                            %error,
                            "action checkout cleanup recovery failed"
                        );
                    }
                }
                continue;
            }
            if let Err(error) = self.enqueue(&action).await {
                tracing::warn!(
                    action_id = %action.action_id,
                    %error,
                    "could not re-enqueue security scan action"
                );
            }
        }
        Ok(())
    }

    pub async fn execute(
        &self,
        request: ActionEnqueueRequestV1,
    ) -> Result<ActionExecuteResponseV1, SecurityScanError> {
        match self.execute_inner(&request).await {
            Ok(response) => Ok(response),
            Err(error) => match self.record_step_failure(&request, &error).await? {
                Some(response) => Ok(response),
                None => Err(error),
            },
        }
    }

    pub async fn on_turn_completed(
        &self,
        event: TurnCompletedEventV1,
    ) -> Result<TurnCompletedResponseV1, SecurityScanError> {
        if !event.terminal {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: None,
            });
        }
        let Some(action) = self
            .runtime
            .get_action_by_session(&event.session_id)
            .await?
        else {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: None,
            });
        };
        if action.status != SecurityActionStatusV1::AwaitingApproval
            || action
                .harness
                .as_ref()
                .is_none_or(|harness| harness.turn_id != event.turn_id)
        {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: None,
            });
        }
        let Some(authoritative) = self.runtime.completed_action(&action).await? else {
            return Ok(TurnCompletedResponseV1 {
                woke: false,
                status: None,
            });
        };
        self.finish_action(action, authoritative).await?;
        Ok(TurnCompletedResponseV1 {
            woke: true,
            status: None,
        })
    }

    async fn execute_inner(
        &self,
        request: &ActionEnqueueRequestV1,
    ) -> Result<ActionExecuteResponseV1, SecurityScanError> {
        let Some(action) = self.runtime.get_action(&request.action_id).await? else {
            return Err(SecurityScanError::InvalidRequest(format!(
                "unknown action {}",
                request.action_id
            )));
        };
        if action.run_id != request.run_id
            || action.attempt != request.attempt
            || request.step > action.step
        {
            return Ok(action_response(&action, true));
        }
        if action.status.is_terminal() {
            if action.cleanup_completed_at.is_none() {
                let _ = self.cleanup(&action).await;
            }
            return Ok(action_response(&action, true));
        }
        if action
            .result
            .as_ref()
            .is_some_and(|result| !result.url.is_empty())
        {
            return self.complete_existing_publication(action).await;
        }
        if !self.runtime.approval_gate_is_live().await? {
            return self
                .fail_closed(
                    action,
                    "approval::gate is not live; GitHub mutations stay closed",
                )
                .await;
        }
        let action = self.prepare(action).await?;
        self.dispatch(action).await
    }

    async fn prepare(
        &self,
        mut action: SecurityActionRecordV1,
    ) -> Result<SecurityActionRecordV1, SecurityScanError> {
        if action.action == SecurityActionKindV1::Issue || action.materialized.is_some() {
            return Ok(action);
        }
        let repository = self.repository(&action.repository)?;
        let expected = action.clone();
        action.status = SecurityActionStatusV1::Preparing;
        action.updated_at = ids::now_ms();
        if !self
            .runtime
            .replace_action(&expected, action.clone())
            .await?
        {
            return self.current_or(expected.action_id).await;
        }
        let target = self
            .runtime
            .materialize_action_target(repository, &action)
            .await?;
        let expected = action.clone();
        action.materialized = Some(target);
        action.updated_at = ids::now_ms();
        if !self
            .runtime
            .replace_action(&expected, action.clone())
            .await?
        {
            return self.current_or(expected.action_id).await;
        }
        Ok(action)
    }

    async fn dispatch(
        &self,
        action: SecurityActionRecordV1,
    ) -> Result<ActionExecuteResponseV1, SecurityScanError> {
        if let Some(session_id) = action
            .harness
            .as_ref()
            .map(|harness| harness.session_id.clone())
        {
            if let Some(completed) = self.runtime.completed_action(&action).await? {
                self.finish_action(action, completed).await?;
                return self.current_response(&session_id).await;
            }
            return Ok(action_response(&action, false));
        }
        let run = self.run_for_action(&action).await?;
        let finding = finding_from_run(&run, action.finding_index)?;
        let plan = match action.action {
            SecurityActionKindV1::Issue => {
                build_issue_plan(&action, finding, &self.config.analysis)
            }
            SecurityActionKindV1::FixPr => {
                let path = action
                    .materialized
                    .as_ref()
                    .map(|target| target.path.as_str())
                    .ok_or_else(|| {
                        SecurityScanError::Dependency(
                            "fix PR action is missing an isolated checkout".into(),
                        )
                    })?;
                build_fix_plan(&action, finding, path, &self.config.analysis)
            }
        };
        let handle = self.runtime.start_action_session(plan).await?;
        let expected = action.clone();
        let mut next = action;
        next.status = SecurityActionStatusV1::AwaitingApproval;
        next.harness = Some(crate::HarnessRunV1 {
            session_id: handle.session_id,
            turn_id: handle.turn_id,
        });
        next.updated_at = ids::now_ms();
        if !self.runtime.replace_action(&expected, next.clone()).await? {
            return Ok(action_response(&expected, true));
        }
        if let Some(completed) = self.runtime.completed_action(&next).await? {
            self.finish_action(next.clone(), completed).await?;
            let current = self
                .runtime
                .get_action(&next.action_id)
                .await?
                .ok_or_else(|| {
                    SecurityScanError::Dependency(format!(
                        "action {} disappeared after completion",
                        next.action_id
                    ))
                })?;
            return Ok(action_response(&current, false));
        }
        Ok(action_response(&next, false))
    }

    async fn finish_action(
        &self,
        action: SecurityActionRecordV1,
        event: TurnCompletedEventV1,
    ) -> Result<(), SecurityScanError> {
        if action
            .result
            .as_ref()
            .is_some_and(|result| !result.url.is_empty())
        {
            let _ = self.cleanup(&action).await;
            return Ok(());
        }
        let now = ids::now_ms();
        let mut finished = action.clone();
        finished.updated_at = now;
        finished.completed_at = Some(now);
        if event.status == "completed" {
            match event
                .result
                .ok_or_else(|| "Harness completed without a result".to_string())
                .and_then(|value| {
                    serde_json::from_value::<ActionHarnessOutputV1>(value)
                        .map_err(|error| format!("invalid action result: {error}"))
                })
                .and_then(|output| {
                    result_from_output(action.action, &action.github_full_name, output)
                        .map_err(|error| error.to_string())
                }) {
                Ok(result) => {
                    finished.status = SecurityActionStatusV1::Completed;
                    finished.result = Some(result);
                    finished.error = None;
                }
                Err(message) => {
                    finished.status = SecurityActionStatusV1::Failed;
                    finished.error = Some(crate::RunErrorV1 {
                        code: "action_failed".into(),
                        message,
                        retryable: false,
                    });
                }
            }
        } else {
            finished.status = SecurityActionStatusV1::Failed;
            finished.error = Some(crate::RunErrorV1 {
                code: "action_failed".into(),
                message: event
                    .result_error
                    .unwrap_or_else(|| "Harness action session failed".into()),
                retryable: false,
            });
        }
        if self
            .runtime
            .replace_action(&action, finished.clone())
            .await?
        {
            let _ = self.cleanup(&finished).await;
        }
        Ok(())
    }

    async fn complete_existing_publication(
        &self,
        action: SecurityActionRecordV1,
    ) -> Result<ActionExecuteResponseV1, SecurityScanError> {
        if action.status == SecurityActionStatusV1::Completed {
            let _ = self.cleanup(&action).await;
            return Ok(action_response(&action, true));
        }
        let expected = action.clone();
        let mut completed = action;
        completed.status = SecurityActionStatusV1::Completed;
        completed.updated_at = ids::now_ms();
        completed.completed_at = Some(completed.updated_at);
        completed.error = None;
        if self
            .runtime
            .replace_action(&expected, completed.clone())
            .await?
        {
            let _ = self.cleanup(&completed).await;
        }
        Ok(action_response(&completed, true))
    }

    async fn fail_closed(
        &self,
        action: SecurityActionRecordV1,
        message: &str,
    ) -> Result<ActionExecuteResponseV1, SecurityScanError> {
        let expected = action.clone();
        let mut failed = action;
        failed.status = SecurityActionStatusV1::Failed;
        failed.updated_at = ids::now_ms();
        failed.completed_at = Some(failed.updated_at);
        failed.error = Some(crate::RunErrorV1 {
            code: "approval_unavailable".into(),
            message: message.to_string(),
            retryable: true,
        });
        if self
            .runtime
            .replace_action(&expected, failed.clone())
            .await?
        {
            let _ = self.cleanup(&failed).await;
        }
        Ok(action_response(&failed, false))
    }

    async fn record_step_failure(
        &self,
        request: &ActionEnqueueRequestV1,
        error: &SecurityScanError,
    ) -> Result<Option<ActionExecuteResponseV1>, SecurityScanError> {
        let Some(action) = self.runtime.get_action(&request.action_id).await? else {
            return Ok(None);
        };
        if action.run_id != request.run_id
            || action.attempt != request.attempt
            || request.step > action.step
            || action.status.is_terminal()
        {
            return Ok(Some(action_response(&action, true)));
        }
        let mut failed = action.clone();
        failed.step_failures = failed.step_failures.saturating_add(1);
        failed.updated_at = ids::now_ms();
        let terminal = matches!(error, SecurityScanError::InvalidRequest(_))
            || failed.step_failures >= MAX_STEP_FAILURES;
        if terminal {
            failed.status = SecurityActionStatusV1::Failed;
            failed.completed_at = Some(failed.updated_at);
        }
        failed.error = Some(crate::RunErrorV1 {
            code: if terminal {
                "step_failed".into()
            } else {
                "step_retrying".into()
            },
            message: "action step failed; dependency details are available in worker logs".into(),
            retryable: !matches!(error, SecurityScanError::InvalidRequest(_)),
        });
        if !self.runtime.replace_action(&action, failed.clone()).await? {
            return Ok(None);
        }
        if terminal {
            let _ = self.cleanup(&failed).await;
        }
        Ok(Some(action_response(&failed, !terminal)))
    }

    async fn enqueue(&self, action: &SecurityActionRecordV1) -> Result<(), SecurityScanError> {
        self.runtime
            .enqueue_action_execute(ActionEnqueueRequestV1::new(
                action.action_id.clone(),
                action.run_id.clone(),
                action.attempt,
                action.step,
            ))
            .await
    }

    async fn cleanup(&self, action: &SecurityActionRecordV1) -> Result<(), SecurityScanError> {
        if action.cleanup_completed_at.is_some() {
            return Ok(());
        }
        if let Some(target) = action.materialized.as_ref() {
            self.runtime.cleanup_action_target(target).await?;
        }
        let mut cleaned = action.clone();
        cleaned.cleanup_completed_at = Some(ids::now_ms());
        cleaned.updated_at = cleaned.cleanup_completed_at.unwrap_or(cleaned.updated_at);
        if self.runtime.replace_action(action, cleaned).await? {
            Ok(())
        } else {
            Err(SecurityScanError::Dependency(format!(
                "action {} changed while recording cleanup completion",
                action.action_id
            )))
        }
    }

    async fn run_for_action(
        &self,
        action: &SecurityActionRecordV1,
    ) -> Result<RunRecordV1, SecurityScanError> {
        self.runtime.get_run(&action.run_id).await?.ok_or_else(|| {
            SecurityScanError::InvalidRequest(format!("unknown run {}", action.run_id))
        })
    }

    fn repository(&self, id: &str) -> Result<&RepositoryConfigV1, SecurityScanError> {
        self.config.repository(id).ok_or_else(|| {
            SecurityScanError::InvalidRequest(format!("repository {id} is not configured"))
        })
    }

    async fn current_or(
        &self,
        action_id: String,
    ) -> Result<SecurityActionRecordV1, SecurityScanError> {
        self.runtime
            .get_action(&action_id)
            .await?
            .ok_or_else(|| SecurityScanError::Dependency(format!("action {action_id} disappeared")))
    }

    async fn current_response(
        &self,
        session_id: &str,
    ) -> Result<ActionExecuteResponseV1, SecurityScanError> {
        let action = self
            .runtime
            .get_action_by_session(session_id)
            .await?
            .ok_or_else(|| {
                SecurityScanError::Dependency(format!(
                    "action for session {session_id} disappeared"
                ))
            })?;
        Ok(action_response(&action, action.status.is_terminal()))
    }
}

fn finding_from_run(
    run: &RunRecordV1,
    finding_index: u32,
) -> Result<&SecurityFindingV1, SecurityScanError> {
    let findings = run
        .report
        .as_ref()
        .map(|report| report.findings.as_slice())
        .unwrap_or(&[]);
    findings
        .get(finding_index as usize)
        .ok_or_else(|| SecurityScanError::InvalidRequest("finding_index is out of range".into()))
}

fn action_response(action: &SecurityActionRecordV1, skipped: bool) -> ActionExecuteResponseV1 {
    ActionExecuteResponseV1 {
        skipped,
        status: action.status,
        step: action.step,
    }
}

pub fn validate_action_request(
    run: &RunRecordV1,
    finding_index: u32,
    action: SecurityActionKindV1,
) -> Result<(), SecurityScanError> {
    if run.status != RunStatusV1::Completed {
        return Err(SecurityScanError::InvalidRequest(
            "actions require a completed Harness report".into(),
        ));
    }
    let finding = finding_from_run(run, finding_index)?;
    match action {
        SecurityActionKindV1::Issue => Ok(()),
        SecurityActionKindV1::FixPr => {
            if run.mode != ScanModeV1::Suggest {
                return Err(SecurityScanError::InvalidRequest(
                    "fix PRs require a completed suggest run".into(),
                ));
            }
            if finding
                .suggested_patch
                .as_deref()
                .is_none_or(|patch| patch.trim().is_empty())
            {
                return Err(SecurityScanError::InvalidRequest(
                    "fix PRs require a suggested patch on that finding".into(),
                ));
            }
            Ok(())
        }
    }
}
