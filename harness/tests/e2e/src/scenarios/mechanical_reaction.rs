use serde_json::{json, Value};

use crate::context::E2eContext;

use super::assessment::{self, AssessmentSpec};
use super::{
    common, CleanupFuture, EvaluationFuture, ExecutionPolicy, ScenarioObservation, ScenarioSpec,
};

pub const ID: &str = "mechanical_reaction";

const SOURCE_KEY: &str = "source";
const MIRROR_KEY: &str = "mirror";
const REACTIONS_ARMED: AssessmentSpec = AssessmentSpec::hard_gated(
    "reactions_armed",
    30,
    "The wake and mechanical call are registered before the source write.",
);
const MECHANICAL_MIRROR: AssessmentSpec = AssessmentSpec::hard_gated(
    "mechanical_mirror",
    35,
    "The call binding mirrors the complete source event without a root write.",
);
const PARENT_WOKEN: AssessmentSpec = AssessmentSpec::hard_gated(
    "parent_woken",
    20,
    "The mirror state event wakes only the original session.",
);
const CLEAN_COMPLETION: AssessmentSpec = AssessmentSpec::hard_gated(
    "clean_completion",
    15,
    "The run finishes without children, errors, or surviving bindings.",
);
const ASSESSMENTS: &[AssessmentSpec] = &[
    REACTIONS_ARMED,
    MECHANICAL_MIRROR,
    PARENT_WOKEN,
    CLEAN_COMPLETION,
];

pub fn scenario(run_id: &str) -> ScenarioSpec {
    let names = Names::new(run_id);
    let source = source_value(run_id);
    ScenarioSpec {
        id: ID,
        version: 2,
        prompt: format!(
            r#"Test a zero-token mechanical reaction in isolated state scope `{scope}`.

Register both reactions before writing any state:

1. A one-shot wake-only state reaction for `{scope}` / `{mirror_key}`, with any non-empty label.
2. A one-shot state call reaction for `{scope}` / `{source_key}` targeting `state::set`.
   Its fixed payload is `{{ "scope": "{scope}", "key": "{mirror_key}" }}` and its
   `event_into` is `/value`, so the full source event becomes the mirror value.

Then write exactly this value once to `{scope}` / `{source_key}`:

`{source}`

Do not write `{mirror_key}` yourself and do not spawn an agent. End the turn after the source
write. The call reaction must create the mirror without a model turn.

When the mirror wake starts a new turn, report briefly that the mirror completed and leave no
binding armed."#,
            scope = names.scope,
            source_key = SOURCE_KEY,
            mirror_key = MIRROR_KEY,
            source = serde_json::to_string(&source).expect("serialize scenario source"),
        ),
        filesystem_root: None,
        execution: ExecutionPolicy {
            max_turns: 20,
            max_output_tokens: Some(8_192),
            max_total_tokens: 250_000,
            stuck_timeout_seconds: 180,
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
        let source = source_value(run_id);
        let mirror = common::state_value(
            context
                .trigger_value(
                    "state::get",
                    json!({ "scope": names.scope, "key": MIRROR_KEY }),
                )
                .await?,
        );
        let calls = common::function_calls(&observation.transcript);
        let registrations: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.function_id == "engine::register_trigger")
            .collect();
        let wakes: Vec<_> = registrations
            .iter()
            .filter(|(_, call)| is_mirror_wake(call, &names))
            .collect();
        let mirrors: Vec<_> = registrations
            .iter()
            .filter(|(_, call)| is_mirror_call(call, &names))
            .collect();
        let writes: Vec<_> = calls
            .iter()
            .enumerate()
            .filter(|(_, call)| call.function_id == "state::set")
            .collect();
        let exact_source_write = writes.len() == 1
            && writes[0].1.arguments
                == json!({ "scope": names.scope, "key": SOURCE_KEY, "value": source });
        let reactions_armed = registrations.len() == 2
            && wakes.len() == 1
            && mirrors.len() == 1
            && writes.len() == 1
            && wakes[0].0 < writes[0].0
            && mirrors[0].0 < writes[0].0;

        let mirror_valid = valid_mirror(&mirror, &names, &source);
        let records = common::trigger_fired_records(&observation.transcript);
        let call_records: Vec<_> = records
            .iter()
            .filter(|record| record.get("target").and_then(Value::as_str) == Some("state::set"))
            .collect();
        let wake_records: Vec<_> = records
            .iter()
            .filter(|record| record.get("target").and_then(Value::as_str) == Some("harness::send"))
            .collect();
        let call_delivered = call_records.len() == 1;
        // A delivered ƒ-call fire always records what it dispatched; only a
        // skip/gc/expiry record omits `payload`, and neither of those can
        // have produced the mirror write `mirror_valid` checks below — so
        // pinning presence here catches a regression that stops recording it.
        let call_payload_recorded =
            call_records.len() == 1 && call_records[0].get("payload").is_some();
        let parent_woken = wake_records.len() == 1
            && wake_records[0].get("retired").and_then(Value::as_bool) == Some(true)
            && observation.metrics.totals.sessions == 1
            && calls
                .iter()
                .all(|call| call.function_id != "harness::spawn");

        let active_bindings = common::active_binding_count(context, &names.root_session).await?;
        let no_errors = observation.metrics.totals.function_call_errors == 0;
        let confirmed = observation.response.to_ascii_lowercase().contains("mirror");
        let mechanical_mirror =
            exact_source_write && mirror_valid && call_delivered && call_payload_recorded;
        let clean_completion = active_bindings == 0 && no_errors && confirmed;

        Ok(assessment::build_evaluation([
            REACTIONS_ARMED.full_or_zero(
                reactions_armed,
                format!(
                    "registrations={}, wakes={}, call_bindings={}, writes={}",
                    registrations.len(),
                    wakes.len(),
                    mirrors.len(),
                    writes.len()
                ),
            ),
            MECHANICAL_MIRROR.full_or_zero(
                mechanical_mirror,
                format!(
                    "exact_source_write={exact_source_write}, mirror_valid={mirror_valid}, \
                         call_delivered={call_delivered}, call_payload_recorded={call_payload_recorded}"
                ),
            ),
            PARENT_WOKEN.full_or_zero(
                parent_woken,
                format!(
                    "wake_records={}, sessions={}",
                    wake_records.len(),
                    observation.metrics.totals.sessions
                ),
            ),
            CLEAN_COMPLETION.full_or_zero(
                clean_completion,
                format!(
                    "active_bindings={active_bindings}, function_errors={}, confirmed={confirmed}",
                    observation.metrics.totals.function_call_errors
                ),
            ),
        ]))
    })
}

