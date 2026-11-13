//! `workflow::validate-result` — the post_turn hook that holds a JSON node to
//! delivering its result, without the harness's `submit_result` tool.
//!
//! A JSON node is sent with a text contract and an opening that asks it to
//! deliver the value with `workflow-result::submit` (`tick::json_output_instruction`,
//! `submit`). This hook observes `harness::hook::on-post-turn` for node sessions
//! (`configuration::observe_post_turn_hook`): a turn whose attempt submitted
//! passes; otherwise a final message that is itself a valid JSON value passes
//! (the older answer style), and anything else is denied with the reason, so the
//! harness re-prompts the turn, at most `max_validation_retries` times. A turn that ends with a wake armed (`harness::status` `expects_wake`)
//! is a parking turn and passes whatever it says; the woken turn that settles
//! the node is checked. Reconcile parses and checks the settled result again
//! (`reconcile::json_node_outcome`), so this hook only corrects: it is bound
//! `on_error: fail_open`, and every failed lookup lets the turn complete.

use std::future::Future;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::functions::Deps;
use crate::reconcile::{correction_reason, json_node_result, output_schema, NOT_SUBMITTED};

pub const VALIDATE_RESULT_ID: &str = "workflow::validate-result";
pub const VALIDATE_RESULT_DESC: &str =
    "Internal post_turn hook on workflow node sessions: a JSON node's turn that delivered no \
     result with workflow-result::submit (and whose final message is not a valid result either) \
     is denied, so the harness re-prompts it; a turn that ends with a wake armed passes. Not \
     called directly.";

/// The corrective prompt the harness sends on a deny (`{reason}` is the deny reason).
pub const RETRY_PROMPT: &str =
    "This workflow node is not settled: {reason}. Deliver the result by \
     calling workflow-result::submit with {\"result\": <the JSON value the node's output schema \
     requires>}, then end the turn with one short line. If you are still waiting on work, arm a \
     wake with harness::observe before ending the turn.";

/// The slice of the post_turn hook input this hook reads.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct PostTurnEvent {
    #[serde(default)]
    pub session_id: String,
    /// The turn's result: the final message's text under a text contract.
    #[serde(default)]
    pub result: Value,
}

