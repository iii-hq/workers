//! INT-028 — agent-profile PRELOADED skills, end to end: a session that
//! starts as a directory profile declaring `skills:` gets those skills'
//! BODIES frozen into its system prompt as one `<preloaded_skills>` block of
//! `<skill id="…">` sections, resolved by the real harness against the real
//! directory binary — the skill is in context on the first step, never
//! something the model has to `directory::skills::get`:
//!   * inheritance is ADDITIVE through `extends`: the engineer profile
//!     (extends: lead) renders the lead's `harness/alpha` first, then its
//!     own `harness/beta` (root-first union), each exactly once;
//!   * each section carries the skill's live body from the run's
//!     `skills/harness/*.md` fixtures (frontmatter stripped);
//!   * an id the directory cannot serve (`harness/missing`) is named as
//!     unavailable instead of failing the session or vanishing silently;
//!   * a skill the profile does NOT declare (`harness/gamma`) gets no
//!     section — and the profile never narrows the session's skills index:
//!     when the harness rendered one, `harness/gamma` is still listed;
//!   * the block sits AFTER the resolved profile body (lead body, then the
//!     engineer body) and the send names no prompt fields.
//!
//! The router matcher pins the shape in one regex; `verify` then reads the
//! raw system prompt back out of the router evidence and checks the parts
//! a regex states poorly (exactly-once sections, their bodies, the
//! unavailable line naming nothing else, the undeclared skill's absence).

use serde_json::Value;

use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use super::ScenarioDriver;
use crate::fixtures::ScenarioFixture;

const LEAD_PROFILE: &str = "---
name: Lead
description: Preloaded-skills base profile.
skills:
  - harness/alpha
---
You are the integration lead.
";

const ENGINEER_PROFILE: &str = "---
name: Engineer
description: Preloaded-skills leaf profile.
extends: lead
skills:
  - harness/beta
  - harness/missing
---
Do the one task you are given, then stop.
";

const ALPHA_SKILL: &str = "---
title: Alpha
---
# Alpha

Alpha procedure: greet before anything else.
";

const BETA_SKILL: &str = "---
title: Beta
---
# Beta

Beta procedure: end every reply with a checksum.
";

const GAMMA_SKILL: &str = "---
title: Gamma
---
# Gamma

Gamma procedure: never preloaded, only indexed.
";

const BLOCK_OPEN: &str = "<preloaded_skills>";
const BLOCK_CLOSE: &str = "</preloaded_skills>";
const UNAVAILABLE_LINE: &str =
    "Declared by the profile but NOT available right now — do not look for them: `harness/missing`.";

/// The resolved identity (lead body, engineer body), the block right after
/// it, the two skill sections root-first with their bodies, and the unknown
/// id named — all in the one request the send produces. The end is not
/// anchored: the skills index and the harness's own runtime context follow.
const PROMPT_REGEX: &str = "(?s)^You are the integration lead\\.\\n\\nDo the one task you are given, then stop\\.\\n\\n\
     <preloaded_skills>\\n.*<skill id=\"harness/alpha\">\\n.*Alpha procedure: greet before anything else\\.\\n</skill>\
     .*<skill id=\"harness/beta\">\\n.*Beta procedure: end every reply with a checksum\\.\\n</skill>\
     .*NOT available right now — do not look for them: `harness/missing`\\..*\\n</preloaded_skills>";

const REPLY: &str = "preloaded skills acknowledged";

