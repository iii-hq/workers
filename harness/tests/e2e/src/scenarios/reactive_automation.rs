use std::time::Duration;

use serde_json::{json, Value};

use crate::context::E2eContext;
use crate::report::CriterionSource;

use super::common;
use super::{
    CleanupFuture, CriterionSpec, EvaluationFuture, ExecutionPolicy, ModelRequirements,
    ObjectiveEvaluation, ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "reactive_automation";
const SIGNAL_KEY: &str = "reactive_automation:signal";
const RESULT_KEY: &str = "reactive_automation:result";

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let scope = scope(run_id);
    let expected = expected(run_id);
    ScenarioSpec {
        id: ID,
        prompt: format!(
            "Set up a one-time automation so that writing key `{SIGNAL_KEY}` in scope `{scope}` writes this exact JSON value to key `{RESULT_KEY}` in the same scope: {}. Activate the automation by writing this signal value to `{SIGNAL_KEY}`: true. Confirm briefly after the automation is configured and the signal has been sent.",
            serde_json::to_string(&expected).expect("serialize expected value"),
        ),
        requirements: ModelRequirements {
            tools: true,
            minimum_context_window: 65_536,
            minimum_output_tokens: 2_048,
            ..ModelRequirements::default()
        },
        execution: ExecutionPolicy {
            max_turns: 24,
            max_output_tokens: 8_192,
            max_total_tokens: 204_800,
            timeout_seconds: 600,
            thinking_level: None,
        },
        threshold: 90,
        criteria: vec![
            CriterionSpec {
                id: "trigger_configuration",
                source: CriterionSource::Objective,
                weight: 40,
                description: "Registers the exact one-shot state reaction requested.",
            },
            CriterionSpec {
                id: "reactive_result",
                source: CriterionSource::Objective,
                weight: 40,
                description: "The reaction produces the exact durable result.",
            },
            CriterionSpec {
                id: "function_discipline",
                source: CriterionSource::Objective,
                weight: 10,
                description: "Signals once, avoids a direct result write, and has no call errors.",
            },
            CriterionSpec {
                id: "confirmation",
                source: CriterionSource::Objective,
                weight: 10,
                description: "The final response briefly confirms completion.",
            },
        ],
        judge_reference: None,
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
        let scope = scope(run_id);
        let expected = expected(run_id);
        let observed = wait_for_state(context, &scope, RESULT_KEY).await?;
        let calls = common::function_calls(&observation.transcript);

        let register_calls: Vec<_> = calls
            .iter()
            .filter(|call| call.function_id == "engine::register_trigger")
            .collect();
        let exact_registration = register_calls.len() == 1
            && registration_matches(
                &register_calls[0].arguments,
                &scope,
                SIGNAL_KEY,
                RESULT_KEY,
                &expected,
            );
        let state_writes: Vec<_> = calls
            .iter()
            .filter(|call| call.function_id == "state::set")
            .collect();
        let exact_signal = state_writes
            .iter()
            .filter(|call| {
                call.arguments == json!({ "scope": scope, "key": SIGNAL_KEY, "value": true })
            })
            .count()
            == 1;
        let direct_result_write = state_writes.iter().any(|call| {
            call.arguments.get("scope").and_then(Value::as_str) == Some(scope.as_str())
                && call.arguments.get("key").and_then(Value::as_str) == Some(RESULT_KEY)
        });
        let result_matches = observed == expected;
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let disciplined = exact_signal && !direct_result_write && no_errors;
        let response = common::final_response(&observation.output);
        let concise_confirmation = !response.trim().is_empty() && response.chars().count() <= 240;

        Ok(ObjectiveEvaluation {
            hard_gates: vec![
                common::gate(
                    "one_shot_trigger_registered",
                    exact_registration,
                    format!(
                        "observed {} engine::register_trigger call(s)",
                        register_calls.len()
                    ),
                ),
                common::gate(
                    "reactive_result_persisted",
                    result_matches,
                    format!("expected {expected}, observed {observed}"),
                ),
                common::gate(
                    "signal_written_once",
                    exact_signal,
                    "the requested signal must be written exactly once",
                ),
                common::gate(
                    "result_not_written_directly",
                    !direct_result_write,
                    "the root agent must not write the result key directly",
                ),
            ],
            awards: vec![
                common::award(
                    "trigger_configuration",
                    if exact_registration { 40 } else { 0 },
                    "awarded for the exact one-shot state reaction",
                ),
                common::award(
                    "reactive_result",
                    if result_matches { 40 } else { 0 },
                    "awarded when the reaction writes the exact result",
                ),
                common::award(
                    "function_discipline",
                    if disciplined { 10 } else { 0 },
                    "awarded for one signal, no direct result write, and no errors",
                ),
                common::award(
                    "confirmation",
                    if concise_confirmation { 10 } else { 0 },
                    "awarded for a non-empty confirmation under 240 characters",
                ),
            ],
            evidence: json!({
                "expected": expected,
                "actual": observed,
                "registrations": register_calls.iter().map(|call| &call.arguments).collect::<Vec<_>>(),
                "state_writes": state_writes.iter().map(|call| &call.arguments).collect::<Vec<_>>(),
                "final_response": response,
            }),
        })
    })
}

