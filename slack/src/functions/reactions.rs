//! `slack::reactions::*` — emoji reactions.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct ReactionReq {
    pub channel: String,
    /// Message ts to react to.
    pub timestamp: String,
    /// Emoji name without colons (e.g. `thumbsup`).
    pub name: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct GetReq {
    pub channel: String,
    pub timestamp: String,
}

slack_method!(
    add,
    "slack::reactions::add",
    "reactions.add",
    "Add an emoji reaction to a message.",
    ReactionReq
);
slack_method!(
    remove,
    "slack::reactions::remove",
    "reactions.remove",
    "Remove an emoji reaction from a message.",
    ReactionReq
);
slack_method!(
    get,
    "slack::reactions::get",
    "reactions.get",
    "Get reactions on a message.",
    GetReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    add(iii, deps);
    remove(iii, deps);
    get(iii, deps);
}
