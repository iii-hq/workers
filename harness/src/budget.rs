//! Durable hard-budget ledger shared by a root harness session and its
//! in-turn sub-agents.
//!
//! Each generation reserves its worst-case input/output usage before calling
//! the router, then reconciles the reservation with the provider's actual
//! usage. The existing per-key in-process lock serializes sibling sessions
//! against the same root ledger; the state worker makes the counters durable.

use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::error::HarnessError;
use crate::types::event::Usage;
use crate::types::model::Pricing;
use crate::types::turn::{TurnOptions, TurnRecord};

const BUDGET_SCOPE: &str = "harness_budget";
const MICRO_USD_PER_USD: f64 = 1_000_000.0;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct BudgetState {
    root_session_id: String,
    max_total_tokens: Option<u64>,
    used_tokens: u64,
    reserved_tokens: u64,
    max_cost_microusd: Option<u64>,
    used_cost_microusd: u64,
    reserved_cost_microusd: u64,
}

impl BudgetState {
    fn new(root_session_id: &str, options: &TurnOptions) -> Result<Self, HarnessError> {
        Ok(Self {
            root_session_id: root_session_id.to_string(),
            max_total_tokens: options.max_total_tokens,
            used_tokens: 0,
            reserved_tokens: 0,
            max_cost_microusd: options.max_cost_usd.map(cost_limit_microusd).transpose()?,
            used_cost_microusd: 0,
            reserved_cost_microusd: 0,
        })
    }

    fn verify_limits(&self, options: &TurnOptions) -> Result<(), HarnessError> {
        let requested_cost = options.max_cost_usd.map(cost_limit_microusd).transpose()?;
        if self.max_total_tokens != options.max_total_tokens
            || self.max_cost_microusd != requested_cost
        {
            return Err(HarnessError::InvalidRequest(
                "execution budgets are frozen when a root session starts and cannot be changed"
                    .into(),
            ));
        }
        Ok(())
    }

    fn reserve(
        &mut self,
        requested_tokens: u64,
        requested_cost_microusd: u64,
    ) -> Result<(), BudgetRejection> {
        if let Some(limit) = self.max_total_tokens {
            let committed = self.used_tokens.saturating_add(self.reserved_tokens);
            if committed.saturating_add(requested_tokens) > limit {
                return Err(BudgetRejection::Exceeded(format!(
                    "generation requires up to {requested_tokens} tokens, but only {} remain in the shared {limit}-token budget",
                    limit.saturating_sub(committed)
                )));
            }
        }
        if let Some(limit) = self.max_cost_microusd {
            let committed = self
                .used_cost_microusd
                .saturating_add(self.reserved_cost_microusd);
            if committed.saturating_add(requested_cost_microusd) > limit {
                return Err(BudgetRejection::Exceeded(format!(
                    "generation requires up to {}, but only {} remain in the shared {} budget",
                    display_microusd(requested_cost_microusd),
                    display_microusd(limit.saturating_sub(committed)),
                    display_microusd(limit)
                )));
            }
        }
        self.reserved_tokens = self.reserved_tokens.saturating_add(requested_tokens);
        self.reserved_cost_microusd = self
            .reserved_cost_microusd
            .saturating_add(requested_cost_microusd);
        Ok(())
    }

    fn reconcile(
        &mut self,
        reservation: &BudgetReservation,
        actual_tokens: u64,
        actual_cost_microusd: u64,
    ) {
        self.reserved_tokens = self
            .reserved_tokens
            .saturating_sub(reservation.reserved_tokens);
        self.reserved_cost_microusd = self
            .reserved_cost_microusd
            .saturating_sub(reservation.reserved_cost_microusd);
        self.used_tokens = self.used_tokens.saturating_add(actual_tokens);
        self.used_cost_microusd = self.used_cost_microusd.saturating_add(actual_cost_microusd);
    }

    fn release(&mut self, reservation: &BudgetReservation) {
        self.reserved_tokens = self
            .reserved_tokens
            .saturating_sub(reservation.reserved_tokens);
        self.reserved_cost_microusd = self
            .reserved_cost_microusd
            .saturating_sub(reservation.reserved_cost_microusd);
    }
}

