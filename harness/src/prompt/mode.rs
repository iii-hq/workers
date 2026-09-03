//! Operating-mode paragraphs prepended before the identity prompt.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Operating mode prepended to the identity prompt; `ask` also caps the
/// dispatch policy at the configured default.
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
            "You are operating in agent mode: use `agent_trigger` autonomously to satisfy the request. Match the user's requested scope and level of detail; do not expand the task. Stop when you have a final answer or hit an irrecoverable error."
        }
    }
}
