//! INT-011 — stopping a running root turn cancels the root and every spawned
//! child while preserving a message queued during the in-flight generation.
//!
//! The root's first response contains two fire-and-forget `harness::spawn`
//! calls. The next root generation and both child generations enter the same
//! scripted-router gate. The runner then observes all three requests, queues a
//! message on the root, calls `harness::stop` with the root turn id, and only
//! releases the gate after the stop acknowledgement.

use serde_json::json;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-011";
const SLUG: &str = "stop-cancel-cascade";
const ROOT_MESSAGE: &str = "Start two child workers and keep the root active.";
const CHILD_A: &str = "child-a-work";
const CHILD_B: &str = "child-b-work";
const QUEUED_MESSAGE: &str = "queued while the root is generating";
const GATE: &str = "stop-cancel-cascade-in-flight";
const SPAWN: &str = "harness::spawn";
const AGENT_TRIGGER: &str = "agent_trigger";

pub(super) fn scenario() -> ScenarioFixture {
    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        SLUG,
        "Stopping a running root turn cancels its root and spawned children while retaining a queued message.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(ROOT_MESSAGE)
            .idempotency_key("{{run_id}}:integration-011")
            .allow_id(SPAWN)
            .agent_trigger(),
    )
    .stop_cancel_cascade(GATE, QUEUED_MESSAGE)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(ROOT_MESSAGE)])
                    // `harness::spawn` is dispatched through the public
                    // `agent_trigger` wrapper; the scripted router only
                    // requires that the tool list is present here.
                    .tools_subset([]),
            )
            .respond(Response::function_calls_raw(
                vec![
                    (
                        "call-child-a",
                        AGENT_TRIGGER,
                        json!({ "function": SPAWN, "payload": { "task": CHILD_A } }),
                    ),
                    (
                        "call-child-b",
                        AGENT_TRIGGER,
                        json!({ "function": SPAWN, "payload": { "task": CHILD_B } }),
                    ),
                ],
                12,
                8,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-child-a", "function_id": AGENT_TRIGGER },
                            { "type": "function_call", "id": "call-child-b", "function_id": AGENT_TRIGGER }
                        ] }),
                    ])
                    .tools_subset([]),
            )
            .gate(GATE)
            .respond(Response::text("root generation should be cancelled", 16, 4)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({
                        "role": "user",
                        "content": [{ "type": "text", "text": CHILD_A }]
                    })])
                    .tools_subset([]),
            )
            .gate(GATE)
            .respond(Response::text("child A should be cancelled", 8, 3)),
    )
    .generation(
        Generation::new(4)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(".")
                    .messages_subset([json!({
                        "role": "user",
                        "content": [{ "type": "text", "text": CHILD_B }]
                    })])
                    .tools_subset([]),
            )
            .gate(GATE)
            .respond(Response::text("child B should be cancelled", 8, 3)),
    )
    .verify(verify)
    .scenario_timeout_ms(60_000)
    .build()
}

fn verify(run: &crate::evidence_data::RunEvidence) -> anyhow::Result<()> {
    anyhow::ensure!(
        run.status.get("status").and_then(serde_json::Value::as_str) == Some("cancelled"),
        "root status must be cancelled: {}",
        run.status
    );
    anyhow::ensure!(
        run.status
            .get("pending_function_calls")
            .and_then(serde_json::Value::as_array)
            .is_some_and(Vec::is_empty),
        "root has pending function calls after cancellation: {}",
        run.status
    );
    let queued_is_empty = run
        .status
        .get("queued")
        .and_then(serde_json::Value::as_array)
        .is_none_or(Vec::is_empty);
    anyhow::ensure!(
        queued_is_empty,
        "queued message was not drained by root cancellation: {}",
        run.status
    );
    anyhow::ensure!(
        run.status
            .get("children")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|children| children.len() == 2),
        "root status must retain both child references: {}",
        run.status
    );

    anyhow::ensure!(
        run.tree_statuses.len() == 3
            && run.tree_statuses.iter().all(|status| {
                status.get("status").and_then(serde_json::Value::as_str) == Some("cancelled")
            }),
        "root and both children must be durably cancelled: {:?}",
        run.tree_statuses
    );

    let control = &run.control;
    anyhow::ensure!(
        control
            .pointer("/gate/name")
            .and_then(serde_json::Value::as_str)
            == Some(GATE)
            && control
                .pointer("/gate/arrived")
                .and_then(serde_json::Value::as_u64)
                == Some(3)
            && control.pointer("/queued_send/queued") == Some(&serde_json::Value::Bool(true))
            && control.pointer("/stop_response/stopping") == Some(&serde_json::Value::Bool(true)),
        "stop intervention did not record all hard gates: {}",
        control
    );

    let aborts = run
        .router_evidence
        .get("aborts")
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    let aborted_requests = aborts
        .iter()
        .filter(|abort| abort.get("aborted") == Some(&serde_json::Value::Bool(true)))
        .filter_map(|abort| abort.get("request_id").and_then(serde_json::Value::as_str))
        .collect::<std::collections::BTreeSet<_>>();
    anyhow::ensure!(
        aborted_requests.len() >= 3,
        "router::abort must acknowledge root and both child requests: {}",
        run.router_evidence
    );

    let transcript = serde_json::to_string(&run.transcript)?;
    anyhow::ensure!(
        transcript.contains(QUEUED_MESSAGE),
        "queued message is missing from the root transcript"
    );
    anyhow::ensure!(
        transcript.contains("stopped by user"),
        "root transcript is missing the durable stop notice"
    );

    let lifecycle_name = "execute integration-probe::turn-completed";
    let lifecycle = run.spans_named(lifecycle_name);
    anyhow::ensure!(lifecycle.len() >= 3, "missing root/child lifecycle spans");
    let mut child_count = 0;
    for span in lifecycle {
        let payload = span
            .invocation_input()
            .ok_or_else(|| anyhow::anyhow!("lifecycle span has no input"))?;
        anyhow::ensure!(
            payload.get("terminal") == Some(&serde_json::Value::Bool(true))
                && payload.get("status").and_then(serde_json::Value::as_str) == Some("cancelled"),
            "lifecycle payload must be terminal cancelled: {payload}"
        );
        if payload
            .get("session_id")
            .and_then(serde_json::Value::as_str)
            != Some(run.session_id.as_str())
        {
            child_count += 1;
            anyhow::ensure!(
                payload
                    .pointer("/parent/session_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(run.session_id.as_str()),
                "child lifecycle is missing the root parent link: {payload}"
            );
        }
    }
    anyhow::ensure!(
        child_count == 2,
        "expected two child lifecycle events, got {child_count}"
    );
    anyhow::ensure!(
        run.generations_consumed == 4 && run.generations_total == 4,
        "{} of {} scripted generations consumed",
        run.generations_consumed,
        run.generations_total
    );
    anyhow::ensure!(
        run.assistant_texts().is_empty(),
        "a gated generation escaped before stop: {:?}",
        run.assistant_texts()
    );
    anyhow::ensure!(
        run.message_counts().0 >= 2,
        "root transcript should contain the original and queued user messages"
    );
    Ok(())
}
