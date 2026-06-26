//! `slack::call` — generic escape hatch. Call any Slack Web API method by name
//! with arbitrary params, so the worker covers the entire API on day one even
//! before a method has a typed wrapper.

use std::sync::Arc;

use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::deps::Deps;

#[derive(Debug, Deserialize, Serialize, JsonSchema)]
pub struct CallReq {
    /// Slack Web API method name, e.g. `chat.postMessage`, `team.info`.
    pub method: String,
    /// Method params object (defaults to `{}`).
    #[serde(default)]
    pub params: Option<Value>,
    /// Use the configured user token (`xoxp-`) instead of the bot token.
    #[serde(default)]
    pub as_user: bool,
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        "slack::call",
        "Call any Slack Web API method by name with arbitrary params (escape hatch).",
        |d, req: CallReq| async move {
            let params = req.params.unwrap_or_else(|| json!({}));
            let value = if req.as_user {
                crate::clients::slack::call_user(&d, &req.method, params).await?
            } else {
                crate::clients::slack::call(&d, &req.method, params).await?
            };
            Ok(crate::response::SlackResponse::from_value(value))
        },
    );
}
