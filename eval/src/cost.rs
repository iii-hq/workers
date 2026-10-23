//! What the monitor's investigations cost: the day's known spend against the
//! optional cap, and what one analysis has usually cost.

use crate::contract::{
    AnalysisCostStatsV1, AnalysisRecordV1, CycleBudgetV1, CycleLedgerV1, CycleWorkV1, EvalStatusV1,
    MonitorConfigV1, MonitorCostV1, WorkReservationV1,
};
use crate::error::EvalError;
use crate::state::DailySpendV1;

const DAY_MS: i64 = 24 * 60 * 60 * 1_000;

/// Start of the UTC day containing `now`. The monitor has no timezone
/// setting, so its day is a UTC day.
pub fn day_start(now: i64) -> i64 {
    now - now.rem_euclid(DAY_MS)
}

/// `records` are every stored analysis and `spent` the persisted spend of a
/// day. Only the investigation's LLM cost is in
/// dollars; a missing cost is unknown, never zero: it adds nothing to the sums
/// and is counted apart.
///
/// The day's capture cost is the larger of the persisted capture spend, which
/// deleting an analysis cannot lower, and the stored analyses' sum. The cap
/// compares it and
/// counts each investigation still running as a median one, so a burst
/// admitted before any of them reports a cost cannot spend past it by their
/// number.
pub fn summarize(
    records: &[AnalysisRecordV1],
    config: Option<&MonitorConfigV1>,
    spent: Option<&DailySpendV1>,
    now: i64,
) -> MonitorCostV1 {
    let since = day_start(now);
    let (mut stored_usd, mut today_unknown) = (0.0, 0);
    for record in records.iter().filter(|record| record.created_at >= since) {
        match record.usage.llm_cost_usd {
            Some(cost) => stored_usd += cost,
            None if record.analyst.is_some() => today_unknown += 1,
            None => {}
        }
    }
    let spent = spent.filter(|spent| spent.since == since);
    let today_capture_usd = spent.map_or(stored_usd, |spent| spent.usd.max(stored_usd));
    let cap_usd = config.and_then(|config| config.daily_cost_cap_usd);

    let (mut known, mut unknown) = (Vec::new(), 0);
    for record in records.iter().filter(|record| {
        record.status == EvalStatusV1::Completed
            && record.analyst.is_some()
            && config.is_some_and(|config| {
                record.model.model == config.model.model
                    && record.model.provider == config.model.provider
                    && record.code_root.is_some() == config.code_repository.is_some()
            })
    }) {
        match record.usage.llm_cost_usd {
            Some(cost) => known.push(cost),
            None => unknown += 1,
        }
    }
    known.sort_by(f64::total_cmp);
    let middle = known.len() / 2;
    let median = match known.len() {
        0 => None,
        count if count % 2 == 1 => Some(known[middle]),
        _ => Some((known[middle - 1] + known[middle]) / 2.0),
    };
    let running = records
        .iter()
        .filter(|record| !record.status.is_terminal() && record.analyst.is_some())
        .count();
    let committed = median.map_or(0.0, |median| median * running as f64);
    MonitorCostV1 {
        since,
        today_usd: today_capture_usd,
        today_capture_usd,
        today_unknown,
        cap_usd,
        capped: cap_usd.is_some_and(|cap| today_capture_usd + committed >= cap),
        per_analysis: AnalysisCostStatsV1 {
            count: known.len() as u32,
            min: known.first().copied(),
            median,
            max: known.last().copied(),
            unknown,
        },
    }
}

// ---------------------------------------------------------------------------
// The loop's per-cycle budget
// ---------------------------------------------------------------------------

/// The reason code of a cycle that ends inconclusive on a limit.
pub const BUDGET_REASON: &str = "budget";

/// The limits a cycle gets when nothing sets others.
pub fn default_budget() -> CycleBudgetV1 {
    CycleBudgetV1 {
        max_spend_usd: 10.0,
        max_duration_ms: 2 * 60 * 60 * 1_000,
        max_calls: 100,
        max_candidates: 3,
    }
}

