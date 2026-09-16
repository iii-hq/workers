//! INT-031 — shared prompt prefix across sessions (MOT-4798): two sessions
//! running as the same directory profile send the router byte-identical
//! stable prefixes — `system_sections[0]` (the frozen profile prompt plus the
//! frozen skills index, `cache_boundary: true`) and the
//! `cache_intent.surface_digest` over it — while the per-session tail
//! (`system_sections[1]`: session id, working directory, policy aid) differs,
//! and the flat `system_prompt` every legacy reader sees is exactly the
//! sections joined with a blank line.
//!
//! Shape: a lead send spawns a peer running as the SAME profile
//! (`agent: lead`), so the peer's opening generation and the parent's are two
//! independent sessions on one profile, racing like INT-026.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const SPAWN: &str = "harness::spawn";

const LEAD_PROFILE: &str = "---
name: Lead
description: Shared-prefix test profile.
---
You are the integration lead. Delegate the task to a peer lead and report.
";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-031";
    const MESSAGE: &str = "Have a peer lead say hello.";

    let model = Model::scripted("fixture-model");

    let peer_args = json!({
        "task": "Say hello, then stop.",
        "agent": "lead",
        "session_id": "{{run_id}}-peer"
    });

    Scenario::new(
        ID,
        "agent-shared-prompt-prefix",
        "Two sessions running as the same directory profile send the router byte-identical \
         stable prompt sections and surface digests and differ only in the per-session tail; \
         the flat system_prompt stays the sections joined with a blank line.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-031")
            .agent("lead")
            .omit_functions(),
    )
    .agent_file("lead.md", LEAD_PROFILE)
    // The peer's opening step races the parent's post-spawn step — every
    // generation is uniquely matchable (the parent's opener by its exact
    // message, the peer's by its session id in the runtime aid).
    .match_any_dispatch()
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex("(?s)^You are the integration lead\\.")
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::function_call_raw(
                "call-peer",
                SPAWN,
                peer_args,
                8,
                4,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(
                        "(?s)^You are the integration lead\\..*Your session id is {{run_id}}-peer\\.",
                    )
                    .messages_subset([json!({ "role": "user" })])
                    .tools_subset([]),
            )
            .respond(Response::text("hello from the peer", 10, 2)),
    )
    .generation(
        Generation::new(3)
            .expect(
                Request::new()
                    .turn_request_step(1)
                    .system_prompt_regex("^You are the integration lead\\.")
                    .messages_subset([
                        json!({ "role": "user" }),
                        json!({ "role": "assistant", "content": [
                            { "type": "function_call", "id": "call-peer", "function_id": SPAWN }
                        ] }),
                        json!({ "role": "function_result", "function_call_id": "call-peer",
                                "is_error": false }),
                    ])
                    .tools_subset([]),
            )
            .respond(Response::text("delegated to the peer", 10, 2)),
    )
    .verify(|run| {
        run.expect_assistant_texts(["delegated to the peer"])?;
        run.expect_no_duplicate_messages()?;

        let openers = step_zero_requests(&run.router_evidence);
        anyhow::ensure!(
            openers.len() == 2,
            "expected the parent's and the peer's opening requests, got {}",
            openers.len()
        );
        let mut heads = std::collections::BTreeSet::new();
        let mut digests = std::collections::BTreeSet::new();
        let mut tails = std::collections::BTreeSet::new();
        for request in &openers {
            let sections = request["system_sections"]
                .as_array()
                .ok_or_else(|| anyhow::anyhow!("no system_sections on the request: {request}"))?;
            anyhow::ensure!(
                sections.len() == 2,
                "two sections expected (stable | session), got {}",
                sections.len()
            );
            anyhow::ensure!(
                sections[0]["cache_boundary"] == json!(true)
                    && sections[1]["cache_boundary"] == json!(false),
                "the boundary sits after the stable section only"
            );
            let head = sections[0]["text"].as_str().unwrap_or_default();
            let tail = sections[1]["text"].as_str().unwrap_or_default();
            let digest = request["cache_intent"]["surface_digest"]
                .as_str()
                .unwrap_or_default();
            anyhow::ensure!(
                digest == format!("sha256:{:x}", Sha256::digest(head.as_bytes())),
                "surface_digest must be the sha256 of the stable section"
            );
            anyhow::ensure!(
                request["system_prompt"].as_str() == Some(format!("{head}\n\n{tail}").as_str()),
                "the flat system_prompt must be the sections joined with a blank line"
            );
            anyhow::ensure!(
                tail.starts_with("Your session id is "),
                "the session tail starts with the runtime aid; got: {tail}"
            );
            heads.insert(head.to_string());
            digests.insert(digest.to_string());
            tails.insert(tail.to_string());
        }
        anyhow::ensure!(
            heads.len() == 1 && digests.len() == 1,
            "both sessions must share one stable prefix and one digest"
        );
        anyhow::ensure!(
            tails.len() == 2,
            "the per-session tail must differ between the two sessions"
        );
        Ok(())
    })
    .build()
}

/// The raw router requests of every step-0 generation (`request_id` ends in
/// `:0`): each session's opening call.
fn step_zero_requests(router_evidence: &Value) -> Vec<Value> {
    router_evidence
        .get("calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|call| call.get("request").cloned())
        .filter(|request| {
            request["request_id"]
                .as_str()
                .is_some_and(|id| id.ends_with(":0"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_declares_one_profile_and_the_peer_generation() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses, ["completed"]);
        assert_eq!(fixture.expected_traces(), 1);
        // Three generations: two parent (spawn, final), one peer.
        assert_eq!(fixture.script.generations.len(), 3);
        assert_eq!(fixture.agent_files.len(), 1);
        assert_eq!(fixture.scenario.send.options.agent.as_deref(), Some("lead"));
    }
}
