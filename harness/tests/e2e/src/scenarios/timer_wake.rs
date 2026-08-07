use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::{
    common, CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "timer_wake";

const RESULT_KEY: &str = "result";
const TIMER_ARMED: AssessmentSpec = AssessmentSpec::required(
    "timer_armed",
    30,
    "One wake-only relative timer is armed before any result write.",
);
const PARENT_WOKEN: AssessmentSpec = AssessmentSpec::required(
    "parent_woken",
    30,
    "The timer retires after waking the original session exactly once.",
);
const WAKE_ACTION: AssessmentSpec = AssessmentSpec::required(
    "wake_action",
    25,
    "The timer-woken turn persists the requested result.",
);
const CLEAN_COMPLETION: AssessmentSpec = AssessmentSpec::required(
    "clean_completion",
    15,
    "The root completes without children, errors, or surviving bindings.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[TIMER_ARMED, PARENT_WOKEN, WAKE_ACTION, CLEAN_COMPLETION];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = Names::new(run_id);
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: format!(
            r#"Test the parent-owned timer control plane in isolated state scope `{scope}`.

Register exactly one wake-only timer for roughly six seconds from now:

- use trigger type `timer` with `in_ms: 6000`;
- give it any non-empty top-level label;
- omit every function target so it wakes this session;
- do not spawn a child.

After the registration succeeds, end the turn immediately without writing state.

When the timer notification starts a new turn, write exactly
`{{ "status": "fired" }}` to `{scope}` / `{result_key}`, then respond briefly that the timer
fired. Leave no binding armed."#,
            scope = names.scope,
            result_key = RESULT_KEY,
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 24,
            max_output_tokens: Some(8_192),
            max_total_tokens: 400_000,
            stuck_timeout_seconds: 120,
        },
        denied_functions: &[],
        threshold: 90,
        criteria: assessment::criteria(ASSESSMENTS),
        judge_reference: None,
        setup: None,
        evaluate,
        cleanup: Some(cleanup),
    }
}

fn evaluate<'a>(
    context: &'a E2eContext,
    observation: &'a ScenarioObservation,
    run_id: &'a str,
) -> EvaluationFuture<'a> {
    Box::pin(async move {
        let names = Names::new(run_id);
        let expected = json!({ "status": "fired" });
        let observed = common::state_value(
            context
                .trigger_value(
                    "state::get",
                    json!({ "scope": names.scope, "key": RESULT_KEY }),
                )
                .await?,
        );
        let calls = common::function_calls(&observation.transcript);
        let registrations: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.function_id == "engine::register_trigger")
            .collect();
        let timers: Vec<_> = registrations
            .iter()
            .filter(|(_, call)| is_timer_registration(call))
            .collect();
        let writes: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.function_id == "state::set")
            .collect();
        let exact_write = writes.len() == 1
            && writes[0].1.arguments
                == json!({ "scope": names.scope, "key": RESULT_KEY, "value": expected });
        let timer_armed = registrations.len() == 1
            && timers.len() == 1
            && writes.len() == 1
            && timers[0].0 < writes[0].0;

        let records = common::trigger_fired_records(&observation.transcript);
        let timer_fired = records.len() == 1
            && records[0].get("retired").and_then(Value::as_bool) == Some(true)
            && records[0].get("once").and_then(Value::as_bool) == Some(true)
            && records[0].get("target").and_then(Value::as_str) == Some("harness::send");
        let root_only = observation.metrics.totals.sessions == 1
            && calls
                .iter()
                .all(|call| call.function_id != "harness::spawn");
        let active_bindings = common::active_binding_count(context, &names.root_session).await?;
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let response = observation.response.to_ascii_lowercase();
        let confirmed = response.contains("timer") && response.contains("fired");

        let parent_woken = timer_fired && root_only;
        let wake_action = exact_write && observed == expected;
        let clean_completion = active_bindings == 0 && no_errors && confirmed;

        Ok(assessment::objective([
            TIMER_ARMED.binary(
                timer_armed,
                format!(
                    "registrations={}, timers={}, writes={}",
                    registrations.len(),
                    timers.len(),
                    writes.len()
                ),
            ),
            PARENT_WOKEN.binary(
                parent_woken,
                format!("timer_fired={timer_fired}, root_only={root_only}"),
            ),
            WAKE_ACTION.binary(
                wake_action,
                format!("exact_write={exact_write}, observed={observed}"),
            ),
            CLEAN_COMPLETION.binary(
                clean_completion,
                format!(
                    "active_bindings={active_bindings}, function_errors={}, confirmed={confirmed}",
                    observation.metrics.totals.function_call_errors
                ),
            ),
        ]))
    })
}

fn is_timer_registration(call: &common::ObservedFunctionCall) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("timer")
        && call
            .arguments
            .pointer("/config/in_ms")
            .and_then(Value::as_u64)
            .is_some_and(|in_ms| (3_000..=15_000).contains(&in_ms))
        && call
            .arguments
            .get("label")
            .and_then(Value::as_str)
            .is_some_and(|label| !label.trim().is_empty())
        && common::is_wake_registration(&call.arguments)
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let names = Names::new(run_id);
        let listed = context
            .trigger_value(
                "harness::triggers::list",
                json!({ "session_id": names.root_session }),
            )
            .await?;
        for subscription_id in listed
            .get("subscriptions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|subscription| subscription.get("subscription_id").and_then(Value::as_str))
        {
            let _: Value = context
                .trigger(
                    "harness::triggers::unregister",
                    json!({
                        "session_id": names.root_session,
                        "subscription_id": subscription_id,
                    }),
                )
                .await?;
        }
        let _: Value = context
            .trigger(
                "state::delete",
                json!({ "scope": names.scope, "key": RESULT_KEY }),
            )
            .await?;
        Ok(())
    })
}

struct Names {
    scope: String,
    root_session: String,
}

impl Names {
    fn new(run_id: &str) -> Self {
        Self {
            scope: format!("e2e:timer:{run_id}"),
            root_session: format!("e2e_{run_id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_an_equivalent_relative_timer() {
        let call = common::ObservedFunctionCall {
            function_id: "engine::register_trigger".to_string(),
            arguments: json!({
                "trigger_type": "timer",
                "config": { "in_ms": 5000 },
                "label": "model-chosen-deadline"
            }),
        };

        assert!(is_timer_registration(&call));

        let spec = scenario("run");
        spec.validate().unwrap();
        assert_eq!(spec.version, 2);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("timer_armed", 30),
                ("parent_woken", 30),
                ("wake_action", 25),
                ("clean_completion", 15),
            ]
        );
    }
}
