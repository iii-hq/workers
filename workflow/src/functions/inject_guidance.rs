//! `workflow::inject-guidance` — a `pre_generate` hook that contributes the
//! workflow orchestration guidance to the agent's system prompt, ONLY while this
//! worker is connected. The hook is bound at worker startup and the binding is
//! dropped when the worker goes away, so the guidance is presence-gated for free:
//! a deployment without the workflow worker never pays for it, and it is no longer
//! hand-duplicated (and drifting) across the four static prompt variants.

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

pub const GUIDANCE_HOOK_ID: &str = "workflow::inject-guidance";
pub const GUIDANCE_HOOK_DESC: &str =
    "Internal pre_generate hook: appends workflow orchestration guidance to the agent \
     system prompt. Bound to harness::hook::pre-generate at worker startup; not called directly.";

/// The orchestration guidance appended to the system prompt — the single canonical
/// copy (previously duplicated, and drifting, across default/anthropic/gpt/kimi).
/// It is pure USAGE guidance: the hook only fires when the workflow worker is
/// present, so the old "look for it / install it" discovery text is unnecessary.
const WORKFLOW_GUIDANCE: &str = "When a task is a MULTI-AGENT PIPELINE you could diagram before starting — fan-out (one sub-agent per item of a list), barrier/join (wait for several sub-agents, then synthesize), or a multi-stage / multi-round flow such as draft → adversarial critique → revise — prefer the workflow worker over orchestrating sub-agents by hand. A workflow is a declarative DAG of agents the orchestrator drives to completion, giving you fan-out, barrier joins, retries, result-collection, and crash-resumability for free — the run survives even if this session ends. Call `workflow::start` — it returns a run_id immediately. To get the result: pass `reply_to:{}`/`notify` and then END YOUR TURN — do NOT claim the result was delivered or guess one, it arrives as a separate message when the run finishes. Never poll `workflow::status` in a loop. Fetch the contract via `engine::functions::info { function_id: \"workflow::start\" }` (it carries the WorkflowDef shape) before its first call in this session. A JOIN node that needs more than one upstream's output must read them ALL — set `input.from` to an ARRAY of `\"node:<id>\"` refs (e.g. `[\"node:draft\",\"node:reviews\"]`), which arrive as one object keyed by node id; `depends_on` only orders execution, so a dep you list but don't read is dropped (and rejected at `workflow::start`). Node tools INHERIT BY DEFAULT: a node that omits `agent.functions` inherits your reach MINUS workflow's own control plane (`workflow::*`, `configuration::*`, `approval::*`, harness hooks, and the `router::chat`/`router::complete` generate calls), so you rarely set it. Set a node's `agent.functions` only to NARROW it (e.g. `[\"web::fetch\"]` for least privilege) or `{ \"allow\": [] }` to lock it down. Reserve one-off sub-agent spawns for work you reason between step by step, not for fan-out or multi-stage orchestration you would otherwise hand-roll.";

/// The slice of the `pre_generate` hook envelope we read (lenient: ignores every
/// other field the harness sends). The harness nests the live generation context
/// under `generate` (see harness `HookRunner::run_pre_generate`).
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

/// Append the workflow guidance to the base prompt. Pure, so it's unit-testable.
/// The harness OVERWRITES `system_prompt` with what we return (it does not merge),
/// so we must return the FULL prompt (base + guidance), not just the addition.
fn enrich(base: &str) -> String {
    if base.is_empty() {
        WORKFLOW_GUIDANCE.to_string()
    } else {
        format!("{base}\n\n{WORKFLOW_GUIDANCE}")
    }
}

/// `pre_generate` hook entrypoint: return a `system_prompt` mutation that appends
/// the workflow guidance. Bound `fail_open`, so an error here never blocks a turn.
pub async fn handle(event: PreGenerateEvent) -> Result<Value, iii_sdk::errors::Error> {
    Ok(json!({ "mutations": { "system_prompt": enrich(&event.generate.system_prompt) } }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enrich_appends_guidance_after_base() {
        let out = enrich("BASE PROMPT");
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
    fn enrich_handles_empty_base() {
        // A missing/empty base must not produce a leading blank block.
        assert_eq!(enrich(""), WORKFLOW_GUIDANCE);
    }
}
