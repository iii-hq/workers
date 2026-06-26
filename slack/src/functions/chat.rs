//! `slack::chat::*` — messaging methods.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PostMessageReq {
    /// Channel, DM, or user id to post to (e.g. `C0123ABC` or `U0123ABC`).
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Block Kit blocks (array). Mutually exclusive with `markdown_text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Value>,
    /// Markdown body (≤12000 chars). Mutually exclusive with `blocks`/`text`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown_text: Option<String>,
    /// Parent message ts to reply in a thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    /// Any other chat.postMessage parameter (unfurl_links, username, …).
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateReq {
    pub channel: String,
    /// ts of the message to edit.
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub markdown_text: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct DeleteReq {
    pub channel: String,
    pub ts: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PostEphemeralReq {
    pub channel: String,
    /// User who will see the ephemeral message.
    pub user: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ScheduleMessageReq {
    pub channel: String,
    /// Unix epoch seconds to post at.
    pub post_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocks: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_ts: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetPermalinkReq {
    pub channel: String,
    pub message_ts: String,
}

slack_method!(
    post_message,
    "slack::chat::post-message",
    "chat.postMessage",
    "Post a message to a channel, DM, or thread.",
    PostMessageReq
);
slack_method!(
    update,
    "slack::chat::update",
    "chat.update",
    "Edit an existing message by ts.",
    UpdateReq
);
slack_method!(
    delete,
    "slack::chat::delete",
    "chat.delete",
    "Delete a message by ts.",
    DeleteReq
);
slack_method!(
    post_ephemeral,
    "slack::chat::post-ephemeral",
    "chat.postEphemeral",
    "Post a message visible only to one user.",
    PostEphemeralReq
);
slack_method!(
    schedule_message,
    "slack::chat::schedule-message",
    "chat.scheduleMessage",
    "Schedule a message to post at a future time.",
    ScheduleMessageReq
);
slack_method!(
    get_permalink,
    "slack::chat::get-permalink",
    "chat.getPermalink",
    "Get a permalink URL for a message.",
    GetPermalinkReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    post_message(iii, deps);
    update(iii, deps);
    delete(iii, deps);
    post_ephemeral(iii, deps);
    schedule_message(iii, deps);
    get_permalink(iii, deps);
}
