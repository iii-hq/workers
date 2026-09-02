//! `workflow::inject-guidance` — a `pre_generate` hook that contributes the
//! workflow orchestration guidance to the agent's system prompt, ONLY while this
//! worker is connected. The hook is bound at worker startup and the binding is
//! dropped when the worker goes away, so the guidance is presence-gated for free:
//! a deployment without the workflow worker never pays for it, and it is no longer
//! hand-duplicated (and drifting) across the four static prompt variants.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const GUIDANCE_HOOK_ID: &str = "workflow::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends workflow orchestration guidance to the agent \
     system prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The orchestration guidance appended to the system prompt — the single canonical
/// copy (previously duplicated, and drifting, across default/anthropic/gpt/kimi).
/// It is pure USAGE guidance: the hook only fires when the workflow worker is
/// present, so the old "look for it / install it" discovery text is unnecessary.
pub(crate) const WORKFLOW_GUIDANCE: &str = "When a task is a MULTI-AGENT PIPELINE you could diagram before starting — fan-out (one sub-agent per item of a list), barrier/join (wait for several sub-agents, then synthesize), or a multi-stage / multi-round flow such as draft → adversarial critique → revise — prefer the workflow worker over orchestrating sub-agents by hand. A workflow is a declarative DAG of agents the orchestrator drives to completion, giving you fan-out, barrier joins, retries, result-collection, and crash-resumability for free — the run survives even if this session ends. Call `workflow::start` — it returns a run_id immediately. To get the result: pass `reply_to:{}`/`notify` and then END YOUR TURN — do NOT claim the result was delivered or guess one, it arrives as a separate message when the run finishes. Never poll `workflow::status` in a loop. Fetch the contract via `engine::functions::info { function_id: \"workflow::start\" }` (it carries the WorkflowDef shape) before its first call in this session. A JOIN node that needs more than one upstream's output must read them ALL — set `input.from` to an ARRAY of `\"node:<id>\"` refs (e.g. `[\"node:draft\",\"node:reviews\"]`), which arrive as one object keyed by node id; `depends_on` only orders execution, so a dep you list but don't read is dropped (and rejected at `workflow::start`). Node tools INHERIT BY DEFAULT: a node that omits `agent.functions` inherits your reach MINUS workflow's own control plane (`workflow::*`, `configuration::*`, `approval::*`, harness hooks, and the `router::chat`/`router::complete` generate calls), so you rarely set it. Set a node's `agent.functions` only to NARROW it (e.g. `[\"web::fetch\"]` for least privilege) or `{ \"allow\": [] }` to lock it down. Reserve one-off sub-agent spawns for work you reason between step by step, not for fan-out or multi-stage orchestration you would otherwise hand-roll.";

/// The `pre_generate` hook envelope slice we read; the generation context is nested
/// under `generate`.
// Lenient: every other envelope field is ignored. See harness `HookRunner::run_pre_generate`.
#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct PreGenerateEvent {
    #[serde(default)]
    pub generate: GenerateContext,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct GenerateContext {
    /// The system prompt assembled so far (base + any prior hook's mutation).
    #[serde(default)]
    pub system_prompt: String,
}

/// Hook envelope returned to the harness: the mutations to apply to the
/// generation.
#[derive(Debug, Serialize, JsonSchema)]
pub struct PreGenerateResponse {
    pub mutations: PreGenerateMutations,
}

/// The harness applies `system_prompt` only when the key is present
/// (`HookRunner`'s `parse_mutations`), so `None` serializes to an empty
/// object: the safe no-op that preserves the harness's assembled prompt.
#[derive(Debug, Default, Serialize, JsonSchema)]
pub struct PreGenerateMutations {
    /// Full replacement system prompt (base + appended guidance). The harness
    /// overwrites, it does not merge.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt: Option<String>,
}

/// Build the `pre_generate` mutations for a given base prompt. Pure, so it's
/// unit-testable.
///
/// Returns NO `system_prompt` when `base` is empty. A missing or renamed
/// `generate.system_prompt` field deserializes to `""` (schema drift), and a
/// fail-open hook must PRESERVE the harness's assembled prompt, never replace
/// it with the guidance alone (fp's hook established this rule). For a real,
/// non-empty base we append the guidance and return the FULL prompt (the
/// harness overwrites, it does not merge).
fn mutations_for(base: &str) -> PreGenerateMutations {
    if base.is_empty() {
        PreGenerateMutations::default()
    } else {
        PreGenerateMutations {
            system_prompt: Some(format!("{base}\n\n{WORKFLOW_GUIDANCE}")),
        }
    }
}

/// `pre_generate` hook entrypoint: return a `system_prompt` mutation that appends
/// the workflow guidance. Bound `fail_open`, so an error here never blocks a turn.
pub async fn handle(
    event: PreGenerateEvent,
) -> Result<PreGenerateResponse, iii_sdk::errors::Error> {
    Ok(PreGenerateResponse {
        mutations: mutations_for(&event.generate.system_prompt),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_guidance_after_a_real_base() {
        let m = mutations_for("BASE PROMPT");
        let out = m
            .system_prompt
            .expect("a non-empty base yields a system_prompt mutation");
        assert!(
            out.starts_with("BASE PROMPT\n\n"),
            "the base prompt must be preserved, guidance appended after it"
        );
        assert!(
            out.contains("workflow::start"),
            "guidance content is present"
        );
        assert!(
            out.contains("END YOUR TURN"),
            "delivery guidance is present"
        );
    }

    #[test]
    fn empty_base_emits_no_system_prompt_mutation() {
        // A missing/malformed hook payload (system_prompt absent → "") must
        // PRESERVE the harness prompt: emit no system_prompt key so the
        // harness keeps its own, rather than replacing the whole prompt with
        // the guidance alone. The wire shape must stay `{"mutations": {}}` —
        // the harness applies system_prompt only when the key is present.
        let wire = serde_json::to_value(PreGenerateResponse {
            mutations: mutations_for(""),
        })
        .expect("response serializes");
        assert_eq!(wire, serde_json::json!({ "mutations": {} }));
    }

    /// Mirrors the registry publish gate (`collect_worker_interface.py`): the
    /// derived response schema must carry a schema-defining keyword, not the
    /// AnyValue schema.
    #[test]
    fn response_schema_passes_the_publish_typed_gate() {
        let schema = schemars::r#gen::SchemaSettings::draft07()
            .into_generator()
            .into_root_schema_for::<PreGenerateResponse>();
        let value = serde_json::to_value(schema).expect("schema serializes");
        let obj = value.as_object().expect("schema is an object");
        assert!(
            ["type", "properties", "$ref"]
                .iter()
                .any(|k| obj.contains_key(*k)),
            "PreGenerateResponse schema is untyped: {value}"
        );
    }
}
