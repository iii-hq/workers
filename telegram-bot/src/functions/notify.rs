//! `telegram-bot::notify` — push a message into a Telegram chat out-of-band.
//!
//! This is the agent-callable "talk to the user" verb. The agent's normal
//! reply is delivered reactively when a turn completes; `notify` lets a
//! scheduled job (a cron-bound reminder) or a long-running task deliver a
//! message to the user *without* a turn producing it.
//!
//! It is **session-scoped**: the caller names a `session_id`, and the message
//! goes only to the chat bound to that session. An agent therefore cannot push
//! to arbitrary chats — only to the conversation it belongs to.

use std::sync::Arc;

use iii_sdk::errors::Error;
use iii_sdk::IIIClient;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::clients::telegram;
use crate::deps::Deps;
use crate::kv;
use crate::render::{chunk, format};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct NotifyRequest {
    /// The session whose chat receives the message. Must be a session this bot
    /// created (e.g. the `session_id` from the channel-context prompt).
    pub session_id: String,
    /// The message text. Markdown is auto-formatted for Telegram unless
    /// `parse_mode` is set explicitly. Long text is split across messages.
    pub text: String,
    /// Optional Telegram `parse_mode` (`MarkdownV2` / `HTML`). When set, `text`
    /// is sent verbatim with that mode instead of being auto-formatted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parse_mode: Option<String>,
}

#[derive(Debug, Serialize, JsonSchema)]
pub struct NotifyResponse {
    pub delivered: bool,
    /// Telegram message ids of the (possibly chunked) messages sent.
    pub message_ids: Vec<i64>,
}

pub fn register(iii: &Arc<IIIClient>, deps: &Arc<Deps>) {
    super::register(
        iii,
        deps,
        super::NOTIFY_ID,
        "Send a message to the Telegram chat bound to a session. Use this to reach the user \
         out-of-band (e.g. a scheduled reminder or when a long task finishes); the message is \
         delivered only to that session's chat.",
        |d, req: NotifyRequest| async move { handle(&d, req).await },
    );
}

async fn handle(deps: &Deps, req: NotifyRequest) -> Result<NotifyResponse, Error> {
    if req.text.is_empty() {
        return Err(Error::Handler("notify: text is empty".into()));
    }

    let Some(chat_id) = kv::chat_id_for_session(deps, &req.session_id).await else {
        return Err(Error::Handler(format!(
            "notify: no Telegram chat is bound to session {}",
            req.session_id
        )));
    };

    let chunks = chunk::split_message(&req.text, chunk::TELEGRAM_MAX_MESSAGE_LEN);
    let mut message_ids = Vec::with_capacity(chunks.len());
    for piece in &chunks {
        let id = send_chunk(deps, chat_id, piece, req.parse_mode.as_deref()).await?;
        message_ids.push(id);
    }

    Ok(NotifyResponse {
        delivered: true,
        message_ids,
    })
}

/// Send one chunk. With an explicit `parse_mode`, send verbatim under that mode
/// and fall back to plain text if Telegram rejects it. Otherwise auto-format
/// the Markdown the same way streamed replies are formatted.
async fn send_chunk(
    deps: &Deps,
    chat_id: i64,
    text: &str,
    parse_mode: Option<&str>,
) -> Result<i64, Error> {
    let (body, mode): (String, Option<&str>) = match parse_mode {
        Some(explicit) => (text.to_string(), Some(explicit)),
        None => {
            let (formatted, mode) = format::format_outgoing(text);
            (formatted, mode)
        }
    };
    match telegram::send_message(deps, chat_id, &body, None, mode).await {
        Ok(id) => Ok(id),
        Err(e) if mode.is_some() => {
            tracing::warn!(error = %e, "notify: formatted send failed, retrying plain");
            telegram::send_message(deps, chat_id, text, None, None).await
        }
        Err(e) => Err(e),
    }
}
