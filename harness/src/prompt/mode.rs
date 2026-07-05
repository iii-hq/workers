//! Operating-mode paragraphs prepended before the identity prompt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Console / send operating mode — prepends a short paragraph before the
/// shared identity prompt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Plan,
    Ask,
    Agent,
    // Coding session: native tool exposure over the coder/shell surface.
    // Unlike the other modes this REPLACES the mesh identity prompt with
    // the coding identity (`prompts/code.txt`) — `paragraph` is unused.
    // (Plain comment on purpose: a doc comment would split the wire
    // schema's flat enum into a oneOf.)
    Code,
}

pub fn paragraph(mode: Mode) -> &'static str {
    match mode {
        Mode::Plan => {
            "You are operating in plan mode: investigate first, then produce a concise numbered plan.\n\
1. Investigate everything needed to fully plan — relevant functions, workers, and triggers — via `agent_trigger`.\n\
2. Ask the user about any ambiguity or uncertain decision until they are confident in the plan, before finalizing it.\n\
3. End the plan with a todo list of the actionable steps required to execute it."
        }
        Mode::Ask => {
            "You are operating in ask mode: answer the user directly and be concise (one or two paragraphs). Only call `agent_trigger` when strictly necessary to ground your answer."
        }
        Mode::Agent => {
            "You are operating in agent mode: use `agent_trigger` autonomously to satisfy the request. Stop when you have a final answer or hit an irrecoverable error."
        }
        // Never prepended: `build_system_prompt` returns the code identity
        // outright for this mode. Kept exhaustive for the compiler.
        Mode::Code => "",
    }
}
