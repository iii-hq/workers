//! `slack::views::*` — Block Kit modals and App Home.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct OpenReq {
    /// Interaction trigger_id (expires ~3s after the interaction).
    pub trigger_id: String,
    /// Block Kit view object.
    pub view: Value,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PublishReq {
    pub user_id: String,
    /// Block Kit view of type `home`.
    pub view: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct UpdateReq {
    pub view: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PushReq {
    pub trigger_id: String,
    pub view: Value,
}

slack_method!(
    open,
    "slack::views::open",
    "views.open",
    "Open a Block Kit modal.",
    OpenReq
);
slack_method!(
    publish,
    "slack::views::publish",
    "views.publish",
    "Publish the App Home tab for a user.",
    PublishReq
);
slack_method!(
    update,
    "slack::views::update",
    "views.update",
    "Update an open view.",
    UpdateReq
);
slack_method!(
    push,
    "slack::views::push",
    "views.push",
    "Push a new modal onto the stack.",
    PushReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    open(iii, deps);
    publish(iii, deps);
    update(iii, deps);
    push(iii, deps);
}
