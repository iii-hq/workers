//! INT-013 — a pre-trigger hook holds a function call; `harness::function::resolve`
//! releases it and the turn completes.
//!
//! NOT REGISTERED YET (no `mod` declaration): running this fixture against
//! the live stack exposed what looks like a real harness defect — once the
//! holder hook parks the call, `harness::status` first times out at the
//! engine and later returns `null` (the turn record disappears from state),
//! so the resolve intervention can never find the parked call. Isolation
//! runs proved the scripted-hook plumbing itself is sound: the same fixture
//! with `Continue` behaviors completes in ~2.7s with both hooks served.
//! Register this scenario once the hold-path defect is fixed (MOT-4296).
//!
//! This is the deferred-call seam end to end: the scripted holder hook answers
//! `{"decision":"hold"}`, the harness parks the call (`CallState::Pending`,
//! turn `awaiting_functions`) instead of executing it, and the runner
//! intervention proves the resolve no-op gates (wrong turn id, unknown call
//! id) leave it parked before `action: "execute"` resumes the hook chain
//! AFTER the holder — the second scripted hook runs exactly once, the target
//! executes exactly once, and the turn finishes on the pinned function result.
//!
//! Out of scope by design: `harness::sweep-pending` only expires pending
//! calls with no holder (`held_by: None`), and a hook hold is currently the
//! only pending-call producer — so the sweep's resolved path has no reachable
//! fixture today.

use serde_json::json;

use super::dsl::{
    ControlledFunction, Generation, Hook, Message, Model, Request, Response, Scenario, Send,
};
use super::ScenarioDriver;
use crate::fixtures::{HookBehavior, ScenarioFixture};

const CALL_ID: &str = "call-held";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-013";
    const MESSAGE: &str = "Record one value; approval will hold it.";
    const FINAL_TEXT: &str = "recorded after release";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one value.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": false,
            "properties": { "value": { "type": "string" } },
            "required": ["value"]
        }))
        .returns_text("recorded");

    Scenario::new(
        ID,
        "held-call-resolve",
        "A pre-trigger hook holds a call and harness::function::resolve releases it through the \
         remaining chain.",
        ScenarioDriver::Direct,
        model,
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-013")
            .allow_function(&record),
    )
    .function(record.clone())
    // The holder parks the call on first sight and must stay silent when the
    // chain resumes; the second hook proves the resume starts AFTER the holder.
    .hook(Hook::new("holder", "pre-trigger", HookBehavior::HoldOnce))
    .hook(Hook::new("chain", "pre-trigger", HookBehavior::Continue).priority(10))
    .held_call_resolve(CALL_ID)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact_after_controls([], [record.tool()]),
            )
            .respond(Response::function_call(
                CALL_ID,
                &record,
                json!({ "value": "held-then-released" }),
                8,
                4,
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
                            { "type": "function_call", "id": CALL_ID }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": CALL_ID,
                                "is_error": false }),
                    ])
                    .tools_exact_after_controls([], [record.tool()]),
            )
            .respond(Response::text(FINAL_TEXT, 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts([FINAL_TEXT])?;
        run.expect_message_counts(1, 2, 1)?;
        // The target executed exactly once, only after the release.
        run.expect_target_calls(1)?;
        anyhow::ensure!(
            run.target_calls[0] == json!({ "value": "held-then-released" }),
            "target payload {:?} != held-then-released",
            run.target_calls[0]
        );
        // The holder saw the original dispatch; the chain hook only ran on the
        // post-resolve resume — each exactly once.
        run.expect_hook_calls("holder", 1)?;
        run.expect_hook_calls("chain", 1)?;
        let control = &run.control;
        anyhow::ensure!(
            control.get("kind").and_then(serde_json::Value::as_str) == Some("held_call_resolve"),
            "intervention control missing: {control}"
        );
        anyhow::ensure!(
            control
                .pointer("/execute_resolve/turn_resumed")
                .and_then(serde_json::Value::as_bool)
                == Some(true),
            "execute resolve did not resume the turn: {control}"
        );
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_holds_and_resolves_one_call() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.hooks.len(), 2);
        assert_eq!(fixture.hooks[0].behavior, HookBehavior::HoldOnce);
        assert!(fixture.intervention.is_some());
        assert_eq!(fixture.expected_traces(), 1);
    }
}
