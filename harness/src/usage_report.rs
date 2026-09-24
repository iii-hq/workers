//! Anonymous usage reporting for the turn loop.
//!
//! One durable publish on the `harness:usage` topic per reported moment, which
//! the engine's `telemetry` worker subscribes to and forwards as a single
//! product event (see `engine/src/workers/telemetry/harness.rs`). The harness
//! never talks to an analytics vendor itself: it announces, and an engine that
//! the operator opted out of telemetry simply never listens.
//!
//! Two reported moments, both bounded per session so a long run never costs
//! more events than a short one:
//!
//! * `harness_session_progress` at root turn 1, 2, 5, 10, 25 and 50. Every
//!   report is cumulative, so the last one supersedes the ones before it and
//!   the histogram of milestones reached IS the drop-off curve.
//! * `harness_turn_failed` once per session per outcome (`failed`,
//!   `cancelled`, `max_turns`), which keeps the reason a run stopped exact
//!   even when it stopped between two milestones.
//!
//! Only root turns report. A sub-agent's usage is already inside its root's
//! cumulative totals, which `harness::metrics` aggregates over the whole
//! session tree.
//!
//! Nothing a user wrote is reported: no message text, no prompts, no file
//! paths, no function ids, no error strings. Model, provider, counters, and
//! the fixed failure buckets `failure_class` already defines.
//!
//! Every step is best effort. A failed read, a failed publish, or a missing
//! `queue` worker is logged at debug and the turn continues — usage reporting
//! that can break a turn is worse than no usage reporting.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use iii_sdk::protocol::TriggerRequest;

use crate::deps::Deps;
use crate::functions::metrics::{self, SessionMetricsRequestV1};
use crate::state::{self, USAGE_SCOPE};
use crate::types::message::AgentMessage;
use crate::types::turn::TurnRecord;

/// Topic the engine's telemetry worker subscribes to.
pub const USAGE_TOPIC: &str = "harness:usage";
const PUBLISH_ID: &str = "iii::durable::publish";

pub const PROGRESS_EVENT: &str = "harness_session_progress";
pub const FAILED_EVENT: &str = "harness_turn_failed";

/// Root turn counts that report. Chosen to be dense where sessions die and
/// sparse afterwards: the gap between 1 and 2 is the one that says whether the
/// harness was tried once or actually used.
pub const MILESTONES: [u64; 6] = [1, 2, 5, 10, 25, 50];

/// Per-session reporting bookkeeping (`harness_usage/<session_id>`).
///
/// Durable rather than in memory because it is what makes a report happen
/// once: a restarted harness, a redelivered turn step, or a second worker
/// must not re-announce a milestone this session already announced.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    /// Root turns this session has completed, terminal or not.
    #[serde(default)]
    pub turns: u64,
    /// Milestones already announced.
    #[serde(default)]
    pub reported: Vec<u64>,
    /// Outcomes already announced, one announcement each.
    #[serde(default)]
    pub outcomes: Vec<String>,
    /// When the first reported turn of this session ended, so a later report
    /// can carry the session's age without a transcript read.
    #[serde(default)]
    pub first_at: i64,
}

/// The milestone this turn count reports at, if any.
pub fn milestone(turns: u64) -> Option<u64> {
    MILESTONES.contains(&turns).then_some(turns)
}

/// Whether this turn belongs to a root session. A sub-agent turn reports
/// nothing of its own, and neither does a turn a reaction delivered into a
/// session on a parent's behalf: it has no `parent` link, but its
/// `display_parent_session_id` says whose tree it belongs to.
fn is_root(record: &TurnRecord) -> bool {
    record.depth == 0 && record.parent.is_none() && record.display_parent_session_id.is_none()
}

