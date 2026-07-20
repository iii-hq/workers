use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Value};

use crate::client::DEFAULT_CALL_TIMEOUT_MS;
use crate::deadline::Deadline;
use crate::runtime::{RunError, RunErrorKind, RunPhase};
use crate::services::RunServices;
use crate::types::recorder::RecorderEventKind;

use super::super::runner::ScenarioRunner;
use super::super::state::ActiveTurn;
use super::{STATUS_POLL_INTERVAL, TARGET_POLL_INTERVAL};

const LIFECYCLE_GRACE: Duration = Duration::from_secs(3);

impl ScenarioRunner<'_> {
    pub(in crate::scenario) async fn r#await(
        &mut self,
        services: &RunServices,
        active: &mut ActiveTurn,
    ) -> Result<(), RunError> {
        let phase = RunPhase::Await;
        let deadline = active.deadline;
        let last_status = Arc::new(Mutex::new(Value::Null));
        let last_status_error = Arc::new(Mutex::new(None::<String>));
        let session_id = self.session_id.clone();

        let terminal = deadline
            .poll_until("terminal harness status", STATUS_POLL_INTERVAL, || {
                let last_status = Arc::clone(&last_status);
                let last_status_error = Arc::clone(&last_status_error);
                let session_id = session_id.clone();
                async move {
                    let response = services
                        .client()
                        .call_with_deadline(
                            "harness::status",
                            json!({ "session_id": session_id }),
                            deadline,
                            DEFAULT_CALL_TIMEOUT_MS,
                        )
                        .await;
                    let status = match response {
                        Ok(status) => {
                            *last_status_error.lock().map_err(|_| {
                                anyhow::anyhow!("last status error lock poisoned")
                            })? = None;
                            status
                        }
                        Err(error) => {
                            *last_status_error.lock().map_err(|_| {
                                anyhow::anyhow!("last status error lock poisoned")
                            })? = Some(error);
                            return Ok(None);
                        }
                    };
                    *last_status
                        .lock()
                        .map_err(|_| anyhow::anyhow!("last status lock poisoned"))? =
                        status.clone();
                    let terminal = matches!(
                        status.get("status").and_then(Value::as_str),
                        Some("completed") | Some("failed") | Some("cancelled")
                    );
                    Ok(terminal.then_some(status))
                }
            })
            .await;

        match terminal {
            Ok(status) => active.final_status = status,
            Err(error) if deadline.is_expired() => {
                if let Some(status_error) = last_status_error
                    .lock()
                    .map_err(|_| {
                        RunError::new(
                            phase,
                            RunErrorKind::Runner,
                            "last status error lock poisoned",
                        )
                    })?
                    .clone()
                {
                    return Err(RunError::with_source(
                        phase,
                        RunErrorKind::Runner,
                        "harness::status remained unavailable at the scenario deadline",
                        anyhow::anyhow!(status_error),
                    ));
                }
                active.timed_out = true;
                active.final_status = last_status
                    .lock()
                    .map_err(|_| {
                        RunError::new(phase, RunErrorKind::Runner, "last status lock poisoned")
                    })?
                    .clone();
                tracing::error!(
                    target: "harness_integration::scenario",
                    "await timed out: {error:#}"
                );
                return Ok(());
            }
            Err(error) => {
                return Err(RunError::with_source(
                    phase,
                    RunErrorKind::Runner,
                    "poll terminal harness status",
                    error,
                ));
            }
        }

        let grace_expires =
            (tokio::time::Instant::now() + LIFECYCLE_GRACE).min(deadline.expires_at());
        let grace = Deadline::at(grace_expires);
        if !grace.is_expired() {
            let lifecycle = grace
                .poll_until("lifecycle delivery grace", TARGET_POLL_INTERVAL, || async {
                    let events = services.recorder().snapshot(None)?;
                    Ok(events
                        .iter()
                        .any(|event| event.kind == RecorderEventKind::Lifecycle)
                        .then_some(()))
                })
                .await;
            if let Err(error) = lifecycle {
                if !grace.is_expired() {
                    return Err(RunError::with_source(
                        phase,
                        RunErrorKind::Runner,
                        "poll lifecycle delivery",
                        error,
                    ));
                }
            }
        }
        Ok(())
    }
}
