//! `cron::on-tick`: one call per fire of a cron expression.
//!
//! Each observer passes `{ expression }` as args ([`CronArgs`]). The
//! function's hooks are set by id before it registers:
//!
//! - **added** retains the expression's job in the [`Scheduler`], starting it
//!   for the first observer of that expression, or rejects the observer with
//!   `INVALID_CRON_EXPRESSION`;
//! - **removed** releases it; the job stops with its last observer;
//! - **match** gives each tick to the observers of its expression, by keys
//!   (`args.expression` for an observer, `input.expression` for a call).
//!
//! Each fire of a job calls the function once ([`OnTickCaller`]); iii
//! delivers the tick to the matched observers whose filters accept it. The
//! handler returns the tick it is called with, so observers observe the tick.
//! The function is private: only this worker calls it. It registers as a
//! standby, so a second instance takes over its observers when the first
//! one leaves, and every expression fires once.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{
    AddedHook, Call, CallAction, CallRequest, IIIClient, Match, ObserveRejection, Observer,
    RegisterFunction, RemovedHook,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::scheduler::{Scheduler, Ticker};

pub const ON_TICK_FN: &str = "cron::on-tick";

pub const ON_TICK_DESCRIPTION: &str =
    "One call per fire of a cron expression. Observe it with args { expression } (UTC; 6 \
     fields: sec min hour day month weekday, a 7th year field is accepted); every observer of \
     that expression receives { expression, scheduled_time, actual_time } once per fire. Only \
     the cron worker calls it.";

/// Rejection codes an observer's `ready()` fails with.
pub mod codes {
    /// The expression does not parse (or is empty).
    pub const INVALID_CRON_EXPRESSION: &str = "INVALID_CRON_EXPRESSION";
}

/// An observer's args. The engine validates them against this schema
/// (`INVALID_ARGS` for a missing or non-string `expression`, or any other
/// field).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CronArgs {
    /// Cron expression (6-field format: sec min hour day month weekday; a 7th year field is accepted). UTC.
    pub expression: String,
}

/// One fire of an expression: the function's input and what its observers
/// observe.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct CronTick {
    /// The expression that fired, exactly as its observers passed it in args.
    pub expression: String,
    /// RFC 3339, UTC.
    pub scheduled_time: String,
    /// RFC 3339, UTC.
    pub actual_time: String,
}

/// Sets the hooks of [`ON_TICK_FN`] over `scheduler` by id, then registers
/// it, so the hooks run for every observer.
pub fn register(iii: &IIIClient, scheduler: Arc<Scheduler>) -> FunctionRef {
    let hooks = OnTickHooks { scheduler };
    iii.on_observer_added_for(ON_TICK_FN, hooks.clone());
    iii.on_observer_removed_for(ON_TICK_FN, hooks);
    // Each tick goes to the observers of its expression.
    iii.on_observer_match_for(
        ON_TICK_FN,
        Match::keys(
            |obs| expression_of(obs.args_value()).into(),
            |call| expression_of(&call.input).into(),
        ),
    );

    iii.register_function(
        ON_TICK_FN,
        RegisterFunction::new(on_tick)
            .description(ON_TICK_DESCRIPTION)
            .args_format::<CronArgs>()
            .private()
            .standby(),
    )
}

/// Removes the hooks [`register`] set, once the function is unregistered.
pub fn clear_hooks(iii: &IIIClient) {
    iii.clear_observer_added_for(ON_TICK_FN);
    iii.clear_observer_removed_for(ON_TICK_FN);
    iii.clear_observer_match_for(ON_TICK_FN);
}

/// The function's handler: the tick it is called with is what observers
/// observe.
fn on_tick(tick: CronTick) -> Result<CronTick, Error> {
    Ok(tick)
}

/// The match key of an observer's args and of a call's input: its
/// `expression`.
fn expression_of(value: &Value) -> Option<&str> {
    value.get("expression")?.as_str()
}

/// The hooks of [`ON_TICK_FN`]: one retained job per observer's expression.
#[derive(Clone)]
pub struct OnTickHooks {
    scheduler: Arc<Scheduler>,
}

#[async_trait]
impl AddedHook for OnTickHooks {
    async fn on_added(&self, obs: Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format.
        let CronArgs { expression } = obs.args()?;
        // Retained before the ack: the first fire after the observer becomes
        // active reaches it.
        admit(&self.scheduler, obs.id(), &expression).await?;
        tracing::info!(observer = %obs.id(), target = %obs.target(), expression = %expression, "cron::on-tick observer added");
        Ok(())
    }
}

#[async_trait]
impl RemovedHook for OnTickHooks {
    async fn on_removed(&self, obs: Observer) {
        self.scheduler.release(obs.id()).await;
        tracing::info!(observer = %obs.id(), "cron::on-tick observer removed");
    }
}

/// Retains the job of `expression` for `observer_id`. An expression that
/// does not parse rejects the observer.
async fn admit(
    scheduler: &Scheduler,
    observer_id: &str,
    expression: &str,
) -> Result<(), ObserveRejection> {
    scheduler
        .retain(observer_id, expression)
        .await
        .map_err(|e| ObserveRejection::new(codes::INVALID_CRON_EXPRESSION, e.to_string()))
}

