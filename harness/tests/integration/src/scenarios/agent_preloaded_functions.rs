//! INT-027 — agent-profile PRELOADED functions, end to end: a session that
//! starts as a directory profile declaring `functions:` gets those functions'
//! contracts frozen into its system prompt as one `<preloaded_functions>`
//! block, resolved by the real harness against the real engine registry —
//! the model can call them on its first step with no
//! `directory::search_functions` / `engine::functions::info` round-trip:
//!   * inheritance is ADDITIVE through `extends`, through the real directory
//!     binary: the engineer profile (extends: lead) renders the lead's
//!     `state::get` first, then its own `state::set` (root-first union), each
//!     exactly once;
//!   * each rendered contract carries the live description and request
//!     schema the engine serves (`state::set` names `scope`, `key`, `value`);
//!   * an id the engine does not know (`nope::missing`) is named as
//!     unavailable instead of failing the session or vanishing silently;
//!   * the block sits AFTER the resolved profile body (lead body, then the
//!     engineer body) and the send names no prompt fields — the profile is
//!     the whole identity, contracts included.
//!
//! The router matcher pins the shape in one regex; `verify` then reads the
//! raw system prompt back out of the router evidence and checks the parts
//! a regex states poorly (exactly-once headings, the schema's keys, the
//! unavailable line naming nothing else).

use serde_json::Value;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const LEAD_PROFILE: &str = "---
name: Lead
description: Preloaded-functions base profile.
functions:
  - state::get
---
You are the integration lead.
";

const ENGINEER_PROFILE: &str = "---
name: Engineer
description: Preloaded-functions leaf profile.
extends: lead
functions:
  - state::set
  - nope::missing
---
Do the one task you are given, then stop.
";

const BLOCK_OPEN: &str = "<preloaded_functions>";
const BLOCK_CLOSE: &str = "</preloaded_functions>";
const UNAVAILABLE_LINE: &str =
    "Declared by the profile but NOT registered right now — do not call: `nope::missing`.";

/// The resolved identity (lead body, engineer body), the block right after
/// it, the two contracts root-first, the live `state::set` schema, and the
/// unknown id named — all in the one request the send produces. The end is
/// not anchored: the harness's own runtime context may follow the block.
const PROMPT_REGEX: &str = "(?s)^You are the integration lead\\.\\n\\nDo the one task you are given, then stop\\.\\n\\n\
     <preloaded_functions>\\n.*### `state::get`\\n.*### `state::set`\\n.*request_schema: \\{.*\"scope\".*\
     NOT registered right now — do not call: `nope::missing`\\..*\\n</preloaded_functions>";

const REPLY: &str = "preloaded contracts acknowledged";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-027";
    const MESSAGE: &str = "Confirm which functions you can call right away.";

    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "agent-preloaded-functions",
        "A send running as a directory agent profile that declares `functions:` (its own plus \
         its parent's, through extends) receives every declared contract in a \
         <preloaded_functions> block of its frozen system prompt — live description and \
         request schema from the engine registry, root-first, each once — and an id the engine \
         does not know is named as unavailable instead of failing the session.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-027")
            .agent("engineer")
            .omit_functions(),
    )
    .agent_file("lead.md", LEAD_PROFILE)
    .agent_file("engineer.md", ENGINEER_PROFILE)
    .generation(
        Generation::new(1)
            .expect(
                Request::new()
                    .turn_request_step(0)
                    .system_prompt_regex(PROMPT_REGEX)
                    .messages_exact([Message::user(MESSAGE)])
                    .tools_subset([]),
            )
            .respond(Response::text(REPLY, 12, 3)),
    )
    .verify(|run| {
        run.expect_assistant_texts([REPLY])?;
        run.expect_no_duplicate_messages()?;

        let prompt = first_system_prompt(&run.router_evidence)?;
        anyhow::ensure!(
            prompt.matches(BLOCK_OPEN).count() == 1,
            "the block must appear exactly once in the system prompt"
        );
        let identity_end = prompt
            .find("Do the one task you are given, then stop.")
            .ok_or_else(|| anyhow::anyhow!("engineer body missing from the system prompt"))?;
        let block_start = prompt
            .find(BLOCK_OPEN)
            .ok_or_else(|| anyhow::anyhow!("{BLOCK_OPEN} missing from the system prompt"))?;
        anyhow::ensure!(
            identity_end < block_start,
            "the block must follow the resolved identity, never precede it"
        );
        let block = preloaded_block(&prompt)?;

        // Root-first union through `extends`, each contract exactly once.
        let headings: Vec<&str> = block
            .lines()
            .filter_map(|line| line.strip_prefix("### "))
            .collect();
        anyhow::ensure!(
            headings == ["`state::get`", "`state::set`"],
            "contract headings must be the parent's then the child's, once each; got {headings:?}"
        );

        // The live contract, not a placeholder: a description line and the
        // engine's request schema with its real property names.
        let set_section = block
            .split("### `state::set`")
            .nth(1)
            .ok_or_else(|| anyhow::anyhow!("no state::set section"))?;
        for needle in ["request_schema: {", "\"scope\"", "\"key\"", "\"value\""] {
            anyhow::ensure!(
                set_section.contains(needle),
                "state::set contract lacks {needle}: {set_section}"
            );
        }
        anyhow::ensure!(
            !set_section.contains("(none published"),
            "state::set must render the engine's schema, not the no-schema fallback"
        );

        // The unknown id is named as unavailable — and nothing else is.
        let unavailable = block
            .lines()
            .find(|line| line.starts_with("Declared by the profile but NOT registered"))
            .ok_or_else(|| anyhow::anyhow!("no unavailable line in the block: {block}"))?;
        anyhow::ensure!(
            unavailable.starts_with(UNAVAILABLE_LINE),
            "unavailable line must name exactly `nope::missing`; got: {unavailable}"
        );
        Ok(())
    })
    .build()
}

