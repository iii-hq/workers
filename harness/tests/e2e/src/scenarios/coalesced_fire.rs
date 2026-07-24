//! E2E-007 — capped fires COALESCE: burst past the rate cap, observe exactly
//! `cap + 1` reaction dispatches, the last stamped as the coalesced stand-in.
//!
//! This gates the MOT-4211 coalescing path end-to-end: a CALL-mode reaction
//! bound to a state key is fired 5 times under a shrunken gate
//! (`III_HARNESS_MAX_FIRES_PER_WINDOW=3`, `III_HARNESS_FIRE_WINDOW_MS=2000`
//! — the env override exists for exactly this scenario). Three fires deliver
//! immediately; the two capped ones park, newest-wins, and ONE trailing fire
//! delivers after the window frees, stamped `__coalesced_fires: 2` with the
//! self-describing recovery note. Without coalescing (pre-MOT-4211 drop
//! semantics) the run times out waiting for the 4th dispatch.
//!
//! Two runner capabilities make this observable at all — call-mode reactions
//! seed no turn and no session trace, so turn completions and session traces
//! are both blind to them:
//! * probe-side WHOLE-RUN call evidence (the probe serves the controlled
//!   function, so it records every dispatch regardless of origin), and
//! * `await_target_calls`, which holds the await phase until the trailing
//!   fire lands.

use serde_json::{json, Value};

use super::dsl::{ControlledFunction, Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const REGISTER: &str = "engine::register_trigger";
const SCOPE: &str = "e2e-007";
const KEY: &str = "burst";
const BURST: usize = 5;
const CAP: usize = 3;

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "E2E-007";
    const MESSAGE: &str = "Arm a call reaction on the burst key.";

    let model = Model::scripted("fixture-model");
    let record = ControlledFunction::new("{{run_id}}::record", "Record one event.")
        .request_schema(json!({
            "type": "object",
            "additionalProperties": true,
            "properties": { "event": { "type": "object" } }
        }))
        .returns_text("recorded");

    // CALL mode: each fire dispatches the recorder directly (no model turn),
    // with the state event injected at `/event`. `once: false` keeps the
    // binding standing for the whole burst.
    let register_args = json!({
        "trigger_type": "state",
        "config": { "scope": SCOPE, "key": KEY },
        "function_id": "harness::react",
        "once": false,
        "metadata": { "call": { "function_id": "{{run_id}}::record" } }
    });

    let mut scenario = Scenario::new(
        ID,
        "coalesced-fire",
        "Burst past the fire-rate cap; capped fires coalesce into one stamped trailing dispatch.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:e2e-007")
            .allow_id(REGISTER)
            .allow_function(&record),
    )
    // Shrink the gate so the trailing fire lands in ~2s instead of ~60s.
    .harness_env("III_HARNESS_MAX_FIRES_PER_WINDOW", "3")
    .harness_env("III_HARNESS_FIRE_WINDOW_MS", "2000")
    // The burst + the trailing coalesced fire — cap + 1 dispatches total.
    .await_target_calls(CAP + 1)
    // Call reactions seed no turn: only Send's own trace tree exists.
    .expect_traces(1)
    .function(record.clone())
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::function_call_raw(
                "call-arm",
                REGISTER,
                register_args,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request()
                    .system_prompt_sha256("{{system_prompt_sha256}}")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-arm", "function_id": REGISTER }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-arm",
                                "is_error": false }),
                    ])
                    .tools_exact([record.tool()]),
            )
            .respond(Response::text("armed", 10, 2)),
    );
    // The burst: 5 sequential writes to one key after the main turn is idle.
    // Under cap=3 the gate admits three and parks the rest.
    for seq in 1..=BURST {
        scenario = scenario.probe_after(
            1,
            "state::set",
            json!({ "scope": SCOPE, "key": KEY, "value": { "seq": seq } }),
        );
    }
    scenario
        .verify(|run| {
            run.expect_assistant_texts(["armed"])?;
            // cap + 1: the burst floods 5 fires, exactly 3 deliver plus ONE
            // trailing coalesced stand-in. 5 dispatches would mean the cap
            // never engaged; 3 would mean the capped tail was dropped
            // (pre-MOT-4211 semantics).
            run.expect_target_calls(CAP + 1)?;

            let events: Vec<Value> = run
                .target_calls
                .iter()
                .map(|call| call.get("event").cloned().unwrap_or(Value::Null))
                .collect();
            let seq_of = |event: &Value| {
                event
                    .get("new_value")
                    .and_then(|value| value.get("seq"))
                    .and_then(Value::as_u64)
            };

            let mut seen = std::collections::BTreeSet::new();
            for (index, event) in events[..CAP].iter().enumerate() {
                anyhow::ensure!(
                    event.get("__coalesced_fires").is_none(),
                    "delivered fire {index} must not carry a coalesced stamp: {event}"
                );
                let seq = seq_of(event)
                    .ok_or_else(|| anyhow::anyhow!("delivered fire {index} has no seq: {event}"))?;
                anyhow::ensure!(
                    (1..=BURST as u64).contains(&seq) && seen.insert(seq),
                    "delivered fires must carry distinct burst seqs, got {seq} in {seen:?}"
                );
            }

            let trailing = &events[CAP];
            anyhow::ensure!(
                trailing.get("__coalesced_fires").and_then(Value::as_u64)
                    == Some((BURST - CAP) as u64),
                "trailing fire must be stamped __coalesced_fires={}: {trailing}",
                BURST - CAP
            );
            let note = trailing
                .get("__coalesced_note")
                .and_then(Value::as_str)
                .unwrap_or_default();
            anyhow::ensure!(
                note.contains("NOT delivered") && note.contains("recompute"),
                "coalesced note must self-describe the recovery contract: {note:?}"
            );
            let trailing_seq = seq_of(trailing)
                .ok_or_else(|| anyhow::anyhow!("trailing fire has no seq: {trailing}"))?;
            anyhow::ensure!(
                (1..=BURST as u64).contains(&trailing_seq) && !seen.contains(&trailing_seq),
                "the trailing event must be one of the CAPPED fires (newest wins), got seq \
                 {trailing_seq} vs delivered {seen:?}"
            );
            // 4 of 5 seqs observed: exactly one capped fire was collapsed
            // into its newer sibling — the coalescing contract.

            run.expect_no_duplicate_messages()
        })
        .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_declares_gate_env_burst_probes_and_call_await() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 1);
        assert_eq!(fixture.probe_actions.len(), BURST);
        assert_eq!(fixture.await_target_calls, Some(CAP + 1));
        // Call reactions leave no session trace: the override pins the floor
        // to Send's own trace instead of the 1-per-probe-action formula.
        assert_eq!(fixture.expected_traces(), 1);
        assert!(fixture
            .harness_env
            .iter()
            .any(|(k, v)| k == "III_HARNESS_MAX_FIRES_PER_WINDOW" && v == "3"));
        assert!(fixture
            .harness_env
            .iter()
            .any(|(k, v)| k == "III_HARNESS_FIRE_WINDOW_MS" && v == "2000"));
    }
}
