//! `harness::ask::settle` / `harness::ask::settlements` — the console's
//! record of what became of a `harness::ask` card (see `ask_settlement`).
//! Trusted control-plane: registered, kept off the agent catalog.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ask_settlement::{self, AskOutcome, AskSettlement};
use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::message::AgentMessage;

pub const SETTLE_ID: &str = "harness::ask::settle";
pub const SETTLE_DESC: &str =
    "Internal control-plane: record that a harness::ask card was answered through the card or \
     dismissed, so every client shows it settled.";

pub const SETTLEMENTS_ID: &str = "harness::ask::settlements";
pub const SETTLEMENTS_DESC: &str =
    "Internal control-plane: list which harness::ask cards in a session were answered or \
     dismissed (the console's ask card reads this).";

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SettleRequest {
    pub session_id: String,
    /// The ask's function-call id (`harness::ask` result `question_id`).
    pub question_id: String,
    pub outcome: AskOutcome,
}

#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub struct SettlementsRequest {
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SettlementsResponse {
    /// The session's settlements, oldest first (at most
    /// `ask_settlement::MAX_SETTLEMENTS`).
    pub settlements: Vec<AskSettlement>,
}

pub async fn settle(deps: &Deps, req: SettleRequest) -> Result<SettlementsResponse, HarnessError> {
    if req.question_id.trim().is_empty() {
        return Err(HarnessError::InvalidRequest(
            "harness::ask::settle needs the ask's question_id".into(),
        ));
    }
    let settlements = ask_settlement::settle(
        deps,
        &req.session_id,
        AskSettlement {
            question_id: req.question_id,
            outcome: req.outcome,
            settled_at: AgentMessage::now_ms(),
        },
    )
    .await?;
    Ok(SettlementsResponse { settlements })
}

pub async fn settlements(
    deps: &Deps,
    req: SettlementsRequest,
) -> Result<SettlementsResponse, HarnessError> {
    Ok(SettlementsResponse {
        settlements: ask_settlement::list(deps, &req.session_id).await?,
    })
}
