use super::*;

#[async_trait]
impl SecurityRuntime for IiiRuntime {
    fn require_ready(&self) -> Result<(), SecurityScanError> {
        if self.private_state_is_ready() {
            Ok(())
        } else {
            Err(SecurityScanError::Dependency(
                "security-scan private state is not ready".into(),
            ))
        }
    }

    async fn get_run(&self, run_id: &str) -> Result<Option<RunRecordV1>, SecurityScanError> {
        let value = self
            .call_private(STATE_GET_ID, json!({ "scope": RUN_SCOPE, "key": run_id }))
            .await?;
        parse_optional_run(value, run_id)
    }

    async fn list_run_summaries(&self) -> Result<Vec<PublicRunSummaryV1>, SecurityScanError> {
        Ok(self
            .list_index_records()
            .await?
            .into_iter()
            .map(|record| record.summary)
            .collect())
    }

    async fn get_reconciliation_snapshot(
        &self,
        run_id: &str,
    ) -> Result<Option<ReconciliationSnapshotV1>, SecurityScanError> {
        let value = self
            .call_private(
                STATE_GET_ID,
                json!({ "scope": RECONCILIATION_SCOPE, "key": run_id }),
            )
            .await?;
        if value.is_null() {
            return Ok(None);
        }
        serde_json::from_value(value).map(Some).map_err(|error| {
            SecurityScanError::Dependency(format!(
                "could not parse reconciliation snapshot {run_id}: {error}"
            ))
        })
    }

    async fn save_reconciliation_snapshot(
        &self,
        snapshot: ReconciliationSnapshotV1,
    ) -> Result<(), SecurityScanError> {
        let replacement = serialize(&snapshot, "reconciliation snapshot")?;
        for _ in 0..INDEX_REPAIR_ATTEMPTS {
            let current = self
                .call_private(
                    STATE_GET_ID,
                    json!({ "scope": RECONCILIATION_SCOPE, "key": snapshot.run_id }),
                )
                .await?;
            if !current.is_null() {
                let current_snapshot: ReconciliationSnapshotV1 =
                    serde_json::from_value(current.clone()).map_err(|error| {
                        SecurityScanError::Dependency(format!(
                            "could not parse current reconciliation snapshot {}: {error}",
                            snapshot.run_id
                        ))
                    })?;
                if snapshot_is_newer(&current_snapshot, &snapshot) {
                    return Ok(());
                }
            }
            let expected = (!current.is_null()).then_some(current);
            if matches!(
                self.compare_and_set_in_scope(
                    RECONCILIATION_SCOPE,
                    &snapshot.run_id,
                    expected,
                    Some(replacement.clone()),
                )
                .await?,
                CasOutcome::Swapped
            ) {
                self.emit_reconciliation_update(&snapshot.run_id);
                return Ok(());
            }
        }
        Err(SecurityScanError::Dependency(format!(
            "reconciliation snapshot {} changed repeatedly while saving",
            snapshot.run_id
        )))
    }

    async fn collect_reconciliation_source(
        &self,
        source: ReconciliationSourceV1,
        github_full_name: &str,
        target_sha: &str,
        collected_at: i64,
    ) -> Result<ReconciliationSourceCollectionV1, SecurityScanError> {
        match source {
            ReconciliationSourceV1::Dependabot => {
                let request = dependabot_api_request(github_full_name)?;
                let response = dependabot_api_response(
                    github_full_name,
                    self.call_typed::<_, GithubApiResponseWire>(GITHUB_API_ID, &request)
                        .await,
                );
                normalize_dependabot_response(github_full_name, collected_at, response)
            }
            ReconciliationSourceV1::CodeScanning => {
                let alerts_request = code_scanning_alerts_api_request(github_full_name)?;
                let analysis_request = code_scanning_analysis_api_request(github_full_name)?;
                let (alerts, analysis) = tokio::join!(
                    self.call_typed::<_, GithubApiResponseWire>(GITHUB_API_ID, &alerts_request),
                    self.call_typed::<_, GithubApiResponseWire>(GITHUB_API_ID, &analysis_request),
                );
                let response = code_scanning_api_response(github_full_name, alerts, analysis);
                normalize_code_scanning_response(
                    github_full_name,
                    target_sha,
                    collected_at,
                    response,
                )
            }
        }
    }

