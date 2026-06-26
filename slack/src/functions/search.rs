//! `slack::search::*` — requires a user token (`xoxp-`); bot tokens cannot search.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::deps::Deps;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct MessagesReq {
    pub query: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    #[serde(flatten, default)]
    pub extra: Map<String, Value>,
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        "slack::search::messages",
        "Search messages (requires a configured user_token).",
        |d, req: MessagesReq| async move {
            let params = serde_json::to_value(&req)
                .map_err(|e| iii_sdk::errors::Error::Handler(format!("serialize search: {e}")))?;
            crate::clients::slack::call_user(&d, "search.messages", params).await
        },
    );
}
