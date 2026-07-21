use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::{Client, DEFAULT_CALL_TIMEOUT_MS};
use crate::deadline::Deadline;
use crate::grader::{self, Evidence};
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::types::recorder::RecorderEventKind;

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
            "router-calls.json",
            &services.router().evidence(),
            phase,
        )?;

        active.recorder_events = services
            .recorder()
            .snapshot(None)
            .map_err(|error| RunError::runner(phase, "snapshot recorder evidence", error))?;
        let (target_calls, lifecycle_events): (Vec<_>, Vec<_>) = active
            .recorder_events
            .iter()
            .partition(|event| event.kind == RecorderEventKind::TargetCall);
        self.write_artifact(
            &prepared.scenario.id,
            "target-calls.json",
            &target_calls,
            phase,
        )?;
        self.write_artifact(
            &prepared.scenario.id,
            "lifecycle-events.json",
            &lifecycle_events,
            phase,
        )?;
        Ok(())
    }

    pub(in crate::scenario) fn grade(
        &mut self,
        services: &RunServices,
        prepared: &PreparedRun,
        active: &ActiveTurn,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Grade;
        let evidence = Evidence {
            run_id: self.run_id.clone(),
            session_id: self.session_id.clone(),
            turn_id: active.turn_id.clone(),
            send_response: Some(active.send_response.clone()),
            status: active.final_status.clone(),
            transcript: active.transcript.clone(),
            generations_consumed: services.router().generations_consumed(),
            generations_total: services.router().total_generations(),
            recorder_events: active.recorder_events.clone(),
        };
        self.invariants = grader::grade(&prepared.scenario.invariants, &evidence);
        let invariants = self.invariants.clone();
        self.write_artifact(&prepared.scenario.id, "invariants.json", &invariants, phase)?;

        if active.timed_out {
            return Err(RunError::new(
                RunPhase::Await,
                RunErrorKind::Timeout,
                "terminal status did not arrive before the scenario deadline",
            ));
        }
        if self.invariants.iter().any(|invariant| !invariant.passed)
            || services.router().contract_failed()
        {
            return Err(RunError::new(
                phase,
                RunErrorKind::Contract,
                "one or more scenario contracts failed",
            ));
        }
        Ok(())
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