/// The hook decision returned to the harness.
#[derive(Debug, PartialEq, Serialize, JsonSchema)]
pub struct ValidateResultResponse {
    /// `continue`, or `deny` to re-prompt the turn.
    pub decision: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

pub async fn handle(
    deps: &Deps,
    event: PostTurnEvent,
) -> Result<ValidateResultResponse, iii_sdk::errors::Error> {
    let reason = match node_output(deps, &event.session_id).await {
        Some((_, true)) => None,
        Some((output, false)) => {
            deny_reason(&output, event.result, || {
                expects_wake(deps, &event.session_id)
            })
            .await
        }
        None => None,
    };
    Ok(ValidateResultResponse {
        decision: if reason.is_some() { "deny" } else { "continue" }.to_string(),
        reason,
    })
}

/// Why the turn's result does not settle a JSON node with contract `output`, or
/// `None` when it passes. `expects_wake` is asked only for an invalid result.
async fn deny_reason<W, Wf>(output: &Value, result: Value, expects_wake: W) -> Option<String>
where
    W: FnOnce() -> Wf,
    Wf: Future<Output = bool>,
{
    let error = json_node_result(output_schema(output), result).err()?;
    // A parking turn ends on a status line; the woken turn that settles the node answers.
    if expects_wake().await {
        return None;
    }
    Some(correction_reason(
        output,
        format!("{NOT_SUBMITTED}, and the final message is not the value either: {error}"),
    ))
}

/// The JSON output contract of the live node attempt running in `session_id`
/// and whether that attempt submitted its result, or `None` (a text node, a
/// stale attempt, a finished run, not a node session, or a failed read).
async fn node_output(deps: &Deps, session_id: &str) -> Option<(Value, bool)> {
    if !crate::functions::wake::is_workflow_session(session_id) {
        return None;
    }
    let lookup = async {
        let Some(run_id) = crate::state::run_id_for_session(&deps.iii, session_id).await? else {
            return Ok(None);
        };
        let Some(record) = crate::state::get_run(&deps.iii, &run_id).await? else {
            return Ok(None);
        };
        let found = record.nodes.iter().find_map(|(uid, cp)| {
            (cp.is_live() && cp.session_id.as_deref() == Some(session_id))
                .then_some((uid, cp.submitted))
        });
        let Some((uid, submitted)) = found.filter(|_| !record.status.is_terminal()) else {
            return Ok(None);
        };
        let def = crate::state::get_def(&deps.iii, &run_id).await?;
        let base_id = uid.split('#').next().unwrap_or(uid);
        Ok::<_, crate::error::WorkflowError>(def.and_then(|def| {
            let output = def.nodes.get(base_id)?.agent.json_output()?.clone();
            Some((output, submitted))
        }))
    };
    lookup.await.unwrap_or_else(|e| {
        tracing::warn!(session_id, error = %e, "validate-result: node lookup failed; passing the turn");
        None
    })
}

/// Whether the session still holds an armed wake. A failed read passes the turn.
async fn expects_wake(deps: &Deps, session_id: &str) -> bool {
    let timeout_ms = deps.cfg().await.dispatch_timeout_ms;
    match deps
        .iii
        .call(iii_sdk::CallRequest {
            function_id: "harness::status".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(timeout_ms),
        })
        .await
    {
        Ok(status) => status["expects_wake"].as_bool().unwrap_or(false),
        Err(e) => {
            tracing::warn!(session_id, error = %e, "validate-result: harness::status failed; passing the turn");
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output() -> Value {
        json!({"type": "json", "schema": {"type": "object", "required": ["answer"]}})
    }

    /// The deny reason and whether `expects_wake` was asked.
    async fn reason(result: Value, armed: bool) -> (Option<String>, bool) {
        let asked = std::cell::Cell::new(false);
        let reason = deny_reason(&output(), result, || {
            asked.set(true);
            std::future::ready(armed)
        })
        .await;
        (reason, asked.get())
    }

    #[tokio::test]
    async fn a_valid_answer_passes_without_reading_the_session() {
        assert_eq!(
            reason(json!(r#"{"answer": "FINAL"}"#), false).await,
            (None, false)
        );
        assert_eq!(
            reason(json!("```json\n{\"answer\": 1}\n```"), false).await,
            (None, false)
        );
    }

    #[tokio::test]
    async fn an_invalid_settling_answer_is_denied_with_the_cause() {
        let (prose, asked) = reason(json!("all done!"), false).await;
        assert!(asked);
        let prose = prose.expect("denied");
        assert!(
            prose.starts_with(NOT_SUBMITTED) && prose.contains("the answer is not a JSON value"),
            "{prose}"
        );

        let (miss, _) = reason(json!(r#"{"other": 1}"#), false).await;
        let miss = miss.expect("denied");
        assert!(
            miss.contains("output schema") && miss.contains("answer"),
            "{miss}"
        );
        // The schema rides on every deny: the opening that carried it may be compacted away.
        let schema = output()["schema"].to_string();
        assert!(
            prose.ends_with(&schema) && miss.ends_with(&schema),
            "{prose} / {miss}"
        );
    }

    #[tokio::test]
    async fn a_parking_turn_passes_whatever_it_says() {
        assert_eq!(reason(json!("waiting on job_x"), true).await, (None, true));
    }

    #[test]
    fn the_response_is_the_hook_wire() {
        let deny = ValidateResultResponse {
            decision: "deny".into(),
            reason: Some("r".into()),
        };
        assert_eq!(
            serde_json::to_value(deny).unwrap(),
            json!({"decision": "deny", "reason": "r"})
        );
        let pass = ValidateResultResponse {
            decision: "continue".into(),
            reason: None,
        };
        assert_eq!(
            serde_json::to_value(pass).unwrap(),
            json!({"decision": "continue"})
        );
        assert!(RETRY_PROMPT.contains("{reason}"));
    }
}
