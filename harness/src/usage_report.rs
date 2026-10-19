//! Anonymous usage reporting for the turn loop.
//!
//! One durable publish on the `harness:usage` topic per reported moment, which
//! the engine's `telemetry` worker subscribes to and forwards as a single
//! product event (see `engine/src/workers/telemetry/harness.rs`). The harness
//! never talks to an analytics vendor itself: it announces, and an engine that
//! the operator opted out of telemetry simply never listens.
//!
//! Three reported moments:
//!
//! * `harness_user_turns`: user turns (root turns, one user message through
//!   its final reply) counted across EVERY session in this state store (the
//!   project, with the default file-backed `state`), reported
//!   when the count reaches 1, 2, 5, 10, 25 and 50, then every 50.
//! * `harness_model_calls`: model calls (assistant messages, sub-agents
//!   included) counted across every session in the same state store, with the tool
//!   call, token and cost totals that go with them, reported on the same
//!   milestones of the model-call count.
//! * `harness_turn_failed` once per session per outcome (`failed`,
//!   `cancelled`, `max_turns`), which keeps the reason a run stopped exact.
//!
//! Both counts are cumulative, so a machine's latest report supersedes the
//! ones before it, and each carries a `*_delta` since the previous successful
//! report of its kind, so the deltas of one machine sum to its total.
//!
//! Model calls are folded in when a ROOT turn ends, from `harness::metrics`
//! over its whole session tree:
//!
//! * A sub-agent's calls are in its root's tree and are counted when the root
//!   turn that waited for it ends.
//! * A spawn with a display-only parent (`parent_session_id`, no live parent
//!   turn) is in that parent's tree but is not a root turn. Its calls are
//!   counted at the parent's NEXT root turn, and never if the parent has no
//!   further turn.
//! * A spawn with no parent of either kind is a root session of its own, so
//!   its turn counts as a user turn.
//! * A session the per-session milestone version ran (its row has no
//!   `folded`) takes its usage so far as a baseline on its first turn here:
//!   that usage is not counted again, and the session joins `sessions_total`.
//!
//! The milestone `dedupe_key`s carry the namespace, so two projects on one
//! engine never share one.
//!
//! This module only READS the harness's own accounting: model usage comes
//! from `harness::metrics` over the session tree, and what this module keeps
//! is its own reporting bookkeeping under `harness_usage`: one row per session
//! (outcomes announced, model usage already folded in) and one machine-wide
//! row (`_machine`) holding the counters, updated by compare-and-set so
//! sessions finishing turns at once never lose or double an increment.
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
use crate::functions::metrics::{self, SessionMetricsRequestV1, SessionUsageTotalsV1};
use crate::state::{self, USAGE_SCOPE};
use crate::types::message::AgentMessage;
use crate::types::turn::TurnRecord;

/// Topic the engine's telemetry worker subscribes to.
pub const USAGE_TOPIC: &str = "harness:usage";
const PUBLISH_ID: &str = "iii::durable::publish";
const STATE_CAS_ID: &str = "state::compare-and-set";

pub const USER_TURNS_EVENT: &str = "harness_user_turns";
pub const MODEL_CALLS_EVENT: &str = "harness_model_calls";
pub const FAILED_EVENT: &str = "harness_turn_failed";

/// Key of the machine-wide row in [`USAGE_SCOPE`]. Session ids never start
/// with an underscore, so it cannot collide with a session's row.
const MACHINE_KEY: &str = "_machine";

/// Compare-and-set attempts on the machine row before a fold gives up.
const CAS_ATTEMPTS: usize = 8;

/// Counts that report. Chosen to be dense where usage dies and sparse
/// afterwards: the gap between 1 and 2 is the one that says whether the
/// harness was tried once or actually used. Past the last one, every
/// [`REPEAT_EVERY`] reports.
pub const MILESTONES: [u64; 6] = [1, 2, 5, 10, 25, 50];

/// After the last listed milestone, every multiple of this reports.
pub const REPEAT_EVERY: u64 = 50;