fn registration_matches(
    arguments: &Value,
    scope: &str,
    signal_key: &str,
    result_key: &str,
    expected: &Value,
) -> bool {
    arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
        && arguments.pointer("/config/scope").and_then(Value::as_str) == Some(scope)
        && arguments.pointer("/config/key").and_then(Value::as_str) == Some(signal_key)
        && arguments
            .get("once")
            .map(|value| value.as_bool() == Some(true))
            .unwrap_or(true)
        && arguments.get("function_id").and_then(Value::as_str) == Some("harness::react")
        && reaction_strategy_matches(arguments, scope, result_key, expected)
}

fn reaction_strategy_matches(
    arguments: &Value,
    scope: &str,
    result_key: &str,
    expected: &Value,
) -> bool {
    if arguments.pointer("/metadata/call").is_some() {
        return event_preserves_result_value(arguments)
            && arguments
                .pointer("/metadata/call/function_id")
                .and_then(Value::as_str)
                == Some("state::set")
            && arguments.pointer("/metadata/call/payload")
                == Some(&json!({ "scope": scope, "key": result_key, "value": expected }));
    }
    arguments
        .pointer("/metadata/task")
        .and_then(Value::as_str)
        .is_some_and(|task| !task.trim().is_empty())
}

fn event_preserves_result_value(arguments: &Value) -> bool {
    arguments
        .pointer("/metadata/call/event_into")
        .and_then(Value::as_str)
        .is_none_or(|pointer| pointer != "/value" && !pointer.starts_with("/value/"))
}

async fn wait_for_state(context: &E2eContext, scope: &str, key: &str) -> anyhow::Result<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let value = common::state_value(
            context
                .trigger("state::get", json!({ "scope": scope, "key": key }))
                .await?,
        );
        if !value.is_null() {
            return Ok(value);
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(Value::Null);
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn cleanup<'a>(context: &'a E2eContext, run_id: &'a str) -> CleanupFuture<'a> {
    Box::pin(async move {
        let scope = scope(run_id);
        for key in [SIGNAL_KEY, RESULT_KEY] {
            let _: Value = context
                .trigger("state::delete", json!({ "scope": scope, "key": key }))
                .await?;
        }
        Ok(())
    })
}

fn scope(run_id: &str) -> String {
    format!("e2e:{run_id}")
}

fn expected(run_id: &str) -> Value {
    json!({ "handled": true, "run_id": run_id })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_requires_the_exact_reactive_edge() {
        let expected = json!({"handled": true});
        let valid = json!({
            "trigger_type": "state",
            "config": {"scope": "s", "key": "signal"},
            "once": true,
            "function_id": "harness::react",
            "metadata": {
                "call": {
                    "function_id": "state::set",
                    "payload": {"scope": "s", "key": "result", "value": expected}
                }
            }
        });
        assert!(registration_matches(
            &valid, "s", "signal", "result", &expected
        ));
        let mut default_once = valid.clone();
        default_once.as_object_mut().unwrap().remove("once");
        assert!(registration_matches(
            &default_once,
            "s",
            "signal",
            "result",
            &expected
        ));
        let mut injects_event = valid.clone();
        injects_event["metadata"]["call"]["event_into"] = json!("/value/_trigger_event");
        assert!(!registration_matches(
            &injects_event,
            "s",
            "signal",
            "result",
            &expected
        ));
        let mut injects_outside_value = valid.clone();
        injects_outside_value["metadata"]["call"]["event_into"] = json!("/_event");
        assert!(registration_matches(
            &injects_outside_value,
            "s",
            "signal",
            "result",
            &expected
        ));
        let task_strategy = json!({
            "trigger_type": "state",
            "config": {"scope": "s", "key": "signal"},
            "once": true,
            "function_id": "harness::react",
            "metadata": {
                "task": "Persist the requested result when this event fires."
            }
        });
        assert!(registration_matches(
            &task_strategy,
            "s",
            "signal",
            "result",
            &expected
        ));
        let mut direct = valid;
        direct["metadata"]["call"]["payload"]["key"] = json!("signal");
        assert!(!registration_matches(
            &direct, "s", "signal", "result", &expected
        ));
    }
}
