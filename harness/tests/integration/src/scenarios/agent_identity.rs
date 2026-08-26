//! INT-026 — agent-profile identity, end to end (MOT-4485): a send naming
//! `options.agent` (and OMITTING `options.functions`) runs as the directory
//! profile, and delegation resolves spawn-side:
//!   * the parent's prompt is the top-level identity ENRICHED with
//!     `You are Lead.` + the profile body, resolved server-side from the
//!     run's `agents/lead.md` — no prompt fields on the send;
//!   * the omitted policy defaults to the configured baseline (`allow: *`),
//!     which is what lets the spawn dispatch at all;
//!   * the spawn (`agent: coder`) seeds a child whose prompt is the
//!     sub-agent identity enriched with `You are Coder.` — the leaf profile
//!     applied spawn-side, no `options.system_prompt` anywhere.
//!
//! The child runs in its own session (untracked by the floor); its generation
//! being consumed with the profile-enriched prompt is the evidence the
//! spawn-side resolution ran.

use serde_json::json;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const SPAWN: &str = "harness::spawn";

const LEAD_PROFILE: &str = "---
name: Lead
description: Orchestrator test profile.
---
You are the integration lead. Delegate the task to your coder and report.
";

const CODER_PROFILE: &str = "---
name: Coder
description: Implementation leaf test profile.
icon: code
leaf: true
---
Do the one task you are given, then stop.
";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-026";
    const MESSAGE: &str = "Have the coder say hello.";

    let model = Model::scripted("fixture-model");

    let coder_args = json!({
        "task": "Say hello, then stop.",
        "agent": "coder",
        "session_id": "{{run_id}}-coder"
    });

    Scenario::new(
        ID,
        "agent-identity",
        "A send running as a directory agent profile (no functions policy on the wire) \
         delegates through harness::spawn: the leaf profile seeds the child with its own \
         enriched identity.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-026")
            .agent("lead")
            .omit_functions(),
    )
    .agent_file("lead.md", LEAD_PROFILE)
    .agent_file("coder.md", CODER_PROFILE)
    // The child's opening step races the parent's post-spawn step — every
    // generation here is uniquely matchable (step + prompt), so dispatch by
    // match instead of arrival order.
    .match_any_dispatch()
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    // DEFAULT identity enriched with the resolved profile.
                    .system_prompt_regex("(?s)# System rules.*You are Lead\\.")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-coder",
                SPAWN,
                coder_args,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    // Sub-agent identity enriched with the LEAF profile.
                    .system_prompt_regex("(?s)You are an iii sub-agent.*You are Coder\\.")
                    .messages_subset([json!({ "role": "user" })])
                    .tools_subset([]),
            )
            .respond(Response::text("hello from the coder", 10, 2)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("You are Lead\\.")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-coder", "function_id": SPAWN }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-coder",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("delegated to the coder", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["delegated to the coder"])?;
        run.expect_no_duplicate_messages()
    })
    .build()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_declares_the_profile_files_and_the_child_generation() {
        let fixture = scenario();
        fixture.validate().unwrap();
        // One primary terminal turn; the coder's turn runs in its own session
        // and chains into the send's trace (spawned from inside the turn).
        assert_eq!(fixture.expected_turn_statuses, ["completed"]);
        assert_eq!(fixture.expected_traces(), 1);
        // Three generations: two parent (spawn, final), one child.
        assert_eq!(fixture.script.generations.len(), 3);
        assert_eq!(fixture.agent_files.len(), 2);
        // The send carries the agent id and NO functions policy — the
        // harness-side default is part of what this scenario pins.
        assert_eq!(fixture.scenario.send.options.agent.as_deref(), Some("lead"));
        assert!(fixture.scenario.send.options.functions.is_none());
    }
}