/// The milestone a count moving from `prev` to `next` reaches, if any: the
/// highest one in `(prev, next]`. User turns move by one, so they hit every
/// milestone; model calls move by several at once and report the highest one
/// they passed, with their real totals.
pub fn crossed(prev: u64, next: u64) -> Option<u64> {
    if next <= prev {
        return None;
    }
    let last = MILESTONES[MILESTONES.len() - 1];
    let repeat = next - next % REPEAT_EVERY;
    if repeat > last && repeat > prev {
        return Some(repeat);
    }
    MILESTONES
        .iter()
        .rev()
        .copied()
        .find(|m| *m > prev && *m <= next)
}

/// Model usage counters, as `harness::metrics` reports them for a session
/// tree.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ModelUsage {
    #[serde(default)]
    pub model_calls: u64,
    #[serde(default)]
    pub tool_calls: u64,
    #[serde(default)]
    pub tool_call_errors: u64,
    #[serde(default)]
    pub input_tokens: u64,
    #[serde(default)]
    pub output_tokens: u64,
    #[serde(default)]
    pub cache_read_tokens: u64,
    #[serde(default)]
    pub cache_write_tokens: u64,
    #[serde(default)]
    pub cost_usd: f64,
}

impl ModelUsage {
    /// A session tree's totals. A counter the provider left unreported keeps
    /// the value `prev` already folded in, so a gap is neither counted as zero
    /// now nor counted twice when it reports again.
    fn observed(totals: &SessionUsageTotalsV1, prev: &ModelUsage) -> Self {
        Self {
            model_calls: totals.turns,
            tool_calls: totals.function_calls,
            tool_call_errors: totals.function_call_errors,
            input_tokens: totals.input_tokens.unwrap_or(prev.input_tokens),
            output_tokens: totals.output_tokens.unwrap_or(prev.output_tokens),
            cache_read_tokens: totals.cache_read_tokens.unwrap_or(prev.cache_read_tokens),
            cache_write_tokens: totals.cache_write_tokens.unwrap_or(prev.cache_write_tokens),
            cost_usd: totals.cost_usd.unwrap_or(prev.cost_usd),
        }
    }

    /// What this has beyond `earlier`; never negative.
    fn since(&self, earlier: &ModelUsage) -> Self {
        Self {
            model_calls: self.model_calls.saturating_sub(earlier.model_calls),
            tool_calls: self.tool_calls.saturating_sub(earlier.tool_calls),
            tool_call_errors: self
                .tool_call_errors
                .saturating_sub(earlier.tool_call_errors),
            input_tokens: self.input_tokens.saturating_sub(earlier.input_tokens),
            output_tokens: self.output_tokens.saturating_sub(earlier.output_tokens),
            cache_read_tokens: self
                .cache_read_tokens
                .saturating_sub(earlier.cache_read_tokens),
            cache_write_tokens: self
                .cache_write_tokens
                .saturating_sub(earlier.cache_write_tokens),
            cost_usd: (self.cost_usd - earlier.cost_usd).max(0.0),
        }
    }

    fn plus(&self, other: &ModelUsage) -> Self {
        Self {
            model_calls: self.model_calls + other.model_calls,
            tool_calls: self.tool_calls + other.tool_calls,
            tool_call_errors: self.tool_call_errors + other.tool_call_errors,
            input_tokens: self.input_tokens + other.input_tokens,
            output_tokens: self.output_tokens + other.output_tokens,
            cache_read_tokens: self.cache_read_tokens + other.cache_read_tokens,
            cache_write_tokens: self.cache_write_tokens + other.cache_write_tokens,
            cost_usd: self.cost_usd + other.cost_usd,
        }
    }
}

/// Per-session reporting bookkeeping (`harness_usage/<session_id>`).
///
/// Durable rather than in memory because it is what makes a report happen
/// once: a restarted harness or a redelivered turn step must not re-announce
/// an outcome, or fold the same model usage into the machine twice.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct UsageRow {
    /// Root turns this session has completed, terminal or not.
    #[serde(default)]
    pub turns: u64,
    /// Outcomes already announced, one announcement each.
    #[serde(default)]
    pub outcomes: Vec<String>,
    /// When this session's first reported turn ended; zero until then.
    #[serde(default)]
    pub first_at: i64,
    /// This session tree's model usage already folded into the machine row.
    /// `None` on a row the per-session milestone version wrote: that session
    /// has usage the machine row never saw, and its first turn here takes it
    /// as the baseline instead of counting it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folded: Option<ModelUsage>,
}