/// The system prompt of the first router call the scripted router recorded
/// (`router_evidence.calls[*].request.system_prompt`).
fn first_system_prompt(router_evidence: &Value) -> anyhow::Result<String> {
    router_evidence
        .get("calls")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|call| {
            call.get("request")?
                .get("system_prompt")?
                .as_str()
                .map(str::to_string)
        })
        .ok_or_else(|| anyhow::anyhow!("no router call carried a system prompt"))
}

/// The `<preloaded_functions>…</preloaded_functions>` block, tags included.
fn preloaded_block(prompt: &str) -> anyhow::Result<&str> {
    let start = prompt
        .find(BLOCK_OPEN)
        .ok_or_else(|| anyhow::anyhow!("{BLOCK_OPEN} missing"))?;
    let end = prompt
        .find(BLOCK_CLOSE)
        .ok_or_else(|| anyhow::anyhow!("{BLOCK_CLOSE} missing"))?;
    anyhow::ensure!(start < end, "malformed block: close tag before open tag");
    Ok(&prompt[start..end + BLOCK_CLOSE.len()])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the harness renders for this fixture's profiles (schemas abridged
    /// to the keys the scenario pins) — the regex must accept it, and reject
    /// the shapes the scenario exists to catch.
    const RENDERED: &str = "You are the integration lead.\n\nDo the one task you are given, then stop.\n\n\
        <preloaded_functions>\nThese functions are preloaded for this agent profile: …\n\n\
        ### `state::get`\nRead the value stored at a key in a state scope\nrequest_schema: \
        {\"properties\":{\"key\":{\"type\":\"string\"},\"scope\":{\"type\":\"string\"}},\"required\":[\"key\",\"scope\"],\"type\":\"object\"}\n\n\
        ### `state::set`\nStore or write a value at a key in a state scope\nrequest_schema: \
        {\"properties\":{\"key\":{\"type\":\"string\"},\"scope\":{\"type\":\"string\"},\"value\":{}},\"required\":[\"key\",\"scope\",\"value\"],\"type\":\"object\"}\n\n\
        Declared by the profile but NOT registered right now — do not call: `nope::missing`. If the task needs one of them, say so rather than improvising a substitute.\n\
        </preloaded_functions>\n\nWorking directory: /tmp/run";

    #[test]
    fn fixture_declares_the_chain_and_pins_the_prompt() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses, ["completed"]);
        assert_eq!(fixture.expected_traces(), 1);
        assert_eq!(fixture.script.generations.len(), 1);
        assert_eq!(fixture.agent_files.len(), 2);
        assert_eq!(
            fixture.scenario.send.options.agent.as_deref(),
            Some("engineer")
        );
        assert!(fixture.scenario.send.options.functions.is_none());
    }

    #[test]
    fn prompt_regex_accepts_the_rendered_shape_and_rejects_the_regressions() {
        let regex = regex::Regex::new(PROMPT_REGEX).unwrap();
        assert!(regex.is_match(RENDERED), "the rendered prompt must match");
        // No block at all — the regression that motivated this scenario.
        assert!(!regex.is_match(
            "You are the integration lead.\n\nDo the one task you are given, then stop.\n\nWorking directory: /tmp/run"
        ));
        // Replace-style inheritance: the parent's contract missing.
        assert!(!regex.is_match(&RENDERED.replace("### `state::get`\n", "")));
        // Child-first order.
        assert!(!regex.is_match(&RENDERED.replace("### `state::get`", "### `state::zzz`")));
        // Unknown id silently dropped instead of named.
        assert!(!regex.is_match(&RENDERED.replace("`nope::missing`", "`other::thing`")));
        // Block before the identity.
        assert!(!regex.is_match(&format!(
            "<preloaded_functions>\n</preloaded_functions>\n\n{RENDERED}"
        )));
    }

    #[test]
    fn helpers_read_the_block_out_of_router_evidence() {
        let block = preloaded_block(RENDERED).unwrap();
        assert!(block.starts_with(BLOCK_OPEN) && block.ends_with(BLOCK_CLOSE));
        assert!(preloaded_block("no block here").is_err());
        let evidence = serde_json::json!({ "calls": [
            { "request": { "system_prompt": RENDERED, "messages": [] } }
        ]});
        assert_eq!(first_system_prompt(&evidence).unwrap(), RENDERED);
        assert!(first_system_prompt(&serde_json::json!({ "calls": [] })).is_err());
    }
}
