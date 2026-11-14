//! `workflow-result::submit` — a JSON node delivers its result by calling this
//! function, so the value never has to be the final message of its turn.
//!
//! The pre-call hook (`stamp_reply`) stamps the calling session into the
//! arguments as `caller_session_id`, from the harness envelope, so a node
//! delivers only its own result. The value is checked against the node's
//! `agent.output.schema`; an invalid one is refused with the cause and the
//! schema, and the model calls again. A valid one is stored as the node's result
//! and the attempt's checkpoint is marked `submitted`; reconcile settles the node
//! with it when the turn ends. The id is outside `workflow::*`, which node
//! sessions are denied by default.

use iii_sdk::errors::Error;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::functions::Deps;
use crate::reconcile::{correction_reason, output_schema, oversized_result, validate_value};
use crate::types::{WorkflowDef, WorkflowRunRecord};

pub const SUBMIT_ID: &str = "workflow-result::submit";
pub const SUBMIT_DESC: &str =
    "Deliver this workflow node's result: call it once with {\"result\": <the JSON value your \
     node's output schema requires>}. An error names what to fix: call it again. Once it is \
     accepted, end your turn with one short line, not the JSON. Only a workflow node's own \
     session can call it.";

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SubmitRequest {
    /// The node's result: the JSON value its output schema requires.
    pub result: Value,
    /// Stamped from the calling session by the workflow's pre-call hook; a
    /// supplied value is overwritten.
    #[serde(default)]
    pub caller_session_id: Option<String>,
}

#[derive(Debug, PartialEq, Serialize, JsonSchema)]
pub struct SubmitResponse {
    pub accepted: bool,
    /// The node uid the result was stored for.
    pub node: String,
}

fn refuse(why: impl std::fmt::Display) -> Error {
    Error::Handler(format!("workflow-result::submit refused: {why}"))
}

/// The live attempt running in `session_id` and its JSON output contract, or
/// why the session may not submit. Pure.
pub(crate) fn submitting_node<'a>(
    record: &WorkflowRunRecord,
    def: &'a WorkflowDef,
    session_id: &str,
) -> Result<(String, &'a Value), String> {
    if record.status.is_terminal() {
        return Err("the run has already finished".into());
    }
    let uid = record
        .nodes
        .iter()
        .find_map(|(uid, cp)| {
            (cp.is_live() && cp.session_id.as_deref() == Some(session_id)).then(|| uid.clone())
        })
        .ok_or("this session is not a live attempt of a node of its run")?;
    let base_id = uid.split('#').next().unwrap_or(&uid);
    let output = def
        .nodes
        .get(base_id)
        .and_then(|node| node.agent.json_output())
        .ok_or("this node answers in text: end your turn with the answer instead")?;
    Ok((uid, output))
}

/// `result` checked against the node's contract: schema and size. Pure.
pub(crate) fn check_result(output: &Value, result: &Value) -> Result<(), String> {
    // A stored null reads back as "no result", so it could never settle the node.
    if result.is_null() {
        return Err("the result is null: deliver the value itself".into());
    }
    validate_value(output_schema(output), result).map_err(|e| correction_reason(output, e))?;
    if let Some(len) = oversized_result(result) {
        return Err(format!(
            "the result is {len} bytes, over the {}-byte cap",
            crate::reconcile::MAX_RESULT_BYTES
        ));
    }
    Ok(())
}