impl UsageRow {
    /// Written by the per-session milestone version: it has reported turns
    /// but no `folded`.
    fn is_legacy(&self) -> bool {
        self.folded.is_none() && self.first_at != 0
    }
}

/// The machine-wide counters (`harness_usage/_machine`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MachineRow {
    /// User (root) turns across every session.
    #[serde(default)]
    pub user_turns: u64,
    /// Sessions that completed at least one root turn.
    #[serde(default)]
    pub sessions: u64,
    /// Model usage across every session.
    #[serde(default)]
    pub usage: ModelUsage,
    /// `user_turns` as of the last `harness_user_turns` that was published.
    #[serde(default)]
    pub user_turns_announced: u64,
    /// `usage.model_calls` as of the last `harness_model_calls` published.
    #[serde(default)]
    pub model_calls_announced: u64,
}

/// What one finished root turn adds to the machine row.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Fold {
    pub user_turns: u64,
    pub sessions: u64,
    pub usage: ModelUsage,
}

impl MachineRow {
    fn fold(&self, add: &Fold) -> Self {
        Self {
            user_turns: self.user_turns + add.user_turns,
            sessions: self.sessions + add.sessions,
            usage: self.usage.plus(&add.usage),
            ..self.clone()
        }
    }
}

/// What one finished root turn adds to the machine row. A legacy session
/// (see [`UsageRow::is_legacy`]) joins the machine count on the first turn
/// whose usage can be read: that usage becomes its baseline and adds nothing,
/// because the user turns behind it were never counted here and the old
/// per-session events already reported it.
fn fold_for(
    first_turn: bool,
    legacy: bool,
    observed: Option<&ModelUsage>,
    prev_folded: &ModelUsage,
) -> Fold {
    let usage = match (legacy, observed) {
        (false, Some(now)) => now.since(prev_folded),
        _ => ModelUsage::default(),
    };
    Fold {
        user_turns: 1,
        sessions: u64::from(first_turn || (legacy && observed.is_some())),
        usage,
    }
}

/// Whether this turn belongs to a root session. A sub-agent turn reports
/// nothing of its own, and neither does a turn a reaction delivered into a
/// session on a parent's behalf: it has no `parent` link, but its
/// `display_parent_session_id` says whose tree it belongs to.
fn is_root(record: &TurnRecord) -> bool {
    record.depth == 0 && record.parent.is_none() && record.display_parent_session_id.is_none()
}

/// Report one finished root turn: its outcome if it was not an ordinary
/// completion, then the machine-wide user-turn and model-call milestones it
/// reached, if any.
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
    let first_turn = row.first_at == 0;
    let legacy = row.is_legacy();
    if !legacy {
        // Nothing folded yet is a real zero for a session this version
        // started, so a failed fold below still leaves it to the next turn.
        row.folded.get_or_insert_with(ModelUsage::default);
    }
    let prev_folded = row.folded.clone().unwrap_or_default();
    row.turns += 1;
    if first_turn {
        row.first_at = AgentMessage::now_ms();
    }

    if outcome != "completed" && !row.outcomes.iter().any(|o| o == outcome) {
        let payload = failed_payload(record, outcome, error_kind, row.turns);
        if publish(deps, payload, timeout_ms).await {
            row.outcomes.push(outcome.to_string());
        }
    }

    // ponytail: `harness::metrics` re-reads the session tree's usage on every
    // root turn so the machine row can take its delta. It reads the usage
    // projection only (no context, no traces); if it ever shows up in a
    // turn's latency, fold per-step usage instead.
    let observed = match metrics::handle(
        deps,
        SessionMetricsRequestV1 {
            root_session_id: session_id.clone(),
            omit_context: true,
            omit_traces: true,
            subtree: false,
        },
    )
    .await
    {
        Ok(m) => Some(ModelUsage::observed(&m.totals, &prev_folded)),
        Err(e) => {
            tracing::debug!(session_id = %session_id, error = %e, "usage metrics unavailable");
            None
        }
    };
    let add = fold_for(first_turn, legacy, observed.as_ref(), &prev_folded);

    match update_machine(deps, timeout_ms, |m| m.fold(&add)).await {
        Ok((before, after)) => {
            if observed.is_some() {
                row.folded = observed;
            }
            announce(deps, record, &before, &after, timeout_ms).await;
        }
        Err(e) => {
            // The session row keeps its old `folded`, so the next turn folds
            // this turn's model usage in; this user turn is not counted.
            if first_turn {
                // Not counted as a session yet: the next fold that lands
                // counts it.
                row.first_at = 0;
            }
            tracing::debug!(error = %e, "machine usage row not updated");
        }
    }

    if let Err(e) = save(deps, session_id, &row, timeout_ms).await {
        tracing::debug!(session_id = %session_id, error = %e, "usage row unwritable");
    }
}

