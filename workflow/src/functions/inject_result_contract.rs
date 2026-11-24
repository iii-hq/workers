//! `workflow::inject-result-contract` — the pre_generate hook that puts a JSON
//! node's result contract in its system prompt on every generation of the
//! node's live attempt, so its opening message is only the task and the input.
//!
//! The worker observes `harness::hook::on-pre-generate` for node sessions
//! (`configuration::observe_result_contract_hook`). The hook finds the
//! session's node (`validate_result::node_output`) and appends
//! [`result_contract`] to the prompt it received. It is bound
//! `on_error: fail_open` and every failed lookup leaves the prompt alone:
//! `workflow::validate-result` holds the settling turn to the contract and
//! repeats the schema on a deny.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::functions::Deps;
use crate::functions::inject_guidance::{
    GenerateContext, PreGenerateMutations, PreGenerateResponse,
};

pub const RESULT_CONTRACT_ID: &str = "workflow::inject-result-contract";
pub const RESULT_CONTRACT_DESC: &str = "Internal pre_generate hook on workflow node sessions: appends a JSON node's result \
     contract (deliver with workflow-result::submit, and the output schema) to its system \
     prompt. Not called directly.";

/// The slice of the pre_generate hook input this hook reads. The observer
/// lists `generate.system_prompt`, so the harness sends the envelope and that
/// field only.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ResultContractEvent {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub generate: GenerateContext,
}

/// The result contract of a JSON node whose output contract is `output`. The
/// node runs under the harness's text contract, so the model gets no
/// `submit_result` tool; `workflow::validate-result` holds the turn that
/// settles the node to this. Pure.
pub(crate) fn result_contract(output: &Value) -> String {
    let schema = match crate::reconcile::output_schema(output) {
        Some(schema) => format!(
            "\nThe value must match this JSON Schema:\n{}",
            serde_json::to_string(schema).unwrap_or_default()
        ),
        None => String::new(),
    };
    format!(
        "Result of your task: deliver it by calling workflow-result::submit once with \
         {{\"result\": <your result as a JSON value>}}. An error names what to fix: call it \
         again. Once it is accepted, end the turn with one short line, not the JSON: the node \
         is then finished; if a wake is still armed, this session and its background jobs are \
         stopped, so submit last. A turn that ends before the result is submitted, waiting on \
         a wake you armed with harness::observe, may end with a status line instead.{schema}"
    )
}

/// `base` with `contract` appended, or no mutation: without a contract, and
/// for an empty `base` (a missing `generate.system_prompt` reads as `""`, and
/// the harness overwrites, so the contract alone would replace the prompt).
/// Appended, never prepended: the harness's prompt cache split needs the
/// prompt to keep its head. Pure.
fn mutations_for(base: &str, contract: Option<&str>) -> PreGenerateMutations {
    match contract {
        Some(contract) if !base.is_empty() => PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{contract}")),
        },
        _ => PreGenerateMutations::default(),
    }
}

pub async fn handle(
    deps: &Deps,
    event: ResultContractEvent,
) -> Result<PreGenerateResponse, iii_sdk::errors::Error> {
    let base = event.generate.system_prompt.as_str();
    // An empty base changes nothing: skip the reads.
    let contract = if base.is_empty() {
        None
    } else {
        // `submitted` is not read: the contract stays for the whole attempt.
        super::validate_result::node_output(deps, &event.session_id)
            .await
            .map(|(output, _)| result_contract(&output))
    };
    Ok(PreGenerateResponse {
        mutations: mutations_for(base, contract.as_deref()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_contract_names_submit_the_schema_and_the_parking_rule() {
        let contract = result_contract(&json!({
            "type": "json",
            "schema": {"type": "object", "required": ["verdict"]}
        }));
        assert!(
            contract.starts_with(
                "Result of your task: deliver it by calling workflow-result::submit once with \
                 {\"result\": "
            ),
            "{contract}"
        );
        assert!(
            contract.contains(r#"{"required":["verdict"],"type":"object"}"#),
            "{contract}"
        );
        assert!(contract.contains("harness::observe"), "{contract}");

        // Without a schema the contract asks for any JSON value.
        for output in [
            json!({"type": "json"}),
            json!({"type": "json", "schema": null}),
        ] {
            let contract = result_contract(&output);
            assert!(
                contract.contains("workflow-result::submit") && !contract.contains("JSON Schema"),
                "{contract}"
            );
        }
    }

    #[test]
    fn the_contract_is_appended_after_the_prompt_it_received() {
        assert_eq!(
            mutations_for("BASE", Some("C")).system_prompt.as_deref(),
            Some("BASE\n\nC")
        );
    }

    #[test]
    fn no_contract_or_an_empty_base_leaves_the_prompt_alone() {
        for mutations in [mutations_for("BASE", None), mutations_for("", Some("C"))] {
            assert_eq!(
                serde_json::to_value(PreGenerateResponse { mutations }).unwrap(),
                json!({"mutations": {}})
            );
        }
    }

    #[test]
    fn the_event_reads_the_projected_hook_input() {
        let event: ResultContractEvent = serde_json::from_value(json!({
            "point": "pre_generate",
            "session_id": "wf_r_1_j@r0",
            "turn_id": "t",
            "step": 0,
            "depth": 0,
            "own_namespace": "main",
            "generate": {"system_prompt": "BASE"}
        }))
        .unwrap();
        assert_eq!(event.session_id, "wf_r_1_j@r0");
        assert_eq!(event.generate.system_prompt, "BASE");

        let empty: ResultContractEvent = serde_json::from_value(json!({})).unwrap();
        assert!(empty.session_id.is_empty() && empty.generate.system_prompt.is_empty());
    }
}
