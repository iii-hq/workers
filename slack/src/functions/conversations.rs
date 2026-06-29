//! `slack::conversations::*` — channel/DM management and history.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Default, Deserialize, Serialize, JsonSchema)]
pub struct ListReq {
    /// Comma-separated types: `public_channel,private_channel,mpim,im`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_archived: Option<bool>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ChannelReq {
    pub channel: String,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct HistoryReq {
    pub channel: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oldest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct RepliesReq {
    pub channel: String,
    /// Parent thread ts.
    pub ts: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CreateReq {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_private: Option<bool>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct InviteReq {
    pub channel: String,
    /// Comma-separated user ids.
    pub users: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OpenReq {
    /// Comma-separated user ids to open a DM/MPIM with.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetTopicReq {
    pub channel: String,
    pub topic: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct SetPurposeReq {
    pub channel: String,
    pub purpose: String,
}

slack_method!(
    list,
    "slack::conversations::list",
    "conversations.list",
    "List channels/DMs the bot can see.",
    ListReq
);
slack_method!(
    info,
    "slack::conversations::info",
    "conversations.info",
    "Get metadata for one conversation.",
    ChannelReq
);
slack_method!(
    history,
    "slack::conversations::history",
    "conversations.history",
    "Fetch a conversation's message history.",
    HistoryReq
);
slack_method!(
    replies,
    "slack::conversations::replies",
    "conversations.replies",
    "Fetch the replies in a thread.",
    RepliesReq
);
slack_method!(
    create,
    "slack::conversations::create",
    "conversations.create",
    "Create a channel.",
    CreateReq
);
slack_method!(
    invite,
    "slack::conversations::invite",
    "conversations.invite",
    "Invite users to a channel.",
    InviteReq
);
slack_method!(
    join,
    "slack::conversations::join",
    "conversations.join",
    "Join a public channel.",
    ChannelReq
);
slack_method!(
    members,
    "slack::conversations::members",
    "conversations.members",
    "List member ids of a conversation.",
    ChannelReq
);
slack_method!(
    open,
    "slack::conversations::open",
    "conversations.open",
    "Open/return a DM or MPIM channel.",
    OpenReq
);
slack_method!(
    set_topic,
    "slack::conversations::set-topic",
    "conversations.setTopic",
    "Set a channel topic.",
    SetTopicReq
);
slack_method!(
    set_purpose,
    "slack::conversations::set-purpose",
    "conversations.setPurpose",
    "Set a channel purpose.",
    SetPurposeReq
);
slack_method!(
    archive,
    "slack::conversations::archive",
    "conversations.archive",
    "Archive a channel.",
    ChannelReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    list(iii, deps);
    info(iii, deps);
    history(iii, deps);
    replies(iii, deps);
    create(iii, deps);
    invite(iii, deps);
    join(iii, deps);
    members(iii, deps);
    open(iii, deps);
    set_topic(iii, deps);
    set_purpose(iii, deps);
    archive(iii, deps);
}