/// Publish the milestones a fold reached, and record each one published so
/// the next report's delta starts from it. The fold that reached a milestone
/// is the only one that sees it in `(before, after]`, so concurrent turns
/// never announce the same milestone twice.
async fn announce(
    deps: &Deps,
    record: &TurnRecord,
    before: &MachineRow,
    after: &MachineRow,
    timeout_ms: u64,
) {
    let ns = deps
        .iii
        .namespace()
        .unwrap_or_else(|| "default".to_string());
    if let Some(reached) = crossed(before.user_turns, after.user_turns) {
        if publish(
            deps,
            user_turns_payload(record, &ns, reached, after),
            timeout_ms,
        )
        .await
        {
            let total = after.user_turns;
            let marked = update_machine(deps, timeout_ms, |m| MachineRow {
                user_turns_announced: m.user_turns_announced.max(total),
                ..m.clone()
            })
            .await;
            if let Err(e) = marked {
                tracing::debug!(error = %e, "user-turn report not recorded");
            }
        }
    }
    if let Some(reached) = crossed(before.usage.model_calls, after.usage.model_calls) {
        if publish(
            deps,
            model_calls_payload(record, &ns, reached, after),
            timeout_ms,
        )
        .await
        {
            let total = after.usage.model_calls;
            let marked = update_machine(deps, timeout_ms, |m| MachineRow {
                model_calls_announced: m.model_calls_announced.max(total),
                ..m.clone()
            })
            .await;
            if let Err(e) = marked {
                tracing::debug!(error = %e, "model-call report not recorded");
            }
        }
    }
}

fn with_provider(mut payload: Value, record: &TurnRecord) -> Value {
    if let Some(provider) = &record.options.provider {
        payload["provider"] = Value::String(provider.clone());
    }
    payload
}

fn failed_payload(
    record: &TurnRecord,
    outcome: &str,
    error_kind: Option<&str>,
    turn_index: u64,
) -> Value {
    let mut payload = json!({
        "event": FAILED_EVENT,
        "dedupe_key": format!("{}:{outcome}", record.session_id),
        "outcome": outcome,
        "turn_index": turn_index,
        "model": record.options.model,
    });
    if let Some(kind) = error_kind {
        payload["error_kind"] = Value::String(kind.to_string());
    }
    with_provider(payload, record)
}

/// `harness_user_turns`: the machine's user turns. `user_turns_delta` is the
/// user turns since the last one of these that was published, so summing it
/// over a machine's events gives its `user_turns_total`.
fn user_turns_payload(record: &TurnRecord, ns: &str, reached: u64, m: &MachineRow) -> Value {
    with_provider(
        json!({
            "event": USER_TURNS_EVENT,
            "dedupe_key": format!("{ns}:user_turns:{reached}"),
            "milestone": reached,
            "user_turns_total": m.user_turns,
            "user_turns_delta": m.user_turns.saturating_sub(m.user_turns_announced),
            "sessions_total": m.sessions,
            "model": record.options.model,
        }),
        record,
    )
}

