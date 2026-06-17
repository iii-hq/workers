//! `harness::run` — `harness::send` with the call held open until the turn
//! ends (harness.md § `harness::run`). On timeout the turn is NOT aborted; the
//! caller re-attaches via `harness::status` or the turn-completed event.

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::functions::send::{self, SendRequest};
use crate::types::message::{AgentMessage, AssistantMessage};
use crate::types::turn::TurnStatus;

const POLL_INTERVAL_MS: u64 = 200;
const DEFAULT_TIMEOUT_MS: u64 = 300_000;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunRequest {
    #[serde(flatten)]
    pub send: SendRequest,
    /// Hold-open budget; default 300000ms. On expiry the turn keeps running.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RunResponse {
    pub session_id: String,
    pub turn_id: String,
    /// Terminal status — or the live one when `timed_out`.
    pub status: TurnStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timed_out: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_error: Option<String>,
    /// The turn's last assistant message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub final_message: Option<AssistantMessage>,
}

pub async fn handle(deps: &Deps, req: RunRequest) -> Result<RunResponse, HarnessError> {
    let timeout_ms = req.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let started = send::start(deps, req.send).await?;
    let cfg = deps.cfg().await;

    let deadline = std::time::Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let record =
            crate::state::get_turn(&deps.iii, &started.session_id, cfg.session_timeout_ms).await?;
        if let Some(rec) = &record {
            if rec.turn_id == started.turn_id && rec.status.is_terminal() {
                let final_message = last_assistant(deps, &started.session_id).await;
                return Ok(RunResponse {
                    session_id: started.session_id,
                    turn_id: started.turn_id,
                    status: rec.status,
                    timed_out: None,
                    result: rec.result.clone(),
                    result_error: rec.result_error.clone(),
                    final_message,
                });
            }
        }
        if std::time::Instant::now() >= deadline {
            let status = record
                .as_ref()
                .map(|r| r.status)
                .unwrap_or(TurnStatus::Running);
            return Ok(RunResponse {
                session_id: started.session_id,
                turn_id: started.turn_id,
                status,
                timed_out: Some(true),
                result: record.as_ref().and_then(|r| r.result.clone()),
                result_error: record.as_ref().and_then(|r| r.result_error.clone()),
                final_message: None,
            });
        }
        tokio::time::sleep(Duration::from_millis(POLL_INTERVAL_MS)).await;
    }
}

async fn last_assistant(deps: &Deps, session_id: &str) -> Option<AssistantMessage> {
    let session = deps.session().await;
    let entries = session.messages(session_id, false).await.ok()?;
    entries.into_iter().rev().find_map(|e| match e.message {
        Some(AgentMessage::Assistant(a)) => Some(a),
        _ => None,
    })
}
