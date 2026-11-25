//! INT-048 — a Claude Code account's usage limit fails the turn once, and
//! `harness::retry` runs it again on another account (D139, D142, D147).
//!
//! A stand-in for the tmux worker lists two accounts, `work` (the global)
//! and `personal`. Before the send the runner creates a project over the
//! send's folder, pinned to `personal`, so the conversation joins it (D8)
//! and its turn resolves `personal`. The router answers that account with a
//! `rate_limited` whose `error_details` mark the usage limit: the turn fails
//! with `llm.usage_limit` naming the account, and neither the router nor the
//! harness retries it. The probe then does what "Continue in work" does — it
//! writes the conversation's `claude_account` and calls `harness::retry` —
//! and the new turn runs on `work` with the same user message, not a second
//! one.

use serde_json::{Value, json};

use super::ScenarioDriver;
use super::dsl::{Generation, Message, Model, Request, Response, Scenario, Send};
use crate::evidence_data::RunEvidence;
use crate::fixtures::ScenarioFixture;

const ID: &str = "INT-048";
const MESSAGE: &str = "Summarize the release notes.";
const REPLY: &str = "The release adds account pinning.";
const LIMIT_MESSAGE: &str = "Claude usage limit reached for the five-hour window";
const FOLDER: &str = "/int-048/acme";
const PINNED: &str = "personal";
const OTHER: &str = "work";
const RESETS_AT_MS: u64 = 4_102_444_800_000;

fn accounts() -> Value {
    json!({
        "available": true,
        "managed": true,
        "global": OTHER,
        "accounts": [{ "id": OTHER }, { "id": PINNED }]
    })
}

fn request(account: &str) -> Request {
    Request::new()
        .turn_request_step(0)
        // The project's folder is the turn's working directory: the runtime
        // aid in the system prompt names it.
        .system_prompt_regex("(?s).+")
        .messages_exact([Message::user(MESSAGE)])
        .without_tools()
        .provider_options_subset(json!({ "claude-code": { "account": account } }))
}

pub(super) fn scenario() -> ScenarioFixture {
    Scenario::new(
        ID,
        "account-usage-limit-retry",
        "A conversation in a project pinned to one Claude Code account hits that account's usage \
         limit: the turn fails once with llm.usage_limit naming the account and no retry, then \
         harness::retry, after the conversation names the other account, runs the turn on it \
         with the same user message.",
        ScenarioDriver::Direct,
        Model::claude_code("claude-opus-4-8"),
    )
    .stand_in_function("tmux::accounts::list", accounts())
    .setup_call(
        "harness::projects::create",
        json!({
            "name": "Acme",
            "color": "teal",
            "folders": [FOLDER],
            "account": { "mode": "pinned", "account_id": PINNED }
        }),
        json!({ "project": { "account": { "mode": "pinned", "account_id": PINNED } } }),
    )
    .send(
        Send::message(MESSAGE)
            .idempotency_key("{{run_id}}:integration-048")
            .metadata(json!({ "fs_scope": { "root": FOLDER } })),
    )
    .terminal_turn_statuses(["failed", "completed"])
    .generation(
        Generation::new(1)
            .expect(request(PINNED))
            .respond(Response::usage_limit(
                LIMIT_MESSAGE,
                "five_hour",
                RESETS_AT_MS,
            )),
    )
    .probe_after(
        1,
        "session::patch-meta",
        json!({ "session_id": "{{session_id}}", "set": { "claude_account": OTHER } }),
    )
    .probe_after(
        1,
        "harness::retry",
        json!({ "session_id": "{{session_id}}" }),
    )
    .generation(
        Generation::new(2)
            .expect(request(OTHER))
            .respond(Response::text(REPLY, 12, 6)),
    )
    // The send's trace and the retry's: a metadata write seeds no turn.
    .expect_traces(2)
    .verify(verify)
    .build()
}

fn verify(run: &RunEvidence) -> anyhow::Result<()> {
    run.expect_assistant_texts([REPLY])?;
    // One user message: the retry re-runs it, it does not send it again.
    let (users, _, _) = run.message_counts();
    anyhow::ensure!(users == 1, "expected one user message, found {users}");
    run.expect_no_duplicate_messages()?;

    let custom = |kind: &str| {
        run.transcript
            .iter()
            .filter(|item| item["custom"]["custom_type"] == kind)
            .map(|item| &item["custom"]["data"])
            .collect::<Vec<_>>()
    };
    // No transient resume: a usage limit is not retried.
    let recoveries = custom("recovery");
    anyhow::ensure!(
        recoveries.is_empty(),
        "a usage limit was resumed: {recoveries:?}"
    );
    let errors = custom("error");
    let [error] = errors.as_slice() else {
        anyhow::bail!("expected one failed turn: {errors:?}")
    };
    anyhow::ensure!(
        error["code"] == "llm.usage_limit"
            && error["retryable"] == false
            && error["recovery"]["attempted"] == 0
            && error["details"]
                == json!({
                    "usage_limit": true,
                    "window": "five_hour",
                    "resets_at_ms": RESETS_AT_MS,
                    "account": PINNED
                }),
        "unexpected usage-limit failure: {error}"
    );

    // The router saw the pinned account, then the one the conversation names.
    let accounts = run
        .step_zero_requests()
        .iter()
        .map(|request| request["provider_options"]["claude-code"]["account"].clone())
        .collect::<Vec<_>>();
    anyhow::ensure!(
        accounts == [json!(PINNED), json!(OTHER)],
        "unexpected accounts on the router requests: {accounts:?}"
    );

    // The retried turn is the run's last: the verbose status names its
    // account and where it came from; it completed without a failure.
    anyhow::ensure!(
        run.status["account"] == json!({ "id": OTHER, "source": "session" })
            && run.status["failure"].is_null(),
        "unexpected status after the retry: {}",
        run.status
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scripts_the_limit_then_the_retry_on_the_other_account() {
        let fixture = scenario();
        fixture.validate().unwrap();
        assert_eq!(fixture.expected_terminal_turns, 2);
        assert_eq!(fixture.expected_turn_statuses, ["failed", "completed"]);
        assert_eq!(fixture.script.generations.len(), 2);
        assert_eq!(fixture.expected_traces(), 2);
        assert_eq!(fixture.stand_in_functions.len(), 1);
        assert_eq!(fixture.setup_calls.len(), 1);
        assert_eq!(
            fixture.scenario.send.options.metadata.as_ref().unwrap()["fs_scope"]["root"],
            FOLDER
        );
        let limited = serde_json::to_value(&fixture.script.generations[0]).unwrap();
        let limited_text = limited.to_string();
        assert!(
            limited_text.contains("\"usage_limit\":true"),
            "{limited_text}"
        );
        assert!(
            limited_text.contains("\"error_kind\":\"rate_limited\""),
            "{limited_text}"
        );
        assert!(limited_text.contains(PINNED), "{limited_text}");
        let retried = serde_json::to_string(&fixture.script.generations[1]).unwrap();
        assert!(retried.contains(OTHER), "{retried}");
        let actions: Vec<_> = fixture
            .probe_actions
            .iter()
            .map(|action| (action.after_turns, action.function_id.as_str()))
            .collect();
        assert_eq!(actions, [(1, "session::patch-meta"), (1, "harness::retry")]);
    }
}