pub fn new_ledger(cycle_id: &str, budget: CycleBudgetV1, started_at: i64) -> CycleLedgerV1 {
    CycleLedgerV1 {
        cycle_id: cycle_id.into(),
        budget,
        started_at,
        reservations: Default::default(),
    }
}

/// What a cycle has drawn on its budget.
#[derive(Debug, Clone, PartialEq)]
pub struct CycleUsage {
    /// Reported costs, plus the reserved estimate of every work item that has
    /// not reported or reported no cost: an unknown cost is never zero.
    pub spend_usd: f64,
    /// Work settled without a cost. Any blocks a cost-reduction claim.
    pub unknown_cost: u32,
    pub calls: u32,
    pub candidates: u32,
    pub elapsed_ms: i64,
}

pub fn usage(ledger: &CycleLedgerV1, now: i64) -> CycleUsage {
    let mut usage = CycleUsage {
        spend_usd: 0.0,
        unknown_cost: 0,
        calls: 0,
        candidates: 0,
        elapsed_ms: (now - ledger.started_at).max(0),
    };
    for reservation in ledger.reservations.values() {
        usage.calls += 1;
        usage.candidates += u32::from(reservation.work.candidate);
        usage.spend_usd += reservation
            .cost_usd
            .unwrap_or(reservation.work.estimated_usd);
        if reservation.settled_at.is_some() && reservation.cost_usd.is_none() {
            usage.unknown_cost += 1;
        }
    }
    usage
}

/// The limit the cycle has reached, if any: the cycle can do no more work.
pub fn exhausted(ledger: &CycleLedgerV1, now: i64) -> Option<String> {
    let (usage, budget) = (usage(ledger, now), &ledger.budget);
    if usage.elapsed_ms >= budget.max_duration_ms {
        Some(format!(
            "time: {} ms of {} ms",
            usage.elapsed_ms, budget.max_duration_ms
        ))
    } else if usage.spend_usd >= budget.max_spend_usd {
        Some(format!(
            "spend: {} USD of {} USD",
            usage.spend_usd, budget.max_spend_usd
        ))
    } else if usage.calls >= budget.max_calls {
        Some(format!("calls: {} of {}", usage.calls, budget.max_calls))
    } else {
        None
    }
}

/// The outcome of a reservation. Only `Reserved` may be dispatched.
#[derive(Debug, Clone, PartialEq)]
pub enum Reservation {
    /// Newly reserved: dispatch it.
    Reserved(WorkReservationV1),
    /// Reserved before (a redelivery, a concurrent caller or a restart):
    /// never dispatch it again.
    Existing(WorkReservationV1),
    /// It would pass a limit: the cycle ends inconclusive with `budget`.
    OverLimit(String),
}

/// Reserves `work` in `ledger`, keyed by its `work_id`, when it fits every
/// limit with what is already reserved. The caller persists the ledger only
/// on `Reserved`, with compare-and-set.
pub fn reserve(ledger: &mut CycleLedgerV1, work: &CycleWorkV1, now: i64) -> Reservation {
    if let Some(existing) = ledger.reservations.get(&work.work_id) {
        return Reservation::Existing(existing.clone());
    }
    if !work.estimated_usd.is_finite() || work.estimated_usd < 0.0 {
        return Reservation::OverLimit(format!("spend: {} has no usable estimate", work.work_id));
    }
    if let Some(reason) = exhausted(ledger, now) {
        return Reservation::OverLimit(reason);
    }
    let (usage, budget) = (usage(ledger, now), &ledger.budget);
    if work.candidate && usage.candidates >= budget.max_candidates {
        return Reservation::OverLimit(format!(
            "candidates: {} of {}",
            usage.candidates, budget.max_candidates
        ));
    }
    if usage.spend_usd + work.estimated_usd > budget.max_spend_usd {
        return Reservation::OverLimit(format!(
            "spend: {} USD reserved, {} more USD passes {} USD",
            usage.spend_usd, work.estimated_usd, budget.max_spend_usd
        ));
    }
    let reservation = WorkReservationV1 {
        work: work.clone(),
        reserved_at: now,
        settled_at: None,
        cost_usd: None,
    };
    ledger
        .reservations
        .insert(work.work_id.clone(), reservation.clone());
    Reservation::Reserved(reservation)
}

