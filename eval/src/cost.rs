//! A cycle's budget: every piece of work is reserved before it is
//! dispatched, and an unreported cost stays charged at its estimate.

use crate::contract::{CycleBudgetV1, CycleLedgerV1, CycleUsageV1, CycleWorkV1, WorkReservationV1};
use crate::error::EvalError;

pub fn new_ledger(cycle_id: &str, budget: CycleBudgetV1, started_at: i64) -> CycleLedgerV1 {
    CycleLedgerV1 {
        cycle_id: cycle_id.into(),
        budget,
        started_at,
        reservations: Default::default(),
    }
}

pub fn usage(ledger: &CycleLedgerV1, now: i64) -> CycleUsageV1 {
    let mut usage = CycleUsageV1 {
        budget: ledger.budget.clone(),
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

/// The limit the cycle has reached, if any: it can do no more work.
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
    Reserved(WorkReservationV1),
    /// Reserved before (redelivery, a concurrent caller, a restart): never
    /// dispatch it again.
    Existing(WorkReservationV1),
    /// It would pass a limit: the cycle ends with outcome `budget`.
    OverLimit(String),
}

/// Reserves `work` keyed by its `work_id` when it fits every limit. An
/// unusable estimate is an invalid request, not a budget limit.
pub fn reserve(
    ledger: &mut CycleLedgerV1,
    work: &CycleWorkV1,
    now: i64,
) -> Result<Reservation, EvalError> {
    if let Some(existing) = ledger.reservations.get(&work.work_id) {
        return Ok(Reservation::Existing(existing.clone()));
    }
    if !work.estimated_usd.is_finite() || work.estimated_usd < 0.0 {
        return Err(EvalError::InvalidRequest(format!(
            "work {} has no usable cost estimate",
            work.work_id
        )));
    }
    if let Some(reason) = exhausted(ledger, now) {
        return Ok(Reservation::OverLimit(reason));
    }
    let (usage, budget) = (usage(ledger, now), &ledger.budget);
    if work.candidate && usage.candidates >= budget.max_candidates {
        return Ok(Reservation::OverLimit(format!(
            "candidates: {} of {}",
            usage.candidates, budget.max_candidates
        )));
    }
    if usage.spend_usd + work.estimated_usd > budget.max_spend_usd {
        return Ok(Reservation::OverLimit(format!(
            "spend: {} USD reserved, {} more USD passes {} USD",
            usage.spend_usd, work.estimated_usd, budget.max_spend_usd
        )));
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
    Ok(Reservation::Reserved(reservation))
}

/// Settles a reservation once; `None` is an unknown cost, still charged at
/// the estimate. `false` when it was already settled.
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
