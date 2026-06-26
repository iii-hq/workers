//! `slack::assistant::*` — assistant-thread UX (status, title, suggested prompts).
//! These are plain Web API methods; the harness bridge will drive them, but they
//! are also directly callable.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetStatusReq {
    pub channel_id: String,
    pub thread_ts: String,
    /// Status text, e.g. `"is thinking..."`. Empty string clears it.
    pub status: String,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetTitleReq {
    pub channel_id: String,
    pub thread_ts: String,
    pub title: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetSuggestedPromptsReq {
    pub channel_id: String,
    pub thread_ts: String,
    /// Up to 4 prompts: `[{ "title": "...", "message": "..." }]`.
    pub prompts: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

slack_method!(
    set_status,
    "slack::assistant::set-status",
    "assistant.threads.setStatus",
    "Set the assistant-thread status (the thinking indicator).",
    SetStatusReq
);
slack_method!(
    set_title,
    "slack::assistant::set-title",
    "assistant.threads.setTitle",
    "Set the assistant-thread title.",
    SetTitleReq
);
slack_method!(
    set_suggested_prompts,
    "slack::assistant::set-suggested-prompts",
    "assistant.threads.setSuggestedPrompts",
    "Set suggested prompts for an assistant thread (≤4).",
    SetSuggestedPromptsReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    set_status(iii, deps);
    set_title(iii, deps);
    set_suggested_prompts(iii, deps);
}
