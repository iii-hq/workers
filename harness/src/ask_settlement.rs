//! What became of a `harness::ask` card: answered through the card, or
//! dismissed without an answer. Durable per session, so a card that was
//! settled in one tab (or before a reload) reads as settled everywhere.
//!
//! WHY a store of its own: the card used to infer "answered" from the
//! session's turn moving past the asking one. Any message the person typed
//! did that too, so a card whose question was never answered closed as
//! answered the moment the person said something else, and an option ticked
//! afterwards went nowhere. Only the card's own Send or Dismiss settles a
//! question now, and this is where it records that.
//!
//! One row per session, holding the newest [`MAX_SETTLEMENTS`] settlements:
//! one `state::get` answers every card in the chat, and the row cannot grow
//! without bound. The write is a read-modify-write under the session lock.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;

/// State scope of the per-session settlement rows (key: the session id).
pub const ASK_SCOPE: &str = "harness_ask";

/// Settlements kept per session; older ones drop off the front.
pub const MAX_SETTLEMENTS: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum AskOutcome {
    /// The card's answer (every question answered or skipped) was sent.
    Answered,
    /// The card was dismissed: the agent was told it gets no answer.
    Dismissed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AskSettlement {
    /// The function-call id of the ask (`AskResponse::question_id`).
    pub question_id: String,
    pub outcome: AskOutcome,
    /// Unix milliseconds.
    pub settled_at: i64,
}

/// `settlements` with `next` recorded: a repeat for the same question
/// replaces the earlier entry (the newest outcome wins), and the list keeps
/// its newest [`MAX_SETTLEMENTS`] entries in recording order.
pub fn with_settlement(mut settlements: Vec<AskSettlement>, next: AskSettlement) -> Vec<AskSettlement> {
    settlements.retain(|s| s.question_id != next.question_id);
    settlements.push(next);
    let overflow = settlements.len().saturating_sub(MAX_SETTLEMENTS);
    settlements.drain(..overflow);
    settlements
}

/// The session's settlements, oldest first. An absent or unreadable row is
/// an empty list: a row written by a newer harness degrades to "nothing
/// settled", never to an error that blocks the card.
pub async fn list(deps: &Deps, session_id: &str) -> Result<Vec<AskSettlement>, HarnessError> {
    let cfg = deps.cfg().await;
    let value =
        crate::state::state_get(&deps.iii, ASK_SCOPE, session_id, cfg.session_timeout_ms).await?;
    if value.is_null() {
        return Ok(Vec::new());
    }
    Ok(serde_json::from_value(value).unwrap_or_default())
}

/// Record a settlement and return the session's list after it. Under the
/// session lock: two cards settling at once must not lose one another.
pub async fn settle(
    deps: &Deps,
    session_id: &str,
    next: AskSettlement,
) -> Result<Vec<AskSettlement>, HarnessError> {
    let _guard = deps.locks.guard(session_id).await;
    let settlements = with_settlement(list(deps, session_id).await?, next);
    let cfg = deps.cfg().await;
    let value = serde_json::to_value(&settlements)
        .map_err(|e| HarnessError::State(format!("ask settlement serialize: {e}")))?;
    crate::state::state_set(&deps.iii, ASK_SCOPE, session_id, value, cfg.session_timeout_ms).await?;
    Ok(settlements)
}

/// Drop a session's row when the session itself is deleted.
pub async fn purge(deps: &Deps, session_id: &str, timeout_ms: u64) -> Result<(), HarnessError> {
    crate::state::state_delete(&deps.iii, ASK_SCOPE, session_id, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settled(question_id: &str, outcome: AskOutcome, at: i64) -> AskSettlement {
        AskSettlement {
            question_id: question_id.into(),
            outcome,
            settled_at: at,
        }
    }

    #[test]
    fn a_settlement_is_appended_in_recording_order() {
        let list = with_settlement(vec![], settled("call_1", AskOutcome::Answered, 1));
        let list = with_settlement(list, settled("call_2", AskOutcome::Dismissed, 2));
        assert_eq!(
            list.iter().map(|s| s.question_id.as_str()).collect::<Vec<_>>(),
            ["call_1", "call_2"]
        );
    }

    #[test]
    fn a_repeat_for_the_same_question_keeps_the_newest_outcome_once() {
        let list = with_settlement(vec![], settled("call_1", AskOutcome::Answered, 1));
        let list = with_settlement(list, settled("call_2", AskOutcome::Answered, 2));
        let list = with_settlement(list, settled("call_1", AskOutcome::Dismissed, 3));
        assert_eq!(
            list,
            vec![
                settled("call_2", AskOutcome::Answered, 2),
                settled("call_1", AskOutcome::Dismissed, 3),
            ]
        );
    }

    #[test]
    fn the_list_keeps_only_the_newest_entries() {
        let mut list = Vec::new();
        for i in 0..(MAX_SETTLEMENTS as i64 + 3) {
            list = with_settlement(list, settled(&format!("call_{i}"), AskOutcome::Answered, i));
        }
        assert_eq!(list.len(), MAX_SETTLEMENTS);
        assert_eq!(list[0].question_id, "call_3");
        assert_eq!(list.last().unwrap().question_id, format!("call_{}", MAX_SETTLEMENTS + 2));
    }

    #[test]
    fn the_wire_shape_is_snake_case() {
        let value = serde_json::to_value(settled("call_1", AskOutcome::Dismissed, 9)).unwrap();
        assert_eq!(
            value,
            serde_json::json!({ "question_id": "call_1", "outcome": "dismissed", "settled_at": 9 })
        );
    }
}