    async fn create_run_if_absent(
        &self,
        run: RunRecordV1,
    ) -> Result<CreateRunOutcome, SecurityScanError> {
        let value = serialize(&run, "run record")?;
        match self.compare_and_set(&run.run_id, None, Some(value)).await? {
            CasOutcome::Swapped => {
                self.sync_run_index_best_effort(&run.run_id).await;
                self.emit_run_update(&run);
                self.archive_run(&run).await;
                Ok(CreateRunOutcome::Created)
            }
            CasOutcome::Current(current) => {
                let existing = parse_run(current, &run.run_id)?;
                if existing.run_id != run.run_id
                    || existing.repository != run.repository
                    || existing.target_sha != run.target_sha
                    || existing.mode != run.mode
                    || existing.model != run.model
                    || existing.schema_version != run.schema_version
                {
                    return Err(SecurityScanError::Dependency(format!(
                        "state collision or corruption for run {}",
                        run.run_id
                    )));
                }
                self.sync_run_index_best_effort(&existing.run_id).await;
                Ok(CreateRunOutcome::Existing(Box::new(existing)))
            }
        }
    }

    async fn replace_run(
        &self,
        expected: &RunRecordV1,
        replacement: RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        if expected.run_id != replacement.run_id
            || expected.repository != replacement.repository
            || expected.target_sha != replacement.target_sha
            || expected.mode != replacement.mode
            || expected.model != replacement.model
        {
            return Err(SecurityScanError::Dependency(
                "run replacement changed immutable identity fields".into(),
            ));
        }
        let expected_value = serialize(expected, "expected run record")?;
        let replacement_value = serialize(&replacement, "replacement run record")?;
        let swapped = matches!(
            self.compare_and_set(
                &expected.run_id,
                Some(expected_value),
                Some(replacement_value),
            )
            .await?,
            CasOutcome::Swapped
        );
        if swapped {
            self.sync_run_index_best_effort(&replacement.run_id).await;
            self.emit_run_update(&replacement);
            self.archive_run(&replacement).await;
        }
        Ok(swapped)
    }

    async fn delete_run_if_unchanged(&self, run: &RunRecordV1) -> Result<(), SecurityScanError> {
        let expected = serialize(run, "run record")?;
        let deleted = matches!(
            self.compare_and_set(&run.run_id, Some(expected), None)
                .await?,
            CasOutcome::Swapped
        );
        if deleted {
            self.sync_run_index_best_effort(&run.run_id).await;
        }
        Ok(())
    }

    async fn enqueue_execute(&self, request: EnqueueRequest) -> Result<(), SecurityScanError> {
        self.call(
            EXECUTE_ID,
            serialize(&request, "queue request")?,
            Some(TriggerAction::Enqueue {
                queue: RUN_QUEUE.into(),
            }),
            None,
        )
        .await
        .map(|_| ())
    }

    async fn get_action(
        &self,
        action_id: &str,
    ) -> Result<Option<crate::SecurityActionRecordV1>, SecurityScanError> {
        let value = self
            .call_private(
                STATE_GET_ID,
                json!({ "scope": ACTION_SCOPE, "key": action_id }),
            )
            .await?;
        parse_optional_action(value, action_id)
    }

    async fn list_actions(&self) -> Result<Vec<crate::SecurityActionRecordV1>, SecurityScanError> {
        let value = self
            .call_private(STATE_LIST_ID, json!({ "scope": ACTION_SCOPE }))
            .await?;
        parse_state_list(&value, "private action")
    }

    async fn create_action_if_absent(
        &self,
        action: crate::SecurityActionRecordV1,
    ) -> Result<crate::CreateActionOutcome, SecurityScanError> {
        let value = serialize(&action, "action record")?;
        match self
            .compare_and_set_in_scope(ACTION_SCOPE, &action.action_id, None, Some(value))
            .await?
        {
            CasOutcome::Swapped => {
                self.emit_action_update(&action);
                Ok(crate::CreateActionOutcome::Created)
            }
            CasOutcome::Current(current) => {
                let existing = parse_action(current, &action.action_id)?;
                if existing.action_id != action.action_id
                    || existing.run_id != action.run_id
                    || existing.finding_index != action.finding_index
                    || existing.action != action.action
                {
                    return Err(SecurityScanError::Dependency(format!(
                        "state collision or corruption for action {}",
                        action.action_id
                    )));
                }
                Ok(crate::CreateActionOutcome::Existing(Box::new(existing)))
            }
        }
    }

