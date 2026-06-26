//! `slack::pins::*` and `slack::bookmarks::*`.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;
use crate::slack_method;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PinReq {
    pub channel: String,
    /// Message ts to pin/unpin.
    pub timestamp: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct PinsListReq {
    pub channel: String,
}

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct BookmarkAddReq {
    pub channel_id: String,
    pub title: String,
    /// Bookmark type, e.g. `link`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

slack_method!(
    pins_add,
    "slack::pins::add",
    "pins.add",
    "Pin a message to a channel.",
    PinReq
);
slack_method!(
    pins_list,
    "slack::pins::list",
    "pins.list",
    "List a channel's pinned items.",
    PinsListReq
);
slack_method!(
    bookmarks_add,
    "slack::bookmarks::add",
    "bookmarks.add",
    "Add a bookmark to a channel.",
    BookmarkAddReq
);

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    pins_add(iii, deps);
    pins_list(iii, deps);
    bookmarks_add(iii, deps);
}
