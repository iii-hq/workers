use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::evidence_data::RunEvidence;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;

use super::super::floor;
use super::super::runner::ScenarioRunner;
use super::super::state::{ActiveTurn, PreparedRun};

const COLLECTION_TIMEOUT: Duration = Duration::from_secs(30);

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn collect(
        &mut self,
        services: &RunServices,
        prepared: &PreparedRun,
        active: &mut ActiveTurn,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Collect;
        // Evidence transport is runner infrastructure, not subject
        // completion. Give it an independent bounded deadline so an RPC
        // failure here is runner_error even when the subject deadline has
        // already elapsed in Await.
        let deadline = Deadline::after(COLLECTION_TIMEOUT);
        active.transcript = self.collect_transcript(services.client(), deadline).await?;
        self.write_artifact(
            &prepared.scenario.id,
            "transcript.json",
            &active.transcript,
            phase,
        )?;
        self.write_artifact(
            &prepared.scenario.id,
            "status.json",
            &active.final_status,
            phase,
        )?;
        self.write_artifact(
            &prepared.scenario.id,
            "control.json",
            &active.control,
            phase,
        )?;
        self.write_artifact(
            &prepared.scenario.id,
            "tree-status.json",
            &active.tree_statuses,
            phase,
        )?;
        active.router_evidence = services.router().evidence();
        self.write_artifact(
            &prepared.scenario.id,
            "router-calls.json",
            &active.router_evidence,
            phase,
        )?;

        let session_ids = if active.tree_sessions.is_empty() {
            vec![self.session_id.clone()]
        } else {
            active.tree_sessions.clone()
        };
        active.traces = if active.timed_out {
            crate::trace_evidence::collect_available_for_sessions(
                services.client(),
                &session_ids,
                deadline,
            )
            .await
        } else {
            crate::trace_evidence::collect_for_sessions(
                services.client(),
                services.probe(),
                &session_ids,
                self.fixture.expected_terminal_turns,
                self.fixture.expected_turn_statuses.len(),
                active.trace_generation,
                deadline,
            )
            .await
        }
        .map_err(|error| RunError::runner(phase, "collect session traces", error))?;
        self.write_artifact(&prepared.scenario.id, "traces.json", &active.traces, phase)?;
        Ok(())
    }

    /// Assemble the returned dataset from everything Collect persisted.
    /// `send_response` is `Some` only in Rpc mode, where the runner itself
    /// submitted `harness::send`.
    pub(in crate::scenario) fn build_evidence(
        &self,
        services: &RunServices,
        active: &ActiveTurn,
        send_response: Option<Value>,
    ) -> RunEvidence {
        RunEvidence {
            run_id: self.run_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: active.turn_id.clone(),
            send_response,
            status: active.final_status.clone(),
            transcript: active.transcript.clone(),
            generations_consumed: services.router().generations_consumed(),
            generations_total: services.router().total_generations(),
            traces: active.traces.clone(),
            target_calls: services.probe().target_calls(),
            control: active.control.clone(),
            tree_sessions: active.tree_sessions.clone(),
            tree_statuses: active.tree_statuses.clone(),
            router_evidence: active.router_evidence.clone(),
        }
    }

    /// Enforce the runner-owned floor, then run the scenario's `verify`
    /// function. The first failure is recorded with run-scoped ids scrubbed
    /// so persisted results stay byte-stable.
    pub(in crate::scenario) fn verify_evidence(
        &mut self,
        services: &RunServices,
        evidence: &RunEvidence,
        timed_out: bool,
    ) -> Result<(), RunError> {
        let failure = floor::floor_failure_for(
            evidence,
            &floor::FloorExpectations {
                turn_statuses: &self.fixture.expected_turn_statuses,
                terminal_turns: self.fixture.expected_terminal_turns,
                traces: self.fixture.expected_traces(),
            },
        )
        .or_else(|| floor::verify_failure(self.fixture.verify, evidence));
        self.failure = failure.map(|message| evidence.scrub(&message));

        if timed_out {
            return Err(RunError::new(
                RunPhase::Await,
                RunErrorKind::Timeout,
                "terminal status did not arrive before the scenario deadline",
            ));
        }
        if self.failure.is_none() && services.router().contract_failed() {
            self.failure = Some("scripted router reported a contract failure".to_string());
        }
        match &self.failure {
            Some(message) => Err(RunError::new(
                RunPhase::Grade,
                RunErrorKind::Contract,
                message.clone(),
            )),
            None => Ok(()),
        }
    }

    async fn collect_transcript(
        &self,
        client: &Client,
        deadline: Deadline,
    ) -> Result<Vec<Value>, RunError> {
        let phase = RunPhase::Collect;
        let mut messages = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = BTreeSet::new();

        loop {
            let mut payload = json!({
                "session_id": self.session_id,
                "limit": 100,
                "include_custom": true,
            });
            if let Some(cursor) = &cursor {
                payload["cursor"] = json!(cursor);
            }
            let page = client
                .call_with_deadline(
                    "session::messages",
                    payload,
                    deadline,
                    DEFAULT_CALL_TIMEOUT_MS,
                )
                .await
                .map_err(|error| {
                    RunError::runner(phase, "collect session transcript", anyhow::anyhow!(error))
                })?;
            if let Some(items) = page.get("messages").and_then(Value::as_array) {
                messages.extend(items.iter().cloned());
            }
            let Some(next) = page.get("next_cursor").and_then(Value::as_str) else {
                break;
            };
            if !seen_cursors.insert(next.to_string()) {
                return Err(RunError::new(
                    phase,
                    RunErrorKind::Runner,
                    format!("session::messages repeated cursor {next:?}"),
                ));
            }
            cursor = Some(next.to_string());
        }
        Ok(messages)
    }
}
