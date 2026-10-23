//! `cron::on-tick`: the observable function next to the `cron` trigger type.
//!
//! The function is owner-delivered. Each observer passes `{ expression }` as
//! args; [`OnTickHandler`] files it in the scheduler under that expression,
//! which runs one job per distinct expression for observers and `cron`
//! trigger bindings alike. Each fire hands the tick to that expression's
//! observers with `obs.emit` (the SDK applies their filters) and calls its
//! bindings as the trigger type always did.
//!
//! Ticks are routed in-process by the scheduler, never by invoking the
//! function, and its handler refuses every invocation: a caller could
//! otherwise make this worker emit forged ticks.
//!
//! On an engine without observe the function registers as a plain function,
//! its hooks never run, and the trigger type carries everything.

use std::sync::Arc;

use async_trait::async_trait;
use iii_sdk::errors::Error;
use iii_sdk::runtime::FunctionRef;
use iii_sdk::{IIIClient, Observe, ObserveHandler, ObserveRejection, Observer, RegisterFunction};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::boot::SchedulerCell;
use crate::scheduler::{Scheduler, TickObserver};

pub const ON_TICK_FN: &str = "cron::on-tick";

pub const ON_TICK_DESCRIPTION: &str =
    "Fires on a cron schedule. Observe it with args { expression } (UTC; 6 fields: sec min \
     hour day month weekday, a 7th year field is accepted); every observer of an expression \
     receives { trigger, expression, scheduled_time, actual_time } once per fire. Only the cron \
     worker fires it: invoking it directly is refused.";

/// Rejection codes an observer's `ready()` fails with.
pub mod codes {
    /// The expression does not parse (or is empty). The message is the one
    /// the `cron` trigger type gives for the same config.
    pub const INVALID_CRON_EXPRESSION: &str = "INVALID_CRON_EXPRESSION";
    pub const FORBIDDEN: &str = "FORBIDDEN";
}

/// An observer's args. The engine validates them against this schema
/// (`INVALID_ARGS` for a missing or non-string `expression`, or any other
/// field): `condition_function_id` is not carried over from the trigger
/// config; filter on the tick instead.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CronArgs {
    /// Cron expression (6-field format: sec min hour day month weekday; a 7th year field is accepted). UTC.
    pub expression: String,
}

/// What each observer receives per fire. One tick serves every observer of
/// the expression, so it carries no `job_id`: a target that needs its
/// observer reads `observer_id` from the envelope.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct CronTick {
    /// Always `cron`, as in the trigger payload: kept while the trigger type
    /// is deprecated, so a handler moved over that switches on it still works.
    pub trigger: String,
    /// The expression that fired, exactly as the observer passed it in args.
    pub expression: String,
    /// RFC 3339, UTC.
    pub scheduled_time: String,
    /// RFC 3339, UTC.
    pub actual_time: String,
}

/// Registers `cron::on-tick` with [`OnTickHandler`] as its hooks.
pub fn register(iii: &IIIClient, scheduler: SchedulerCell) -> FunctionRef {
    iii.register_function(
        ON_TICK_FN,
        RegisterFunction::new(refuse_invocation)
            .description(ON_TICK_DESCRIPTION)
            .observe(
                Observe::owner()
                    .args_format::<CronArgs>()
                    .handler(OnTickHandler { scheduler }),
            ),
    )
}

/// The function's handler: ticks only come from the scheduler.
fn refuse_invocation(_tick: CronTick) -> Result<(), Error> {
    Err(Error::Remote {
        code: codes::FORBIDDEN.to_string(),
        message: format!(
            "{ON_TICK_FN} is an event source: observe it \
             (iii.observe({{ function_id: '{ON_TICK_FN}', args: {{ expression }} }}, target)); \
             only the cron worker fires its ticks"
        ),
        stacktrace: None,
    })
}

/// Files each observer of `cron::on-tick` under its expression, and takes it
/// out again when the engine removes it.
pub struct OnTickHandler {
    scheduler: SchedulerCell,
}