/// `harness_model_calls`: the machine's model calls and everything that goes
/// with them. `model_calls_delta` sums to `model_calls_total` the same way.
fn model_calls_payload(record: &TurnRecord, ns: &str, reached: u64, m: &MachineRow) -> Value {
    let u = &m.usage;
    with_provider(
        json!({
            "event": MODEL_CALLS_EVENT,
            "dedupe_key": format!("{ns}:model_calls:{reached}"),
            "milestone": reached,
            "model_calls_total": u.model_calls,
            "model_calls_delta": u.model_calls.saturating_sub(m.model_calls_announced),
            "tool_calls_total": u.tool_calls,
            "tool_call_errors_total": u.tool_call_errors,
            "input_tokens": u.input_tokens,
            "output_tokens": u.output_tokens,
            "cache_read_tokens": u.cache_read_tokens,
            "cache_write_tokens": u.cache_write_tokens,
            "cost_usd": u.cost_usd,
            "user_turns_total": m.user_turns,
            "sessions_total": m.sessions,
            "model": record.options.model,
        }),
        record,
    )
}

/// Apply `next` to the machine row by compare-and-set, retrying when another
/// turn moved it first. Returns the row before and after this update.
async fn update_machine(
    deps: &Deps,
    timeout_ms: u64,
    next: impl Fn(&MachineRow) -> MachineRow,
) -> Result<(MachineRow, MachineRow), crate::error::HarnessError> {
    let mut current = state::state_get(&deps.iii, USAGE_SCOPE, MACHINE_KEY, timeout_ms).await?;
    for _ in 0..CAS_ATTEMPTS {
        let before = machine_row(&current);
        let after = next(&before);
        let value = serde_json::to_value(&after).map_err(|e| {
            crate::error::HarnessError::State(format!("machine usage row serialize: {e}"))
        })?;
        let expected = (!current.is_null()).then(|| current.clone());
        match cas_machine(deps, expected, value, timeout_ms).await? {
            None => return Ok((before, after)),
            Some(moved) => current = moved,
        }
    }
    Err(crate::error::HarnessError::State(
        "machine usage row contended".to_string(),
    ))
}