fn is_mirror_wake(call: &common::ObservedFunctionCall, names: &Names) -> bool {
    call.function_id == "engine::register_trigger"
        && call.arguments.get("trigger_type").and_then(Value::as_str) == Some("state")
        && call
            .arguments
            .pointer("/config/scope")
            .and_then(Value::as_str)
            == Some(names.scope.as_str())
        && call
            .arguments
            .pointer("/config/key")
            .and_then(Value::as_str)
            == Some(MIRROR_KEY)
        && call
            .arguments
            .get("label")
            .and_then(Value::as_str)
            .is_some_and(|label| !label.trim().is_empty())
        && common::is_wake_registration(&call.arguments)
}

fn is_mirror_call(call: &common::ObservedFunctionCall, names: &Names) -> bool {
    if call.function_id != "engine::register_trigger"
        || call.arguments.get("trigger_type").and_then(Value::as_str) != Some("state")
        || call
            .arguments
            .pointer("/config/scope")
            .and_then(Value::as_str)
            != Some(names.scope.as_str())
        || call
            .arguments
            .pointer("/config/key")
            .and_then(Value::as_str)
            != Some(SOURCE_KEY)
    {
        return false;
    }

    let (function_id, payload, event_into) = if let Some(target) = call
        .arguments
        .get("target")
        .filter(|target| !target.is_null())
    {
        (
            target.get("function_id"),
            target.get("payload"),
            target.get("event_into"),
        )
    } else {
        (
            call.arguments.get("function_id"),
            call.arguments.pointer("/metadata/payload"),
            call.arguments.pointer("/metadata/event_into"),
        )
    };

    function_id.and_then(Value::as_str) == Some("state::set")
        && payload
            .and_then(|payload| payload.get("scope"))
            .and_then(Value::as_str)
            == Some(names.scope.as_str())
        && payload
            .and_then(|payload| payload.get("key"))
            .and_then(Value::as_str)
            == Some(MIRROR_KEY)
        && event_into.and_then(Value::as_str) == Some("/value")
}

fn valid_mirror(mirror: &Value, names: &Names, source: &Value) -> bool {
    mirror.get("scope").and_then(Value::as_str) == Some(names.scope.as_str())
        && mirror.get("key").and_then(Value::as_str) == Some(SOURCE_KEY)
        && mirror.get("new_value") == Some(source)
        && mirror
            .get("event_type")
            .and_then(Value::as_str)
            .is_some_and(|event_type| event_type.starts_with("state:"))
}

fn source_value(run_id: &str) -> Value {
    json!({ "message": "mirror me", "run_id": run_id })
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
        for key in [SOURCE_KEY, MIRROR_KEY] {
            let _: Value = context
                .trigger("state::delete", json!({ "scope": names.scope, "key": key }))
                .await?;
        }
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
            scope: format!("e2e:mechanical:{run_id}"),
            root_session: format!("e2e_{run_id}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_both_call_binding_forms() {
        let names = Names::new("run");
        for arguments in [
            json!({
                "trigger_type": "state",
                "config": { "scope": names.scope, "key": SOURCE_KEY },
                "function_id": "state::set",
                "metadata": {
                    "payload": { "scope": names.scope, "key": MIRROR_KEY },
                    "event_into": "/value"
                }
            }),
            json!({
                "trigger_type": "state",
                "config": { "scope": names.scope, "key": SOURCE_KEY },
                "target": {
                    "function_id": "state::set",
                    "payload": { "scope": names.scope, "key": MIRROR_KEY },
                    "event_into": "/value"
                }
            }),
        ] {
            let call = common::ObservedFunctionCall {
                function_id: "engine::register_trigger".to_string(),
                arguments,
            };
            assert!(is_mirror_call(&call, &names));
        }

        let source = source_value("run");
        assert!(valid_mirror(
            &json!({
                "event_type": "state:created",
                "scope": names.scope,
                "key": SOURCE_KEY,
                "new_value": source
            }),
            &names,
            &source
        ));
        let spec = scenario("run");
        spec.validate().unwrap();
        assert_eq!(spec.version, 2);
        assert_eq!(
            spec.criteria
                .iter()
                .map(|criterion| (criterion.id, criterion.weight))
                .collect::<Vec<_>>(),
            vec![
                ("reactions_armed", 30),
                ("mechanical_mirror", 35),
                ("parent_woken", 20),
                ("clean_completion", 15),
            ]
        );
    }
}