    async fn replace_action(
        &self,
        expected: &crate::SecurityActionRecordV1,
        replacement: crate::SecurityActionRecordV1,
    ) -> Result<bool, SecurityScanError> {
        if expected.action_id != replacement.action_id
            || expected.run_id != replacement.run_id
            || expected.finding_index != replacement.finding_index
            || expected.action != replacement.action
            || expected.repository != replacement.repository
            || expected.target_sha != replacement.target_sha
        {
            return Err(SecurityScanError::Dependency(
                "action replacement changed immutable identity fields".into(),
            ));
        }
        let expected_value = serialize(expected, "expected action record")?;
        let replacement_value = serialize(&replacement, "replacement action record")?;
        let previous_session = expected
            .harness
            .as_ref()
            .map(|harness| harness.session_id.as_str());
        let replacement_session = replacement
            .harness
            .as_ref()
            .map(|harness| harness.session_id.as_str());
        let session_changed = previous_session != replacement_session;
        if session_changed {
            if let Some(session_id) = previous_session {
                self.forget_action_session(session_id, &replacement.action_id)
                    .await?;
            }
        }
        let inserted_replacement_session = if session_changed {
            match replacement_session {
                Some(session_id) => match self
                    .remember_action_session(session_id, &replacement.action_id)
                    .await
                {
                    Ok(inserted) => inserted,
                    Err(error) => {
                        if let Some(previous_session) = previous_session {
                            self.remember_action_session(previous_session, &replacement.action_id)
                                .await?;
                        }
                        return Err(error);
                    }
                },
                None => false,
            }
        } else {
            false
        };
        let replacement_outcome = match self
            .compare_and_set_in_scope(
                ACTION_SCOPE,
                &expected.action_id,
                Some(expected_value),
                Some(replacement_value),
            )
            .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                if inserted_replacement_session {
                    if let Some(session_id) = replacement_session {
                        self.forget_action_session(session_id, &replacement.action_id)
                            .await?;
                    }
                }
                if session_changed {
                    if let Some(session_id) = previous_session {
                        self.remember_action_session(session_id, &replacement.action_id)
                            .await?;
                    }
                }
                return Err(error);
            }
        };
        let swapped = matches!(replacement_outcome, CasOutcome::Swapped);
        if swapped {
            self.emit_action_update(&replacement);
        } else if session_changed {
            if inserted_replacement_session {
                if let Some(session_id) = replacement_session {
                    self.forget_action_session(session_id, &replacement.action_id)
                        .await?;
                }
            }
            if let Some(session_id) = previous_session {
                self.remember_action_session(session_id, &replacement.action_id)
                    .await?;
            }
        }
        Ok(swapped)
    }

    async fn delete_action_if_unchanged(
        &self,
        action: &crate::SecurityActionRecordV1,
    ) -> Result<(), SecurityScanError> {
        let expected = serialize(action, "action record")?;
        let _ = self
            .compare_and_set_in_scope(ACTION_SCOPE, &action.action_id, Some(expected), None)
            .await?;
        if let Some(harness) = action.harness.as_ref() {
            self.forget_action_session(&harness.session_id, &action.action_id)
                .await?;
        }
        Ok(())
    }

    async fn enqueue_action_execute(
        &self,
        request: crate::ActionEnqueueRequestV1,
    ) -> Result<(), SecurityScanError> {
        self.call(
            ACTION_EXECUTE_ID,
            serialize(&request, "action queue request")?,
            Some(TriggerAction::Enqueue {
                queue: ACTION_QUEUE.into(),
            }),
            None,
        )
        .await
        .map(|_| ())
    }

    async fn approval_gate_is_live(&self) -> Result<bool, SecurityScanError> {
        match self
            .call(
                "engine::functions::info",
                json!({ "function_id": "approval::gate" }),
                None,
                Some(5_000),
            )
            .await
        {
            Ok(value) => Ok(function_info_matches(&value, "approval::gate")),
            Err(error) if accessor_is_missing(&error) => Ok(false),
            Err(error) => Err(error),
        }
    }

    async fn stop_analysis(&self, harness: &crate::HarnessRunV1) -> Result<(), SecurityScanError> {
        match self
            .call(
                "harness::stop",
                json!({
                    "session_id": harness.session_id,
                    "turn_id": harness.turn_id,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await
        {
            Ok(_) => Ok(()),
            Err(error) if accessor_is_missing(&error) => Ok(()),
            Err(error) => Err(error),
        }
    }

    async fn ensure_analysis_chat_link(
        &self,
        run: &RunRecordV1,
    ) -> Result<bool, SecurityScanError> {
        let Some(harness) = run.harness.as_ref() else {
            return Ok(false);
        };
        let response = self
            .call(
                "session::get",
                json!({ "session_id": harness.session_id }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        if response.is_null() {
            return Ok(false);
        }
        let meta = response
            .get("meta")
            .and_then(Value::as_object)
            .ok_or_else(|| {
                SecurityScanError::Dependency(
                    "session::get returned no metadata for the analysis chat".into(),
                )
            })?;
        let mut metadata = meta
            .get("metadata")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        let already_linked = metadata.get("security_scan") == Some(&Value::Bool(true))
            && metadata.get("security_scan_run_id").and_then(Value::as_str)
                == Some(run.run_id.as_str());
        if !already_linked {
            metadata.insert("security_scan".into(), Value::Bool(true));
            metadata.insert(
                "security_scan_run_id".into(),
                Value::String(run.run_id.clone()),
            );
            self.call(
                "session::set-meta",
                json!({
                    "session_id": harness.session_id,
                    "metadata": metadata,
                }),
                None,
                Some(RPC_TIMEOUT_MS),
            )
            .await?;
        }
        Ok(true)
    }
}