#[derive(Debug, Clone)]
pub struct BudgetReservation {
    root_session_id: String,
    reserved_tokens: u64,
    reserved_cost_microusd: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BudgetRejection {
    Exceeded(String),
    Unavailable(String),
}

impl BudgetRejection {
    pub fn reason(&self) -> &str {
        match self {
            Self::Exceeded(reason) | Self::Unavailable(reason) => reason,
        }
    }
}

#[derive(Debug, Clone)]
pub enum ReserveOutcome {
    Unlimited,
    Reserved(BudgetReservation),
    Rejected(BudgetRejection),
}

/// Validate and freeze a root session's requested budget. Existing sessions
/// inherit their first turn's ledger; they cannot acquire a budget after
/// unmetered work has already happened or replace one mid-session.
pub async fn prepare_root(
    deps: &Deps,
    session_id: &str,
    options: &mut TurnOptions,
    previous: Option<&TurnRecord>,
) -> Result<(), HarnessError> {
    validate_options(options)?;

    if let Some(previous) = previous {
        let previous_budgeted = previous.options.budget_root_session_id.is_some();
        let requested_budgeted =
            options.max_total_tokens.is_some() || options.max_cost_usd.is_some();

        if !previous_budgeted {
            if requested_budgeted {
                return Err(HarnessError::InvalidRequest(
                    "cannot add an execution budget to an existing unmetered session; start a new session"
                        .into(),
                ));
            }
            return Ok(());
        }

        if options
            .max_total_tokens
            .is_some_and(|value| Some(value) != previous.options.max_total_tokens)
            || options.max_cost_usd.is_some_and(|value| {
                cost_limit_microusd(value).ok()
                    != previous
                        .options
                        .max_cost_usd
                        .and_then(|prior| cost_limit_microusd(prior).ok())
            })
        {
            return Err(HarnessError::InvalidRequest(
                "execution budgets are frozen when a root session starts and cannot be changed"
                    .into(),
            ));
        }

        options.max_total_tokens = previous.options.max_total_tokens;
        options.max_cost_usd = previous.options.max_cost_usd;
        options.budget_root_session_id = previous.options.budget_root_session_id.clone();
        let root = options.budget_root_session_id.as_deref().ok_or_else(|| {
            HarnessError::State("budgeted turn is missing its root session id".into())
        })?;
        let _guard = deps.locks.guard(&budget_lock_key(root)).await;
        let state = load_state(deps, root).await?.ok_or_else(|| {
            HarnessError::State(format!("shared budget ledger {root} is missing"))
        })?;
        state.verify_limits(options)?;
        return Ok(());
    }

    if options.max_total_tokens.is_none() && options.max_cost_usd.is_none() {
        return Ok(());
    }

    options.budget_root_session_id = Some(session_id.to_string());
    let _guard = deps.locks.guard(&budget_lock_key(session_id)).await;
    match load_state(deps, session_id).await? {
        Some(state) => state.verify_limits(options),
        None => save_state(deps, &BudgetState::new(session_id, options)?).await,
    }
}

/// Reserve one generation's maximum possible charge. Cost budgets fail closed
/// when the selected model lacks usable catalog pricing.
pub async fn reserve(
    deps: &Deps,
    record: &TurnRecord,
    input_tokens: u64,
    output_tokens: u64,
) -> Result<ReserveOutcome, HarnessError> {
    let Some(root) = record.options.budget_root_session_id.as_deref() else {
        return Ok(ReserveOutcome::Unlimited);
    };
    let requested_tokens = input_tokens.saturating_add(output_tokens);
    let requested_cost_microusd = if record.options.max_cost_usd.is_some() {
        let model = deps
            .router()
            .await
            .models_get(record.options.provider.as_deref(), &record.options.model)
            .await;
        let Some(pricing) = model.and_then(|model| model.pricing) else {
            return Ok(ReserveOutcome::Rejected(BudgetRejection::Unavailable(
                format!(
                    "cannot enforce max_cost_usd for model {} because no pricing is configured in \
                     the model catalog; configure input/output pricing or remove max_cost_usd",
                    record.options.model
                ),
            )));
        };
        match maximum_cost_microusd(input_tokens, output_tokens, &pricing) {
            Ok(cost) => cost,
            Err(reason) => {
                return Ok(ReserveOutcome::Rejected(BudgetRejection::Unavailable(
                    reason,
                )))
            }
        }
    } else {
        0
    };

    let _guard = deps.locks.guard(&budget_lock_key(root)).await;
    let mut state = load_state(deps, root)
        .await?
        .ok_or_else(|| HarnessError::State(format!("shared budget ledger {root} is missing")))?;
    state.verify_limits(&record.options)?;
    match state.reserve(requested_tokens, requested_cost_microusd) {
        Ok(()) => {
            save_state(deps, &state).await?;
            Ok(ReserveOutcome::Reserved(BudgetReservation {
                root_session_id: root.to_string(),
                reserved_tokens: requested_tokens,
                reserved_cost_microusd: requested_cost_microusd,
            }))
        }
        Err(rejection) => Ok(ReserveOutcome::Rejected(rejection)),
    }
}

/// Release a reservation when generation never reached the provider.
pub async fn release(deps: &Deps, reservation: &BudgetReservation) -> Result<(), HarnessError> {
    let root = &reservation.root_session_id;
    let _guard = deps.locks.guard(&budget_lock_key(root)).await;
    let mut state = load_state(deps, root)
        .await?
        .ok_or_else(|| HarnessError::State(format!("shared budget ledger {root} is missing")))?;
    state.release(reservation);
    save_state(deps, &state).await
}

/// Replace a worst-case reservation with actual provider usage. Missing or
/// partial usage consumes the full reservation so a provider cannot silently
/// bypass the hard limit.
pub async fn reconcile(
    deps: &Deps,
    reservation: &BudgetReservation,
    usage: Option<&Usage>,
) -> Result<(), HarnessError> {
    let actual_tokens = usage
        .and_then(|usage| match (usage.input, usage.output) {
            (Some(input), Some(output)) => Some(input.saturating_add(output)),
            _ => None,
        })
        .unwrap_or(reservation.reserved_tokens);
    let actual_cost_microusd = usage
        .and_then(|usage| usage.cost_usd)
        .and_then(|cost| observed_cost_microusd(cost).ok())
        .unwrap_or(reservation.reserved_cost_microusd);

    let root = &reservation.root_session_id;
    let _guard = deps.locks.guard(&budget_lock_key(root)).await;
    let mut state = load_state(deps, root)
        .await?
        .ok_or_else(|| HarnessError::State(format!("shared budget ledger {root} is missing")))?;
    state.reconcile(reservation, actual_tokens, actual_cost_microusd);
    save_state(deps, &state).await
}

pub async fn purge(
    deps: &Deps,
    root_session_id: &str,
    timeout_ms: u64,
) -> Result<(), HarnessError> {
    let _guard = deps.locks.guard(&budget_lock_key(root_session_id)).await;
    crate::state::state_delete(&deps.iii, BUDGET_SCOPE, root_session_id, timeout_ms).await
}

fn validate_options(options: &TurnOptions) -> Result<(), HarnessError> {
    if options.max_total_tokens == Some(0) {
        return Err(HarnessError::InvalidRequest(
            "max_total_tokens must be greater than zero".into(),
        ));
    }
    if let Some(cost) = options.max_cost_usd {
        cost_limit_microusd(cost)?;
    }
    Ok(())
}

fn cost_limit_microusd(cost_usd: f64) -> Result<u64, HarnessError> {
    if !cost_usd.is_finite() || cost_usd < 0.0 {
        return Err(HarnessError::InvalidRequest(
            "max_cost_usd must be a finite non-negative number".into(),
        ));
    }
    let scaled = cost_usd * MICRO_USD_PER_USD;
    if scaled > u64::MAX as f64 {
        return Err(HarnessError::InvalidRequest(
            "max_cost_usd is too large".into(),
        ));
    }
    Ok(scaled.floor() as u64)
}

fn observed_cost_microusd(cost_usd: f64) -> Result<u64, HarnessError> {
    if !cost_usd.is_finite() || cost_usd < 0.0 {
        return Err(HarnessError::State(
            "provider returned an invalid cost_usd".into(),
        ));
    }
    let scaled = cost_usd * MICRO_USD_PER_USD;
    if scaled > u64::MAX as f64 {
        return Err(HarnessError::State("provider cost_usd is too large".into()));
    }
    Ok(scaled.ceil() as u64)
}

fn maximum_cost_microusd(
    input_tokens: u64,
    output_tokens: u64,
    pricing: &Pricing,
) -> Result<u64, String> {
    let base_input_rate = pricing
        .input
        .filter(|rate| rate.is_finite() && *rate >= 0.0)
        .ok_or_else(|| "model catalog has no valid input pricing".to_string())?;
    let input_rate = [pricing.cache_read, pricing.cache_write]
        .into_iter()
        .flatten()
        .try_fold(None::<f64>, |maximum, rate| {
            if !rate.is_finite() || rate < 0.0 {
                return Err("model catalog contains invalid input pricing".to_string());
            }
            Ok(Some(maximum.map_or(rate, |current| current.max(rate))))
        })?
        .map_or(base_input_rate, |cached_rate| {
            base_input_rate.max(cached_rate)
        });
    let output_rate = pricing
        .output
        .filter(|rate| rate.is_finite() && *rate >= 0.0)
        .ok_or_else(|| "model catalog has no valid output pricing".to_string())?;

    // Catalog rates are USD per million tokens. Numerically, token * rate is
    // therefore already micro-USD.
    let cost = input_tokens as f64 * input_rate + output_tokens as f64 * output_rate;
    if !cost.is_finite() || cost > u64::MAX as f64 {
        return Err("model pricing produces an unrepresentable cost".into());
    }
    Ok(cost.ceil() as u64)
}

fn display_microusd(value: u64) -> String {
    format!("${:.6}", value as f64 / MICRO_USD_PER_USD)
}

fn budget_lock_key(root_session_id: &str) -> String {
    format!("budget:{root_session_id}")
}

async fn load_state(
    deps: &Deps,
    root_session_id: &str,
) -> Result<Option<BudgetState>, HarnessError> {
    let cfg = deps.cfg().await;
    let value = crate::state::state_get(
        &deps.iii,
        BUDGET_SCOPE,
        root_session_id,
        cfg.session_timeout_ms,
    )
    .await?;
    if value.is_null() {
        return Ok(None);
    }
    serde_json::from_value(value)
        .map(Some)
        .map_err(|error| HarnessError::State(format!("budget ledger parse: {error}")))
}

async fn save_state(deps: &Deps, state: &BudgetState) -> Result<(), HarnessError> {
    let cfg = deps.cfg().await;
    let value = serde_json::to_value(state)
        .map_err(|error| HarnessError::State(format!("budget ledger serialize: {error}")))?;
    crate::state::state_set(
        &deps.iii,
        BUDGET_SCOPE,
        &state.root_session_id,
        value,
        cfg.session_timeout_ms,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(tokens: Option<u64>, cost_microusd: Option<u64>) -> BudgetState {
        BudgetState {
            root_session_id: "root".into(),
            max_total_tokens: tokens,
            used_tokens: 0,
            reserved_tokens: 0,
            max_cost_microusd: cost_microusd,
            used_cost_microusd: 0,
            reserved_cost_microusd: 0,
        }
    }

    #[test]
    fn concurrent_reservations_cannot_exceed_the_shared_limit() {
        let mut ledger = state(Some(100), Some(1_000));
        assert!(ledger.reserve(60, 700).is_ok());
        assert!(matches!(
            ledger.reserve(41, 100),
            Err(BudgetRejection::Exceeded(_))
        ));
        assert!(matches!(
            ledger.reserve(20, 301),
            Err(BudgetRejection::Exceeded(_))
        ));
    }

    #[test]
    fn reconciliation_replaces_reserved_with_actual_usage() {
        let mut ledger = state(Some(100), Some(1_000));
        ledger.reserve(60, 700).unwrap();
        let reservation = BudgetReservation {
            root_session_id: "root".into(),
            reserved_tokens: 60,
            reserved_cost_microusd: 700,
        };
        ledger.reconcile(&reservation, 25, 300);
        assert_eq!(ledger.reserved_tokens, 0);
        assert_eq!(ledger.reserved_cost_microusd, 0);
        assert_eq!(ledger.used_tokens, 25);
        assert_eq!(ledger.used_cost_microusd, 300);
        assert!(ledger.reserve(75, 700).is_ok());
    }

    #[test]
    fn pricing_reserves_the_most_expensive_input_class() {
        let pricing = Pricing {
            input: Some(2.0),
            output: Some(10.0),
            cache_read: Some(0.2),
            cache_write: Some(2.5),
        };
        assert_eq!(maximum_cost_microusd(1_000, 100, &pricing), Ok(3_500));
    }

    #[test]
    fn pricing_fails_closed_without_uncached_input_or_output_rates() {
        let no_input = Pricing {
            cache_read: Some(0.2),
            output: Some(10.0),
            ..Pricing::default()
        };
        assert!(maximum_cost_microusd(1_000, 100, &no_input)
            .unwrap_err()
            .contains("input pricing"));

        let no_output = Pricing {
            input: Some(2.0),
            ..Pricing::default()
        };
        assert!(maximum_cost_microusd(1_000, 100, &no_output)
            .unwrap_err()
            .contains("output pricing"));
    }

    #[test]
    fn configured_cost_rounds_down_and_observed_cost_rounds_up() {
        assert_eq!(cost_limit_microusd(0.000_001_9), Ok(1));
        assert_eq!(observed_cost_microusd(0.000_001_1), Ok(2));
    }
}