/// Report one finished root turn: the milestone if it reached one, and the
/// outcome if it was not an ordinary completion.
///
/// `outcome` is the turn's own status (`completed`, `failed`, `cancelled`) or
/// `max_turns` when the step cap ended it. `error_kind` is
/// [`crate::turn_loop::failure_class`]'s bucket for a failure and `None`
/// otherwise.
pub async fn report(deps: &Deps, record: &TurnRecord, outcome: &str, error_kind: Option<&str>) {
    if !is_root(record) {
        return;
    }
    let cfg = deps.cfg().await;
    let timeout_ms = cfg.session_timeout_ms;
    let session_id = &record.session_id;

    let mut row = match load(deps, session_id, timeout_ms).await {
        Ok(row) => row,
        Err(e) => {
            tracing::debug!(session_id = %session_id, error = %e, "usage row unreadable");
            return;
        }
    };
    let now = AgentMessage::now_ms();
    row.turns += 1;
    if row.first_at == 0 {
        row.first_at = now;
    }

    let mut announced = false;

    if outcome != "completed" && !row.outcomes.iter().any(|o| o == outcome) {
        let mut payload = json!({
            "event": FAILED_EVENT,
            "dedupe_key": format!("{session_id}:{outcome}"),
            "outcome": outcome,
            "turn_index": row.turns,
            "model": record.options.model,
        });
        if let Some(kind) = error_kind {
            payload["error_kind"] = Value::String(kind.to_string());
        }
        if let Some(provider) = &record.options.provider {
            payload["provider"] = Value::String(provider.clone());
        }
        if publish(deps, payload, timeout_ms).await {
            row.outcomes.push(outcome.to_string());
            announced = true;
        }
    }

    if let Some(reached) = milestone(row.turns) {
        if !row.reported.contains(&reached) {
            // ponytail: `harness::metrics` re-walks the session tree's
            // transcript, so this read grows with the session. It runs at most
            // six times per session, which is why it is not incremental
            // counters; make it incremental if a milestone read ever shows up
            // in a turn's latency.
            match metrics::handle(
                deps,
                SessionMetricsRequestV1 {
                    root_session_id: session_id.clone(),
                },
            )
            .await
            {
                Ok(m) => {
                    let totals = m.totals;
                    let mut payload = json!({
                        "event": PROGRESS_EVENT,
                        "dedupe_key": format!("{session_id}:progress:{reached}"),
                        "turn_index": reached,
                        "outcome": outcome,
                        "model": record.options.model,
                        "sessions_total": totals.sessions,
                        "turns_total": totals.turns,
                        "tool_calls_total": totals.function_calls,
                        "tool_call_errors_total": totals.function_call_errors,
                        "input_tokens": totals.input_tokens,
                        "output_tokens": totals.output_tokens,
                        "cache_read_tokens": totals.cache_read_tokens,
                        "cache_write_tokens": totals.cache_write_tokens,
                        "cost_usd": totals.cost_usd,
                        "session_age_ms": (now - row.first_at).max(0),
                    });
                    if let Some(provider) = &record.options.provider {
                        payload["provider"] = Value::String(provider.clone());
                    }
                    if publish(deps, payload, timeout_ms).await {
                        row.reported.push(reached);
                        announced = true;
                    }
                }
                Err(e) => {
                    tracing::debug!(
                        session_id = %session_id,
                        error = %e,
                        "usage metrics unavailable"
                    );
                }
            }
        }
    }

    // The turn counter is worth keeping even when nothing was announced: it is
    // what decides the next milestone.
    if let Err(e) = save(deps, session_id, &row, timeout_ms).await {
        tracing::debug!(session_id = %session_id, error = %e, "usage row unwritable");
    } else if announced {
        tracing::debug!(
            session_id = %session_id,
            turns = row.turns,
            outcome,
            "usage reported"
        );
    }
}

/// Published through the `queue` worker, not fire-and-forget pub/sub: a turn
/// ends once, and the message waits in the queue until a subscriber takes it
/// rather than being dropped when nothing is listening at that instant.
async fn publish(deps: &Deps, data: Value, timeout_ms: u64) -> bool {
    let sent = deps
        .iii
        .trigger(TriggerRequest {
            function_id: PUBLISH_ID.into(),
            payload: json!({ "topic": USAGE_TOPIC, "data": data }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await;
    match sent {
        Ok(_) => true,
        Err(e) => {
            tracing::debug!(error = %e, topic = USAGE_TOPIC, "usage publish failed");
            false
        }
    }
}

async fn load(
    deps: &Deps,
    session_id: &str,
    timeout_ms: u64,
) -> Result<UsageRow, crate::error::HarnessError> {
    let value = state::state_get(&deps.iii, USAGE_SCOPE, session_id, timeout_ms).await?;
    if value.is_null() {
        return Ok(UsageRow::default());
    }
    Ok(serde_json::from_value(value).unwrap_or_default())
}

async fn save(
    deps: &Deps,
    session_id: &str,
    row: &UsageRow,
    timeout_ms: u64,
) -> Result<(), crate::error::HarnessError> {
    let value = serde_json::to_value(row)
        .map_err(|e| crate::error::HarnessError::State(format!("usage row serialize: {e}")))?;
    state::state_set(&deps.iii, USAGE_SCOPE, session_id, value, timeout_ms).await
}

/// Drop a deleted session's row.
pub async fn delete(
    iii: &iii_sdk::IIIClient,
    session_id: &str,
    timeout_ms: u64,
) -> Result<(), crate::error::HarnessError> {
    state::state_delete(iii, USAGE_SCOPE, session_id, timeout_ms).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_listed_turn_counts_report() {
        for turns in MILESTONES {
            assert_eq!(milestone(turns), Some(turns));
        }
        for turns in [0, 3, 4, 9, 26, 51, 1_000] {
            assert_eq!(milestone(turns), None, "turn {turns} must not report");
        }
    }

    #[test]
    fn only_a_turn_with_no_parent_of_either_kind_is_root() {
        let root = crate::types::turn::tests::record();
        assert!(is_root(&root));

        let mut displayed = root.clone();
        displayed.display_parent_session_id = Some("s_parent".into());
        assert!(!is_root(&displayed));

        let mut deep = root.clone();
        deep.depth = 1;
        assert!(!is_root(&deep));
    }

    #[test]
    fn a_fifty_turn_session_reports_six_times() {
        let reported = (1..=50).filter(|t| milestone(*t).is_some()).count();
        assert_eq!(reported, MILESTONES.len());
    }

    #[test]
    fn a_missing_row_starts_empty_and_a_malformed_one_does_too() {
        assert_eq!(
            serde_json::from_value::<UsageRow>(json!({})).unwrap(),
            UsageRow::default()
        );
        let row: UsageRow = serde_json::from_value(json!({
            "turns": 5, "reported": [1, 2, 5], "outcomes": ["failed"], "first_at": 7
        }))
        .unwrap();
        assert_eq!(row.turns, 5);
        assert_eq!(row.reported, vec![1, 2, 5]);
        assert_eq!(row.outcomes, vec!["failed".to_string()]);
    }
}
