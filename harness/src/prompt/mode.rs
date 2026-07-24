//! Operating-mode paragraphs prepended before the identity prompt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Console / send operating mode — prepends a short paragraph before the
/// shared identity prompt. `ask` is also structural: the turn's dispatch
/// policy is capped at the configured default policy, never widened.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Ask,
    Agent,
}

pub fn paragraph(mode: Mode) -> &'static str {
    match mode {
        Mode::Ask => {
            "You are operating in ask mode: answer the user directly and be concise (one or two paragraphs). Only call `agent_trigger` when strictly necessary to ground your answer."
        }
        Mode::Agent => {
            "You are operating in agent mode: use `agent_trigger` autonomously to satisfy the request. Stop when you have a final answer or hit an irrecoverable error."
        }
    }
}
