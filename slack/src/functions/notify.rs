//! `slack::notify` — session-scoped out-of-band send. The agent reaches the
//! Slack thread bound to a session (e.g. a scheduled reminder), and only that
//! thread; it cannot push to arbitrary channels through this verb.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::clients::slack;
use crate::deps::Deps;
use crate::kv;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NotifyRequest {
    /// A session this bot created (its replies are bound to a Slack thread).
    pub session_id: String,
    /// Message text (markdown). Used unless `blocks` is provided.
    #[serde(default)]
    pub text: Option<String>,
    /// Optional Block Kit blocks.
    #[serde(default)]
    pub blocks: Option<Value>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NotifyResponse {
    pub delivered: bool,
    pub channel: String,
    pub ts: Option<String>,
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        "slack::notify",
        "Send a message to the Slack thread bound to a session (out-of-band, e.g. a reminder).",
        |d, req: NotifyRequest| async move { handle(&d, req).await },
    );
}

async fn handle(deps: &Deps, req: NotifyRequest) -> Result<NotifyResponse, Error> {
    let Some(target) = kv::session_target(deps, &req.session_id).await else {
        return Err(Error::Handler(format!(
            "notify: no Slack thread is bound to session {}",
            req.session_id
        )));
    };

    let mut params = json!({ "channel": target.channel, "thread_ts": target.thread_ts });
    match (&req.blocks, &req.text) {
        (Some(blocks), _) => {
            params["blocks"] = blocks.clone();
            if let Some(t) = &req.text {
                params["text"] = json!(t);
            }
        }
        (None, Some(text)) => {
            params["markdown_text"] = json!(text);
        }
        (None, None) => return Err(Error::Handler("notify: text or blocks required".into())),
    }

    let resp = slack::call(deps, "chat.postMessage", params).await?;
    Ok(NotifyResponse {
        delivered: true,
        channel: target.channel,
        ts: resp.get("ts").and_then(Value::as_str).map(str::to_string),
    })
}
