use super::*;

#[async_trait]
impl ExecutionRuntime for IiiRuntime {
    async fn get_run_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<RunRecordV1>, SecurityScanError> {
        let mut matches = self
            .list_index_records()
            .await?
            .into_iter()
            .filter(|record| record.harness_session_id.as_deref() == Some(session_id));
        let found = matches.next();
        if matches.next().is_some() {
            return Err(SecurityScanError::Dependency(format!(
                "multiple runs reference Harness session {session_id}"
            )));
        }
        let Some(found) = found else {
            return Ok(None);
        };
        let run = self.get_run(&found.summary.run_id).await?;
        Ok(run.filter(|run| {
            run.harness
                .as_ref()
                .is_some_and(|harness| harness.session_id == session_id)
        }))
    }

    async fn materialize_target(
        &self,
        repository: &RepositoryConfigV1,
        request: &MaterializationRequest,
    ) -> Result<MaterializedTargetV1, SecurityScanError> {
        let session_id = &request.session_id;
        let existing = self
            .call(
                "worktree::list",
                json!({
                    "repo_path": repository.path,
                    "session_id": session_id,
                    "include_status": false,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        let mut worktrees = serde_json::from_value::<WorktreeListWire>(existing)
            .map_err(|error| dependency_parse("worktree::list", error))?
            .worktrees;
        if worktrees.len() > 1 {
            return Err(SecurityScanError::Dependency(format!(
                "worktree::list returned multiple checkouts for {session_id}"
            )));
        }
        if let Some(worktree) = worktrees.pop() {
            match worktree.lifecycle.as_str() {
                "orphaned" => {
                    let removed = self
                        .call(
                            "worktree::remove",
                            json!({
                                "worktree_id": worktree.worktree_id,
                                "force": false,
                                "delete_branch": true,
                            }),
                            None,
                            Some(RPC_TIMEOUT_MS),
                        )
                        .await?;
                    if removed.get("removed").and_then(Value::as_bool) != Some(true) {
                        return Err(SecurityScanError::Dependency(
                            "worktree::remove did not clear an orphaned scanner checkout".into(),
                        ));
                    }
                }
                "active" | "claimed" => {
                    return materialized_from_existing(worktree, repository, &request.target_sha)
                }
                lifecycle => {
                    return Err(SecurityScanError::Dependency(format!(
                        "scanner checkout {} has unexpected lifecycle {lifecycle}",
                        worktree.worktree_id
                    )))
                }
            }
        }

        let created = self
            .call(
                "worktree::create",
                json!({
                    "repo_path": repository.path,
                    "base_ref": request.target_sha,
                    "session_id": session_id,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        let worktree: WorktreeCreateWire = serde_json::from_value(created)
            .map_err(|error| dependency_parse("worktree::create", error))?;
        materialized_from_created(worktree, &request.target_sha)
    }

    async fn cleanup_target(&self, target: &MaterializedTargetV1) -> Result<(), SecurityScanError> {
        let response = match self
            .call(
                "worktree::remove",
                json!({
                    "worktree_id": target.worktree_id,
                    "force": false,
                    "delete_branch": true,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await
        {
            Ok(response) => response,
            Err(error) if worktree_is_missing(&error) => return Ok(()),
            Err(error) => return Err(error),
        };
        if response.get("removed").and_then(Value::as_bool) != Some(true) {
            return Err(SecurityScanError::Dependency(format!(
                "worktree::remove did not remove scanner checkout {}",
                target.worktree_id
            )));
        }
        if response.get("branch_deleted").and_then(Value::as_bool) != Some(true) {
            tracing::warn!(
                worktree_id = %target.worktree_id,
                "scanner checkout was removed but its branch was not deleted"
            );
        }
        Ok(())
    }

    async fn start_analysis(
        &self,
        plan: AnalysisPlan,
    ) -> Result<AnalysisHandle, SecurityScanError> {
        let existing = self
            .call(
                "harness::status",
                json!({ "session_id": plan.session_id }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        if !existing.is_null() {
            let status: HarnessStatusWire = serde_json::from_value(existing)
                .map_err(|error| dependency_parse("harness::status", error))?;
            if let Some(turn_id) = status.turn_id {
                self.jail_unattended_session(&plan).await;
                return Ok(AnalysisHandle {
                    session_id: plan.session_id,
                    turn_id,
                });
            }
        }
        // Jail before send so the first coder::* read is not held on the
        // Console approval gate while set-mode is still in flight.
        self.jail_unattended_session(&plan).await;
        let request = harness_request(&plan);
        let response = self
            .call("harness::send", request, None, Some(RPC_TIMEOUT_MS))
            .await?;
        let response: HarnessSendWire = serde_json::from_value(response)
            .map_err(|error| dependency_parse("harness::send", error))?;
        if !response.accepted {
            return Err(SecurityScanError::Dependency(
                "harness::send did not accept the analysis turn".into(),
            ));
        }
        Ok(AnalysisHandle {
            session_id: response.session_id,
            turn_id: response.turn_id,
        })
    }

    async fn completed_analysis(
        &self,
        run: &RunRecordV1,
    ) -> Result<Option<crate::TurnCompletedEventV1>, SecurityScanError> {
        let harness = run.harness.as_ref().ok_or_else(|| {
            SecurityScanError::Dependency(format!(
                "analyzing run {} has no Harness checkpoint",
                run.run_id
            ))
        })?;
        self.completed_session(harness).await
    }
}

#[async_trait]
impl crate::action_executor::ActionRuntime for IiiRuntime {
    async fn materialize_action_target(
        &self,
        repository: &RepositoryConfigV1,
        action: &crate::SecurityActionRecordV1,
    ) -> Result<MaterializedTargetV1, SecurityScanError> {
        self.materialize_target(repository, &MaterializationRequest::for_action(action))
            .await
    }

    async fn completed_action(
        &self,
        action: &crate::SecurityActionRecordV1,
    ) -> Result<Option<crate::TurnCompletedEventV1>, SecurityScanError> {
        let harness = action.harness.as_ref().ok_or_else(|| {
            SecurityScanError::Dependency(format!(
                "action {} has no Harness checkpoint",
                action.action_id
            ))
        })?;
        self.completed_session(harness).await
    }

    async fn get_action_by_session(
        &self,
        session_id: &str,
    ) -> Result<Option<crate::SecurityActionRecordV1>, SecurityScanError> {
        let value = self
            .call_private(
                STATE_GET_ID,
                json!({ "scope": ACTION_SESSION_SCOPE, "key": session_id }),
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        let record: ActionSessionIndexRecordV1 =
            serde_json::from_value(value).map_err(|error| {
                SecurityScanError::Dependency(format!(
                    "could not parse action session index {session_id}: {error}"
                ))
            })?;
        let action = self.get_action(&record.action_id).await?;
        Ok(action.filter(|action| {
            action
                .harness
                .as_ref()
                .is_some_and(|harness| harness.session_id == session_id)
        }))
    }
}