pub(super) fn scenario() -> ScenarioFixture {
    const ID: &str = "INT-028";
    const MESSAGE: &str = "Confirm which skills are already loaded.";

    let model = Model::scripted("fixture-model");

    Scenario::new(
        ID,
        "agent-preloaded-skills",
        "A send running as a directory agent profile that declares `skills:` (its own plus \
         its parent's, through extends) receives every declared skill's BODY in a \
         <preloaded_skills> block of its frozen system prompt — live from the directory, \
         root-first, each once — an id the directory cannot serve is named as unavailable, \
         and an undeclared skill is neither preloaded nor hidden from the skills index.",
        ScenarioDriver::Direct,
        model.clone(),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-028")
            .agent("engineer")
            .omit_functions(),
    )
    .agent_file("lead.md", LEAD_PROFILE)
    .agent_file("engineer.md", ENGINEER_PROFILE)
    .skill_file("harness/alpha.md", ALPHA_SKILL)
    .skill_file("harness/beta.md", BETA_SKILL)
    .skill_file("harness/gamma.md", GAMMA_SKILL)
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

        // Root-first union through `extends`, each skill exactly once, and
        // the undeclared one absent.
        let sections: Vec<&str> = block
            .lines()
            .filter_map(|line| line.strip_prefix("<skill id=\""))
            .filter_map(|rest| rest.strip_suffix("\">"))
            .collect();
        anyhow::ensure!(
            sections == ["harness/alpha", "harness/beta"],
            "skill sections must be the parent's then the child's, once each; got {sections:?}"
        );
        anyhow::ensure!(
            block.matches("</skill>").count() == 2,
            "every section must close exactly once"
        );

        // The live body, frontmatter stripped: the file's H1 is in, its
        // `title:` key is not.
        let beta = block
            .split("<skill id=\"harness/beta\">")
            .nth(1)
            .and_then(|rest| rest.split("</skill>").next())
            .ok_or_else(|| anyhow::anyhow!("no harness/beta section"))?;
        anyhow::ensure!(
            beta.contains("# Beta")
                && beta.contains("Beta procedure: end every reply with a checksum."),
            "harness/beta section lacks its body: {beta}"
        );
        anyhow::ensure!(
            !beta.contains("title: Beta"),
            "the section must carry the body, not the frontmatter: {beta}"
        );

        // The unknown id is named as unavailable — and nothing else is.
        let unavailable = block
            .lines()
            .find(|line| line.starts_with("Declared by the profile but NOT available"))
            .ok_or_else(|| anyhow::anyhow!("no unavailable line in the block: {block}"))?;
        anyhow::ensure!(
            unavailable.starts_with(UNAVAILABLE_LINE),
            "unavailable line must name exactly `harness/missing`; got: {unavailable}"
        );

        // Preloading is not curation: the skills index, when the harness had
        // a catalog to render, still lists the undeclared skill. (Whether the
        // index is present depends on the catalog seed racing the directory's
        // boot registration, so only its content is pinned.)
        if let Some(index) = skills_index(&prompt) {
            anyhow::ensure!(
                index.contains("harness/gamma"),
                "the profile must not narrow the skills index; harness/gamma missing from: {index}"
            );
        }
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

/// The `<preloaded_skills>…</preloaded_skills>` block, tags included.
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

/// The harness's `<available_skills>…</available_skills>` index, when the
/// prompt carries one.
fn skills_index(prompt: &str) -> Option<&str> {
    let start = prompt.find("<available_skills>")?;
    let end = prompt.find("</available_skills>")?;
    (start < end).then(|| &prompt[start..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What the harness renders for this fixture's profiles — the regex must
    /// accept it, and reject the shapes the scenario exists to catch.
    const RENDERED: &str = "You are the integration lead.\n\nDo the one task you are given, then stop.\n\n\
        <preloaded_skills>\nThese skills are preloaded for this agent profile: …\n\n\
        <skill id=\"harness/alpha\">\n# Alpha\n\nAlpha procedure: greet before anything else.\n</skill>\n\n\
        <skill id=\"harness/beta\">\n# Beta\n\nBeta procedure: end every reply with a checksum.\n</skill>\n\n\
        Declared by the profile but NOT available right now — do not look for them: `harness/missing`. If the task needs one of them, say so rather than improvising a substitute.\n\
        </preloaded_skills>\n\n<available_skills>\n- **harness/alpha** — Alpha.\n- **harness/beta** — Beta.\n- **harness/gamma** — Gamma.\n</available_skills>\n\nYour session id is s.";

    #[test]
    fn fixture_declares_the_chain_the_skills_and_pins_the_prompt() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_turn_statuses, ["completed"]);
        assert_eq!(fixture.expected_traces(), 1);
        assert_eq!(fixture.script.generations.len(), 1);
        assert_eq!(fixture.agent_files.len(), 2);
        assert_eq!(fixture.skill_files.len(), 3);
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
        // No block at all — skills listed by name only, bodies never loaded.
        assert!(!regex.is_match(
            "You are the integration lead.\n\nDo the one task you are given, then stop.\n\n<available_skills>\n- **harness/alpha** — Alpha.\n</available_skills>"
        ));
        // Replace-style inheritance: the parent's skill missing.
        assert!(!regex.is_match(
            &RENDERED.replace("<skill id=\"harness/alpha\">", "<skill id=\"harness/zzz\">")
        ));
        // A body-less section (id only) is not a preloaded skill.
        assert!(!regex
            .is_match(&RENDERED.replace("Alpha procedure: greet before anything else.\n", "")));
        // Unknown id silently dropped instead of named.
        assert!(!regex.is_match(&RENDERED.replace("`harness/missing`", "`other/thing`")));
        // Block before the identity.
        assert!(!regex.is_match(&format!(
            "<preloaded_skills>\n</preloaded_skills>\n\n{RENDERED}"
        )));
    }

    #[test]
    fn helpers_read_the_block_and_the_index_out_of_the_prompt() {
        let block = preloaded_block(RENDERED).unwrap();
        assert!(block.starts_with(BLOCK_OPEN) && block.ends_with(BLOCK_CLOSE));
        assert!(!block.contains("<available_skills>"));
        assert!(preloaded_block("no block here").is_err());
        assert!(skills_index(RENDERED).unwrap().contains("harness/gamma"));
        assert!(skills_index(block).is_none());
        let evidence = serde_json::json!({ "calls": [
            { "request": { "system_prompt": RENDERED, "messages": [] } }
        ]});
        assert_eq!(first_system_prompt(&evidence).unwrap(), RENDERED);
        assert!(first_system_prompt(&serde_json::json!({ "calls": [] })).is_err());
    }
}