pub async fn handle(deps: &Deps, req: SubmitRequest) -> Result<SubmitResponse, Error> {
    let session_id = req
        .caller_session_id
        .filter(|s| crate::functions::wake::is_workflow_session(s))
        .ok_or_else(|| refuse("call it from the turn of a workflow node session"))?;
    let run_id = crate::state::run_id_for_session(&deps.iii, &session_id)
        .await?
        .ok_or_else(|| refuse("this session belongs to no known workflow run"))?;
    // The same lock tick and sweep hold, so a settle never reads a half-written submit.
    let _guard = deps.locks.guard(&run_id).await;
    let mut record = crate::state::get_run(&deps.iii, &run_id)
        .await?
        .ok_or_else(|| refuse("its run is gone"))?;
    let def = crate::state::get_def(&deps.iii, &run_id)
        .await?
        .ok_or_else(|| refuse("its run's definition is gone"))?;
    let (uid, output) = submitting_node(&record, &def, &session_id).map_err(refuse)?;
    check_result(output, &req.result).map_err(refuse)?;
    crate::state::put_node_result(&deps.iii, &run_id, &uid, &req.result).await?;
    if let Some(cp) = record.nodes.get_mut(&uid) {
        cp.submitted = true;
    }
    crate::state::put_run(&deps.iii, &record).await?;
    Ok(SubmitResponse {
        accepted: true,
        node: uid,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{NodeCheckpoint, NodeState, RunStatus};
    use serde_json::json;

    fn def() -> WorkflowDef {
        serde_json::from_value(json!({
            "nodes": {
                "j": {"agent": {"model": "m", "output": {"type": "json", "schema": {
                    "type": "object", "properties": {"n": {"type": "integer"}}, "required": ["n"]}}},
                      "input": {"from": "run_input"}},
                "t": {"agent": {"model": "m"}, "input": {"from": "run_input"}}
            },
            "output": {"from": ["node:j", "node:t"]}
        }))
        .unwrap()
    }

    fn record(nodes: &[(&str, NodeState, &str)]) -> WorkflowRunRecord {
        let mut r = WorkflowRunRecord {
            run_id: "r_1".into(),
            step: 0,
            status: RunStatus::Running,
            abort: false,
            def_ref: "r_1".into(),
            input: Value::Null,
            nodes: Default::default(),
            fanout_src: Default::default(),
            result: None,
            result_error: None,
            notify: None,
            caller_session_id: None,
            caller_fs_root: None,
            idem_key: None,
            created_at: 0,
            updated_at: 0,
        };
        for (uid, state, sid) in nodes {
            r.nodes.insert(
                uid.to_string(),
                NodeCheckpoint {
                    state: *state,
                    session_id: Some(sid.to_string()),
                    ..Default::default()
                },
            );
        }
        r
    }

    #[test]
    fn a_live_json_node_may_submit_and_finds_its_pass() {
        let d = def();
        let r = record(&[
            ("j", NodeState::Done, "wf_r_1_j@r0"),
            ("j#it1", NodeState::Running, "wf_r_1_j#it1@r0"),
        ]);
        let (uid, output) = submitting_node(&r, &d, "wf_r_1_j#it1@r0").unwrap();
        assert_eq!(uid, "j#it1");
        assert_eq!(output["type"], "json");
        // A finished attempt cannot overwrite its result.
        assert!(submitting_node(&r, &d, "wf_r_1_j@r0").is_err());
    }

    #[test]
    fn text_nodes_strangers_and_finished_runs_are_refused() {
        let d = def();
        let r = record(&[("t", NodeState::Running, "wf_r_1_t@r0")]);
        assert!(submitting_node(&r, &d, "wf_r_1_t@r0")
            .unwrap_err()
            .contains("answers in text"));
        assert!(submitting_node(&r, &d, "wf_r_1_x@r0").is_err());
        let mut done = record(&[("j", NodeState::Running, "wf_r_1_j@r0")]);
        done.status = RunStatus::Completed;
        assert!(submitting_node(&done, &d, "wf_r_1_j@r0")
            .unwrap_err()
            .contains("finished"));
    }

    #[test]
    fn a_result_is_held_to_the_schema_and_a_string_is_not_parsed() {
        let output = json!({"type": "json", "schema": {"type": "object",
            "properties": {"n": {"type": "integer"}}, "required": ["n"]}});
        assert!(check_result(&output, &json!({"n": 7})).is_ok());
        let err = check_result(&output, &json!({"n": "7"})).unwrap_err();
        assert!(err.contains("Required JSON Schema"), "{err}");
        // A submitted string is a string, not JSON text to parse.
        assert!(check_result(&output, &json!("{\"n\": 7}")).is_err());
        assert!(check_result(&json!({"type": "json", "schema": null}), &json!("any")).is_ok());
        assert!(check_result(&json!({"type": "json"}), &Value::Null)
            .unwrap_err()
            .contains("null"));
    }
}