/// Settles a reservation once with what the work reported; `None` is an
/// unknown cost, still charged at the estimate. Returns `false` when it was
/// already settled (nothing changes).
pub fn settle(
    ledger: &mut CycleLedgerV1,
    work_id: &str,
    cost_usd: Option<f64>,
    now: i64,
) -> Result<bool, EvalError> {
    let Some(reservation) = ledger.reservations.get_mut(work_id) else {
        return Err(EvalError::NotFound(format!(
            "cycle {} has no reservation {work_id}",
            ledger.cycle_id
        )));
    };
    if reservation.settled_at.is_some() {
        return Ok(false);
    }
    reservation.settled_at = Some(now);
    reservation.cost_usd = cost_usd.filter(|cost| cost.is_finite() && *cost >= 0.0);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const NOON: i64 = 5 * DAY_MS + 12 * 60 * 60 * 1_000;

    fn record(
        created_at: i64,
        status: &str,
        investigated: bool,
        cost: Option<f64>,
    ) -> AnalysisRecordV1 {
        let mut value = json!({
            "schema_version": 1, "evaluation_id": "e", "observation_key": "k",
            "origin": "automatic", "session_id": "s", "turn_id": "t",
            "model": {"model": "m", "provider": "p"},
            "config_revision": "r", "rules_version": "v", "criteria_version": "c",
            "status": status, "step": 0, "created_at": created_at, "updated_at": created_at,
            "deadline": created_at, "observe_since": created_at,
            "counters": {"sessions": 0, "entries": 0, "diagnostics": 0, "suggestions": 0,
                         "rejected_suggestions": 0},
            "stages": [], "usage": {"judge_calls": 0, "judge_input_tokens": 0,
                "judge_output_tokens": 0, "judge_usage_complete": true}
        });
        if investigated {
            value["analyst"] = json!({"session_id": "a", "sent_at": created_at});
        }
        if let Some(cost) = cost {
            value["usage"]["llm_cost_usd"] = json!(cost);
        }
        serde_json::from_value(value).unwrap()
    }

    fn config(cap: Option<f64>) -> MonitorConfigV1 {
        serde_json::from_value(json!({
            "enabled": true, "model": {"model": "m", "provider": "p"},
            "daily_cost_cap_usd": cap, "revision": "r", "updated_at": 0
        }))
        .unwrap()
    }

    #[test]
    fn the_day_starts_at_utc_midnight() {
        assert_eq!(day_start(NOON), 5 * DAY_MS);
        assert_eq!(day_start(5 * DAY_MS), 5 * DAY_MS);
        assert_eq!(day_start(5 * DAY_MS - 1), 4 * DAY_MS);
    }

    #[test]
    fn today_sums_known_cost_and_counts_the_unknown_apart() {
        let records = [
            record(NOON - 1_000, "completed", true, Some(0.25)),
            record(NOON - 2_000, "completed", true, Some(0.5)),
            // Investigated, no cost reported: unknown, not zero.
            record(NOON - 3_000, "failed", true, None),
            // Never investigated: nothing to report.
            record(NOON - 4_000, "completed", false, None),
            // Yesterday.
            record(5 * DAY_MS - 1, "completed", true, Some(9.0)),
        ];
        let cost = summarize(&records, Some(&config(Some(0.75))), None, NOON);
        assert_eq!(cost.since, 5 * DAY_MS);
        assert_eq!((cost.today_usd, cost.today_unknown), (0.75, 1));
        assert!(cost.capped, "reaching the cap is capped");
        assert!(!summarize(&records, Some(&config(Some(0.76))), None, NOON).capped);
        assert!(!summarize(&records, Some(&config(None)), None, NOON).capped);
        assert_eq!(summarize(&records, None, None, NOON).cap_usd, None);
    }

    #[test]
    fn one_analysis_costs_what_the_same_setup_cost_before() {
        let mut other_model = record(NOON, "completed", true, Some(100.0));
        other_model.model.model = "other".into();
        let mut with_code = record(NOON, "completed", true, Some(200.0));
        with_code.code_root = Some("/code".into());
        let records = [
            record(NOON, "completed", true, Some(0.3)),
            record(NOON, "completed", true, Some(0.1)),
            record(NOON, "completed", true, Some(0.2)),
            record(NOON, "completed", true, Some(0.4)),
            record(NOON, "completed", true, None),
            record(NOON, "completed", false, None),
            record(NOON, "failed", true, Some(50.0)),
            other_model,
            with_code,
        ];
        let stats = summarize(&records, Some(&config(None)), None, NOON).per_analysis;
        assert_eq!(stats.count, 4);
        assert_eq!(stats.unknown, 1);
        assert_eq!(
            (stats.min, stats.median, stats.max),
            (Some(0.1), Some(0.25), Some(0.4))
        );
        let odd = summarize(&records[..3], Some(&config(None)), None, NOON).per_analysis;
        assert_eq!(odd.median, Some(0.2));
        let none = summarize(&[], Some(&config(None)), None, NOON).per_analysis;
        assert_eq!(
            (none.count, none.min, none.median, none.max),
            (0, None, None, None)
        );
    }

    #[test]
    fn the_spend_outlives_the_analyses_that_made_it() {
        let records = [record(NOON - 1_000, "completed", true, Some(0.25))];
        let spent = |since, usd| DailySpendV1 { since, usd };
        let cap = Some(&config(Some(1.0)));
        // Two analyses were deleted: the stored one alone is far from the cap.
        let cost = summarize(&records, cap, Some(&spent(5 * DAY_MS, 1.1)), NOON);
        assert_eq!(cost.today_usd, 1.1);
        assert!(cost.capped);
        // Yesterday's spend is not today's; the stored analyses still count.
        let stale = summarize(&records, cap, Some(&spent(4 * DAY_MS, 9.0)), NOON);
        assert_eq!((stale.today_usd, stale.capped), (0.25, false));
        // Spend that was never persisted (older records) is not lost.
        let floor = summarize(&records, cap, Some(&spent(5 * DAY_MS, 0.1)), NOON);
        assert_eq!(floor.today_usd, 0.25);
    }

    #[test]
    fn a_day_stored_before_the_buckets_counts_as_capture() {
        let legacy: DailySpendV1 =
            serde_json::from_value(json!({"since": 5 * DAY_MS, "usd": 1.1})).unwrap();
        let cost = summarize(&[], Some(&config(Some(1.0))), Some(&legacy), NOON);
        assert_eq!(cost.today_capture_usd, 1.1);
        assert!(cost.capped);
    }

    #[test]
    fn an_investigation_still_running_counts_as_a_median_one() {
        let mut records = vec![
            record(NOON - 4_000, "completed", true, Some(0.3)),
            record(NOON - 3_000, "completed", true, Some(0.5)),
        ];
        // Median 0.4: two running investigations hold 0.8 of a 1.0 cap.
        records.push(record(NOON - 2_000, "investigating", true, None));
        records.push(record(NOON - 1_000, "investigating", true, None));
        // Collected but not investigating yet: it may never be.
        records.push(record(NOON, "collecting", false, None));
        let cost = summarize(&records, Some(&config(Some(1.5))), None, NOON);
        assert!((cost.today_usd - 0.8).abs() < 1e-9);
        assert!(cost.capped, "0.8 spent and 0.8 in flight reach 1.5");
        let roomy = summarize(&records, Some(&config(Some(1.7))), None, NOON);
        assert!(!roomy.capped);
        // With no history there is nothing to estimate by.
        let blind = summarize(&records[2..], Some(&config(Some(0.01))), None, NOON);
        assert!(!blind.capped);
    }
}