/// One compare-and-set on the machine row through the public
/// `state::compare-and-set` (`harness_usage` is a public scope, unlike the
/// binding scopes the harness's private accessor serves). `Ok(None)` means it
/// swapped; `Ok(Some(current))` is what another turn stored first.
async fn cas_machine(
    deps: &Deps,
    expected: Option<Value>,
    value: Value,
    timeout_ms: u64,
) -> Result<Option<Value>, crate::error::HarnessError> {
    let mut payload = json!({ "scope": USAGE_SCOPE, "key": MACHINE_KEY, "value": value });
    if let Some(expected) = expected {
        payload["expected"] = expected;
    }
    let resp = deps
        .iii
        .trigger(TriggerRequest {
            function_id: STATE_CAS_ID.into(),
            payload,
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
        .map_err(|e| {
            crate::error::HarnessError::State(format!(
                "{STATE_CAS_ID} {USAGE_SCOPE}/{MACHINE_KEY}: {e}"
            ))
        })?;
    if resp.get("swapped").and_then(Value::as_bool) == Some(true) {
        Ok(None)
    } else {
        Ok(Some(resp.get("current").cloned().unwrap_or(Value::Null)))
    }
}

fn machine_row(value: &Value) -> MachineRow {
    if value.is_null() {
        return MachineRow::default();
    }
    serde_json::from_value(value.clone()).unwrap_or_default()
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

/// Drop a deleted session's row. The machine row keeps what it counted.
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

    fn usage(model_calls: u64, input: u64, cost: f64) -> ModelUsage {
        ModelUsage {
            model_calls,
            input_tokens: input,
            cost_usd: cost,
            ..Default::default()
        }
    }

    /// Prevents: a user-turn count that steps by one skipping or repeating a
    /// milestone, or going silent after 50.
    #[test]
    fn counting_one_at_a_time_hits_every_milestone_then_every_fifty() {
        let hit: Vec<u64> = (1..=200u64).filter_map(|n| crossed(n - 1, n)).collect();
        assert_eq!(hit, vec![1, 2, 5, 10, 25, 50, 100, 150, 200]);
    }

    /// Prevents: a model-call count that jumps past a milestone never
    /// reporting it.
    #[test]
    fn a_jump_reports_the_highest_milestone_it_passed() {
        assert_eq!(crossed(0, 12), Some(10));
        assert_eq!(crossed(10, 12), None);
        assert_eq!(crossed(48, 103), Some(100));
        assert_eq!(crossed(50, 99), None);
        assert_eq!(crossed(99, 100), Some(100));
        assert_eq!(crossed(5, 5), None);
        assert_eq!(crossed(7, 3), None);
    }

    /// Prevents: a counter the provider left unreported folding in as zero,
    /// then the whole value again when it reports.
    #[test]
    fn an_unreported_counter_keeps_what_was_folded() {
        let prev = usage(3, 100, 0.5);
        let totals = SessionUsageTotalsV1 {
            turns: 5,
            input_tokens: None,
            cost_usd: Some(0.75),
            ..Default::default()
        };
        let now = ModelUsage::observed(&totals, &prev);
        assert_eq!(now.input_tokens, 100);
        let delta = now.since(&prev);
        assert_eq!(delta.model_calls, 2);
        assert_eq!(delta.input_tokens, 0);
        assert!((delta.cost_usd - 0.25).abs() < 1e-9);
    }

    #[test]
    fn a_shrunk_session_never_takes_from_the_machine() {
        let delta = usage(2, 10, 0.1).since(&usage(5, 50, 0.4));
        assert_eq!(delta, ModelUsage::default());
    }

    /// Prevents: sessions overwriting each other's counts instead of adding.
    #[test]
    fn turns_from_different_sessions_add_up_on_the_machine() {
        let mut m = MachineRow::default();
        for (session_usage, first) in [
            (usage(3, 30, 0.3), true),
            (usage(4, 40, 0.4), true),
            (usage(2, 20, 0.2), false),
        ] {
            m = m.fold(&Fold {
                user_turns: 1,
                sessions: u64::from(first),
                usage: session_usage,
            });
        }
        assert_eq!(m.user_turns, 3);
        assert_eq!(m.sessions, 2);
        assert_eq!(m.usage.model_calls, 9);
        assert_eq!(m.usage.input_tokens, 90);
        assert_eq!(m.user_turns_announced, 0);
    }

    /// Prevents: per-machine deltas that do not sum to the total, including
    /// after a report whose publish failed.
    #[test]
    fn user_turn_deltas_sum_to_the_total() {
        let record = crate::types::turn::tests::record();
        let mut m = MachineRow::default();
        let mut sum = 0;
        for _ in 1..=200u64 {
            let before = m.clone();
            m = m.fold(&Fold {
                user_turns: 1,
                ..Default::default()
            });
            if let Some(reached) = crossed(before.user_turns, m.user_turns) {
                // Milestone 10's publish fails: never recorded.
                if reached == 10 {
                    continue;
                }
                let payload = user_turns_payload(&record, "default", reached, &m);
                sum += payload["user_turns_delta"].as_u64().unwrap();
                assert_eq!(sum, m.user_turns);
                m.user_turns_announced = m.user_turns;
            }
        }
        assert_eq!(sum, 200);
    }

    #[test]
    fn the_user_turns_event_carries_machine_counts() {
        let record = crate::types::turn::tests::record();
        let m = MachineRow {
            user_turns: 100,
            sessions: 7,
            user_turns_announced: 50,
            ..Default::default()
        };
        let p = user_turns_payload(&record, "default", 100, &m);
        assert_eq!(p["event"], USER_TURNS_EVENT);
        assert_eq!(p["dedupe_key"], "default:user_turns:100");
        assert_eq!(p["milestone"], 100);
        assert_eq!(p["user_turns_total"], 100);
        assert_eq!(p["user_turns_delta"], 50);
        assert_eq!(p["sessions_total"], 7);
        assert!(p.get("model_calls_total").is_none());
    }

    #[test]
    fn the_model_calls_event_carries_everything_for_model_calls() {
        let record = crate::types::turn::tests::record();
        let m = MachineRow {
            user_turns: 12,
            sessions: 3,
            usage: ModelUsage {
                model_calls: 103,
                tool_calls: 80,
                tool_call_errors: 4,
                input_tokens: 9_000,
                output_tokens: 700,
                cache_read_tokens: 50,
                cache_write_tokens: 5,
                cost_usd: 1.5,
            },
            model_calls_announced: 52,
            ..Default::default()
        };
        let p = model_calls_payload(&record, "default", 100, &m);
        assert_eq!(p["event"], MODEL_CALLS_EVENT);
        assert_eq!(p["dedupe_key"], "default:model_calls:100");
        assert_eq!(p["milestone"], 100);
        assert_eq!(p["model_calls_total"], 103);
        assert_eq!(p["model_calls_delta"], 51);
        assert_eq!(p["tool_calls_total"], 80);
        assert_eq!(p["tool_call_errors_total"], 4);
        assert_eq!(p["input_tokens"], 9_000);
        assert_eq!(p["cost_usd"], 1.5);
        assert_eq!(p["user_turns_total"], 12);
        assert!(p.get("turns_total").is_none());
    }

    #[test]
    fn a_failure_still_reports_once_per_session_outcome() {
        let record = crate::types::turn::tests::record();
        let p = failed_payload(&record, "failed", Some("llm.rate_limited"), 4);
        assert_eq!(p["event"], FAILED_EVENT);
        assert_eq!(p["dedupe_key"], format!("{}:failed", record.session_id));
        assert_eq!(p["turn_index"], 4);
        assert_eq!(p["error_kind"], "llm.rate_limited");
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

    /// Prevents: rows written by the per-session milestone version failing to
    /// load (they carry `reported`, and no `folded`).
    #[test]
    fn rows_from_the_previous_layout_and_malformed_rows_load() {
        let row: UsageRow = serde_json::from_value(json!({
            "turns": 5, "reported": [1, 2, 5], "outcomes": ["failed"], "first_at": 7
        }))
        .unwrap();
        assert_eq!(row.turns, 5);
        assert_eq!(row.outcomes, vec!["failed".to_string()]);
        assert_eq!(row.folded, None);
        assert!(row.is_legacy());
        assert!(!UsageRow::default().is_legacy());
        assert_eq!(machine_row(&Value::Null), MachineRow::default());
        assert_eq!(machine_row(&json!("garbage")), MachineRow::default());
    }

    /// Prevents: a session the old version ran folding its whole history
    /// into the machine model calls on its first turn after the upgrade,
    /// while its earlier user turns and the session itself go uncounted.
    #[test]
    fn a_legacy_session_takes_its_history_as_the_baseline() {
        let history = usage(4, 700, 0.2);
        let add = fold_for(false, true, Some(&history), &ModelUsage::default());
        assert_eq!(add.user_turns, 1);
        assert_eq!(add.sessions, 1);
        assert_eq!(add.usage, ModelUsage::default());

        // Usage unreadable: count the turn, keep the session for the next one.
        let add = fold_for(false, true, None, &ModelUsage::default());
        assert_eq!((add.user_turns, add.sessions), (1, 0));
    }

    #[test]
    fn a_new_session_folds_its_usage_and_counts_once() {
        let first = fold_for(
            true,
            false,
            Some(&usage(2, 50, 0.1)),
            &ModelUsage::default(),
        );
        assert_eq!(first.sessions, 1);
        assert_eq!(first.usage.model_calls, 2);

        let next = fold_for(false, false, Some(&usage(5, 80, 0.3)), &usage(2, 50, 0.1));
        assert_eq!(next.sessions, 0);
        assert_eq!(next.usage.model_calls, 3);
        assert_eq!(next.usage.input_tokens, 30);
    }

    /// Prevents: two projects on one engine (one namespace each) sharing a
    /// milestone key, so the engine drops the second project's report.
    #[test]
    fn milestone_keys_carry_the_namespace() {
        let record = crate::types::turn::tests::record();
        let m = MachineRow {
            user_turns: 1,
            usage: usage(1, 0, 0.0),
            ..Default::default()
        };
        assert_ne!(
            user_turns_payload(&record, "a", 1, &m)["dedupe_key"],
            user_turns_payload(&record, "b", 1, &m)["dedupe_key"]
        );
        assert_eq!(
            model_calls_payload(&record, "b", 1, &m)["dedupe_key"],
            "b:model_calls:1"
        );
    }
}
