//! E2E-004 — a join predecessor registered with a divergent reaction spec is
//! rejected at `engine::register_trigger` time.
//!
//! The rctest-x7k2 postmortem shape: the agent registers the full downstream
//! task on the first predecessor and a `task: "placeholder"` shorthand on the
//! second. A join fires with the COMPLETING predecessor's spec, so before the
//! interceptor enforced spec equality the placeholder silently replaced the
//! real reaction. This scenario drives both registrations through the real
//! harness interceptor and pins the contract: first succeeds, second is
//! rejected with the corrective error.

use serde_json::{json, Value};

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::evidence_data::message_text;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";

/// One join-predecessor registration payload. `key` and `task` are the only
/// per-call variations — the same join id makes the second registration a
/// spec-mismatch the interceptor must reject.
fn register(key: &str, task: &str) -> Value {
    json!({
        "trigger_type": "harness::turn-completed",
        "config": { "session_id": format!("{{{{run_id}}}}-{key}") },
        "function_id": "harness::react",
        "metadata": {
            "task": task,
            "join": {
                "id": "{{run_id}}-join",
                "expect": ["w1", "w2"],
                "key": key
            }
        }
    })
}

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-004";
    const MESSAGE: &str = "Wire the join predecessors.";

    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "join-spec-mismatch",
        "A join predecessor with a divergent reaction spec is rejected at registration.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-004")
            .allow_id(REGISTER),
    )
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([REGISTER], []),
            )
            .respond(Response::function_call_raw(
                "call-w1",
                REGISTER,
                register(
                    "w1",
                    "You are the finalizer: verify every writer and report.",
                ),
                8,
                4,
            )),
    )
    // History from here on carries the first registration's result — a random
    // subscription id — so later generations pin the stable shape via
    // positional subset instead of exact history.
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-w1", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-w1",
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([REGISTER], []),
            )
            .respond(Response::function_call_raw(
                "call-w2",
                REGISTER,
                register("w2", "placeholder"),
                8,
                4,
            )),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant" }),
                        json!({ "role": "function_result", "function_call_id": "call-w1",
                                "is_error": false }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-w2", "function_id": REGISTER }
                        ] }),
                        // The rejection: the mismatch surfaces as an error
                        // function_result the model can read and correct.
                        json!({ "role": "function_result", "function_call_id": "call-w2",
                                "is_error": true }),
                    ])
                    .tools_exact_after_controls([REGISTER], []),
            )
            .respond(Response::text("join mismatch rejected", 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["join mismatch rejected"])?;

        let results = run.function_results(REGISTER);
        anyhow::ensure!(
            results.len() == 2,
            "expected 2 register_trigger results, got {}",
            results.len()
        );

        let first = results[0];
        anyhow::ensure!(
            first.get("is_error").and_then(Value::as_bool) == Some(false),
            "first predecessor registration must succeed: {first}"
        );
        anyhow::ensure!(
            message_text(first).contains("subscription_id"),
            "first registration must return a subscription id: {first}"
        );

        let second = results[1];
        anyhow::ensure!(
            second.get("is_error").and_then(Value::as_bool) == Some(true),
            "divergent predecessor registration must be REJECTED — a placeholder \
             spec that registers silently replaces the real reaction at fire \
             time: {second}"
        );
        let text = message_text(second);
        anyhow::ensure!(
            text.contains("differs from live predecessor"),
            "rejection must name the conflict: {text}"
        );
        anyhow::ensure!(
            text.contains("only `join.key` may differ"),
            "rejection must tell the agent how to fix the registration: {text}"
        );

        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_is_valid() {
        scenario().validate().unwrap();
    }

    /// The fixture's premise: both registrations target the same join id with
    /// different keys AND different tasks — the exact shape the interceptor
    /// must reject on the second registration.
    #[test]
    fn registrations_share_the_join_id_and_differ_only_where_intended() {
        let a = register("w1", "the real task");
        let b = register("w2", "placeholder");
        assert_eq!(
            a.pointer("/metadata/join/id"),
            b.pointer("/metadata/join/id")
        );
        assert_ne!(
            a.pointer("/metadata/join/key"),
            b.pointer("/metadata/join/key")
        );
        assert_ne!(a.pointer("/metadata/task"), b.pointer("/metadata/task"));
        assert_ne!(
            a.pointer("/config/session_id"),
            b.pointer("/config/session_id")
        );
    }
}