#[async_trait]
impl ObserveHandler for OnTickHandler {
    async fn on_observer_added(&self, obs: &Observer) -> Result<(), ObserveRejection> {
        // Checked by the engine against args_format.
        let CronArgs { expression } = obs.args()?;
        // Filed before the ack: the first fire after the engine marks the
        // observer active reaches it.
        let scheduler = self.scheduler.read().await;
        admit(&scheduler, &expression, Arc::new(obs.clone())).await?;
        drop(scheduler);
        tracing::info!(observer = %obs.id(), target = %obs.target(), expression = %expression, "cron::on-tick observer added");
        Ok(())
    }

    async fn on_observer_removed(&self, obs: &Observer) {
        self.scheduler.read().await.remove_observer(obs.id()).await;
        tracing::info!(observer = %obs.id(), "cron::on-tick observer removed");
    }
}

/// Files `observer` under `expression`, joining its job or starting one.
/// The expression goes through the parser the trigger type uses, with the
/// same message; a failure rejects the observer.
async fn admit(
    scheduler: &Scheduler,
    expression: &str,
    observer: Arc<dyn TickObserver>,
) -> Result<(), ObserveRejection> {
    scheduler
        .add_observer(expression, observer)
        .await
        .map_err(|e| ObserveRejection::new(codes::INVALID_CRON_EXPRESSION, e.to_string()))
}

impl TickObserver for Observer {
    fn id(&self) -> &str {
        Observer::id(self)
    }

    fn emit(&self, tick: &CronTick) -> Result<bool, String> {
        Observer::emit(self, tick).map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::scheduler::tests::{counting_scheduler, RecordingObserver};

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
    fn the_tick_has_no_job_id_and_no_condition() {
        let schema = serde_json::to_value(schemars::schema_for!(CronTick)).unwrap();
        let mut fields: Vec<&String> = schema["properties"].as_object().unwrap().keys().collect();
        fields.sort();
        assert_eq!(
            fields,
            ["actual_time", "expression", "scheduled_time", "trigger"]
        );
    }

    #[tokio::test]
    async fn a_bad_expression_is_rejected_with_the_trigger_types_message() {
        let (scheduler, _invoker) = counting_scheduler();
        for (expression, message) in [
            ("", "Cron expression is required"),
            ("   ", "Cron expression is required"),
            ("not a cron", "invalid cron expression 'not a cron'"),
            ("* * * * *", "invalid cron expression '* * * * *'"),
        ] {
            let observer = Arc::new(RecordingObserver::new("o1"));
            let rejection = admit(&scheduler, expression, observer).await.unwrap_err();
            assert_eq!(rejection.code, codes::INVALID_CRON_EXPRESSION);
            assert!(
                rejection.message.contains(message),
                "{expression:?}: {}",
                rejection.message
            );
            // The trigger type refuses the same config with the same text.
            let err = scheduler
                .register(crate::scheduler::JobSpec {
                    trigger_id: "t1".into(),
                    expression: expression.into(),
                    function_id: "app::f".into(),
                    condition_function_id: None,
                    metadata: None,
                })
                .await
                .unwrap_err();
            assert_eq!(err.to_string(), rejection.message);
        }
        assert_eq!(scheduler.job_count().await, 0);
    }

    #[tokio::test]
    async fn an_observer_is_admitted_into_its_expressions_job() {
        let (scheduler, _invoker) = counting_scheduler();
        admit(
            &scheduler,
            "0 0 9 * * Mon-Fri",
            Arc::new(RecordingObserver::new("o1")),
        )
        .await
        .unwrap();
        assert_eq!(scheduler.job_count().await, 1);
        scheduler.shutdown().await;
    }

    #[test]
    fn invoking_the_function_directly_is_forbidden() {
        let err = refuse_invocation(CronTick {
            trigger: "cron".into(),
            expression: "* * * * * *".into(),
            scheduled_time: "2026-10-20T00:00:00+00:00".into(),
            actual_time: "2026-10-20T00:00:00+00:00".into(),
        })
        .unwrap_err();
        match err {
            Error::Remote { code, message, .. } => {
                assert_eq!(code, "FORBIDDEN");
                assert!(message.contains("observe it"), "{message}");
            }
            other => panic!("expected a FORBIDDEN remote error, got {other:?}"),
        }
    }
}
