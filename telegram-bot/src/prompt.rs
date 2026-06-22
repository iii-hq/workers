//! Telegram channel-context prompt. The bot enriches the harness's built-in
//! system prompt with this so the agent knows it is talking to a Telegram user:
//! its reply text is the only thing the user sees, there is no visible console,
//! and reaching the user later requires a scheduled callback into this session.

use crate::config::ModelRef;

/// The channel-context template, with `{chat_id}` / `{session_id}` / `{model}`
/// placeholders substituted at send time.
const CHANNEL_CONTEXT: &str = include_str!("../prompts/channel-context.txt");

/// Build the Telegram channel-context prompt for a specific chat/session.
pub fn channel_context(chat_id: i64, session_id: &str, model: &ModelRef) -> String {
    CHANNEL_CONTEXT
        .replace("{chat_id}", &chat_id.to_string())
        .replace("{session_id}", session_id)
        .replace("{model}", &model.id)
}

/// A fresh Telegram session id (`s_<uuid>`), minted bot-side so the channel
/// context can carry a real session id from the very first message — letting
/// the agent target this session for reminders/callbacks.
pub fn new_session_id() -> String {
    format!("s_{}", uuid::Uuid::new_v4().simple())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> ModelRef {
        ModelRef {
            provider: "anthropic".into(),
            id: "claude-sonnet-4".into(),
        }
    }

    #[test]
    fn channel_context_substitutes_placeholders() {
        let out = channel_context(42, "s_abc", &model());
        assert!(out.contains("chat_id: 42"));
        assert!(out.contains("session_id: s_abc"));
        assert!(out.contains("model: claude-sonnet-4"));
        // No unsubstituted placeholders remain.
        assert!(!out.contains("{chat_id}"));
        assert!(!out.contains("{session_id}"));
        assert!(!out.contains("{model}"));
    }

    #[test]
    fn channel_context_mentions_no_visible_console() {
        let out = channel_context(1, "s_1", &model());
        assert!(out.contains("console.log"));
        assert!(out.contains("telegram-bot::notify"));
    }

    #[test]
    fn new_session_id_is_prefixed_and_unique() {
        let a = new_session_id();
        let b = new_session_id();
        assert!(a.starts_with("s_"));
        assert_ne!(a, b);
    }
}