/// The scheduler's [`Ticker`] in the worker: each tick is one fire-and-forget
/// call of [`ON_TICK_FN`], made only when an observer of its expression is
/// active.
pub struct OnTickCaller {
    iii: Arc<IIIClient>,
}

impl OnTickCaller {
    pub fn new(iii: Arc<IIIClient>) -> Self {
        Self { iii }
    }
}

impl Ticker for OnTickCaller {
    fn tick(&self, tick: CronTick) {
        let input = match serde_json::to_value(&tick) {
            Ok(input) => input,
            Err(error) => {
                tracing::error!(expression = %tick.expression, %error, "cron tick did not serialize");
                return;
            }
        };
        let call = Call {
            input,
            ..Call::default()
        };
        if !self.iii.is_observed_for(ON_TICK_FN, &call) {
            return;
        }
        let iii = self.iii.clone();
        tokio::spawn(async move {
            let request = CallRequest {
                action: Some(CallAction::Void),
                ..CallRequest::new(ON_TICK_FN, call.input)
            };
            if let Err(error) = iii.call(request).await {
                tracing::warn!(expression = %tick.expression, %error, "cron::on-tick call failed");
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::scheduler::tests::recording_scheduler;

    #[test]
    fn args_require_a_string_expression_and_nothing_else() {
        let args: CronArgs = serde_json::from_value(json!({"expression": "0 0 9 * * Mon-Fri"}))
            .expect("expression alone is valid");
        assert_eq!(args.expression, "0 0 9 * * Mon-Fri");
        for bad in [
            json!({}),
            json!({"expression": 5}),
            json!({"expression": "* * * * * *", "condition_function_id": "app::check"}),
        ] {
            assert!(
                serde_json::from_value::<CronArgs>(bad.clone()).is_err(),
                "{bad} must be refused"
            );
        }
    }

    /// What the engine validates observers' args against: `INVALID_ARGS`
    /// before the worker hears of the observer.
    #[test]
    fn args_format_requires_expression_and_denies_unknown_fields() {
        let schema = serde_json::to_value(schemars::schema_for!(CronArgs)).unwrap();
        assert_eq!(schema["required"], json!(["expression"]));
        assert_eq!(schema["additionalProperties"], json!(false));
        assert_eq!(schema["properties"]["expression"]["type"], "string");
    }

    #[test]
    fn the_tick_carries_the_expression_and_both_times() {
        let schema = serde_json::to_value(schemars::schema_for!(CronTick)).unwrap();
        let mut fields: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        fields.sort();
        assert_eq!(fields, ["actual_time", "expression", "scheduled_time"]);
    }

    #[test]
    fn the_handler_returns_the_tick_it_is_called_with() {
        let tick = CronTick {
            expression: "* * * * * *".into(),
            scheduled_time: "2026-10-20T00:00:00+00:00".into(),
            actual_time: "2026-10-20T00:00:00.004+00:00".into(),
        };
        assert_eq!(on_tick(tick.clone()).unwrap(), tick);
    }

    /// Observers are filed under their args' expression and a call matches
    /// the observers of its input's expression.
    #[test]
    fn the_match_key_is_the_expression() {
        let args = json!({ "expression": "0 0 9 * * Mon-Fri" });
        let tick = serde_json::to_value(CronTick {
            expression: "0 0 9 * * Mon-Fri".into(),
            scheduled_time: "2026-10-20T09:00:00+00:00".into(),
            actual_time: "2026-10-20T09:00:00+00:00".into(),
        })
        .unwrap();
        assert_eq!(expression_of(&args), Some("0 0 9 * * Mon-Fri"));
        assert_eq!(expression_of(&args), expression_of(&tick));
        assert_eq!(expression_of(&json!({})), None);
        assert_eq!(expression_of(&json!({ "expression": 5 })), None);
    }

    #[tokio::test]
    async fn a_bad_expression_is_rejected_with_invalid_cron_expression() {
        let (scheduler, _ticker) = recording_scheduler();
        for (expression, message) in [
            ("", "Cron expression is required"),
            ("   ", "Cron expression is required"),
            ("not a cron", "invalid cron expression 'not a cron'"),
            ("* * * * *", "invalid cron expression '* * * * *'"),
        ] {
            let rejection = admit(&scheduler, "o1", expression).await.unwrap_err();
            assert_eq!(rejection.code, codes::INVALID_CRON_EXPRESSION);
            assert!(
                rejection.message.contains(message),
                "{expression:?}: {}",
                rejection.message
            );
        }
        assert_eq!(scheduler.job_count().await, 0);
    }

    #[tokio::test]
    async fn an_observer_is_admitted_into_its_expressions_job() {
        let (scheduler, _ticker) = recording_scheduler();
        admit(&scheduler, "o1", "0 0 9 * * Mon-Fri").await.unwrap();
        admit(&scheduler, "o2", "0 0 9 * * Mon-Fri").await.unwrap();
        assert_eq!(scheduler.job_count().await, 1);
        scheduler.shutdown().await;
    }
}
