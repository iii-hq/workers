//! UI-008 — the Console's limit card re-runs a turn that hit a Claude Code
//! account's usage limit on another account (chat-projects §4.3, D139,
//! D142, D147).
//!
//! Driven by Playwright (`ade/web/e2e/claude-accounts.spec.ts`), which
//! stands in for the tmux worker the stack does not run: it registers
//! `tmux::accounts::list` (and the rest of tmux's account functions) with
//! iii-browser-sdk, offering `default` ("Work") and `personal`, the global.
//! The fixture registers no stand-in of its own, so one function id has one
//! owner; without Playwright's tmux the turn resolves no account and the
//! first generation does not match.
//!
//! A setup call creates the project `acme-web` over the send's folder,
//! following the global, so the conversation joins it (D8) and its turn
//! resolves `personal`. The router answers that account with its five-hour
//! usage limit: the turn fails with `llm.usage_limit` naming the account.
//! The CONSOLE then does the rest — "Continue in Work" on the limit card
//! writes the conversation's `claude_account` (`session::patch-meta`) and
//! calls `harness::retry` — and the re-run turn runs on `default`, in the
//! project, with the same user message. Its tokens go to the account
//! ledger under `default` and `acme-web`, which the accounts screen's
//! split reads (`harness::metrics::account-split`).
//!
//! `verify` is INT-048's, with these accounts.

use serde_json::json;

use super::ScenarioDriver;
use super::account_usage_limit_retry::{
    LIMIT_MESSAGE, MESSAGE, REPLY, RESETS_AT_MS, request, verify_limit_then_retry,
};
use super::dsl::{Generation, Model, Response, Scenario, Send};
use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

const ID: &str = "UI-008";
/// The send's folder, the project's.
const FOLDER: &str = "/ui-008/acme-web";
/// The global account in Playwright's tmux, where the turn hits the limit.
const LIMITED: &str = "personal";
/// The account the Console's "Continue in Work" pins the conversation to.
const OTHER: &str = "default";
/// The re-run turn's tokens, 5:1 like the outside spend the spec's tmux
/// reports, so the split's shares do not depend on the price.
const INPUT_TOKENS: u64 = 30;
const OUTPUT_TOKENS: u64 = 6;

pub(super) fn scenario() -> ScenarioFixture {
    Scenario::new(
        ID,
        "console-account-usage-limit",
        "A Console conversation in a project following the global Claude Code account hits that \
         account's usage limit: the turn fails with llm.usage_limit naming it, then the limit \
         card's \"Continue in\" pins the conversation to another account and re-runs the turn \
         there (harness::retry) with the same user message.",
        ScenarioDriver::Playground,
        Model::claude_code("claude-opus-4-5"),
    )
    .setup_call(
        "harness::projects::create",
        json!({ "name": "acme-web", "color": "blue", "folders": [FOLDER] }),
        json!({ "project": { "name": "acme-web" } }),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:ui-008")
            .metadata(json!({ "fs_scope": { "root": FOLDER } })),
    )
    .terminal_turn_statuses(["failed", "completed"])
    .generation(
        Generation::new(1)
            .expect(request(LIMITED))
            .respond(Response::usage_limit(
                LIMIT_MESSAGE,
                "five_hour",
                RESETS_AT_MS,
            )),
    )
    .generation(
        Generation::new(2)
            .expect(request(OTHER))
            .respond(Response::text(REPLY, INPUT_TOKENS, OUTPUT_TOKENS)),
    )
    // The send's trace and the retry's: a metadata write seeds no turn.
    .expect_traces(2)
    // The spec walks the card, the details, the footer, the accounts screen
    // and a project's account after the turns: the playground stays up for
    // the whole Playwright test.
    .scenario_timeout_ms(120_000)
    .verify(verify)
    .build()
}

fn verify(run: &RunEvidence) -> anyhow::Result<()> {
    verify_limit_then_retry(run, LIMITED, OTHER)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_the_limit_then_the_consoles_retry() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.driver, ScenarioDriver::Playground);
        assert_eq!(fixture.slug, "console-account-usage-limit");
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.expected_turn_statuses, ["failed", "completed"]);
        assert_eq!(fixture.script.generations.len(), 2);
        assert_eq!(fixture.expected_traces(), 2);
        // Playwright owns tmux and the Console the retry: no stand-in, no probe.
        assert!(fixture.stand_in_functions.is_empty());
        assert!(fixture.probe_actions.is_empty());
        assert_eq!(fixture.setup_calls.len(), 1);
        assert_eq!(
            fixture.scenario.send.options.metadata.as_ref().unwrap()["fs_scope"]["root"],
            FOLDER
        );
        let limited = serde_json::to_string(&fixture.script.generations[0]).unwrap();
        assert!(limited.contains("\"usage_limit\":true"), "{limited}");
        assert!(limited.contains(LIMITED), "{limited}");
        let retried = serde_json::to_string(&fixture.script.generations[1]).unwrap();
        assert!(
            retried.contains(&format!("\"account\":\"{OTHER}\"")) && retried.contains(REPLY),
            "{retried}"
        );
    }
}
