//! OpenTelemetry baggage helpers — correlate Telegram ingress with harness turns.

use serde_json::{json, Value};

/// Metadata forwarded on `harness::send` `options.metadata` for trace propagation.
pub fn tracing_metadata(session_id: &str, message_id: &str) -> Value {
    json!({
        "session_id": session_id,
        "message_id": message_id,
        "surface": "telegram",
    })
}

/// Run an async block with session/turn baggage for harness binding handlers.
pub async fn with_session_baggage<F, Fut, T>(
    deps: &crate::deps::Deps,
    session_id: &str,
    entry_id: Option<&str>,
    f: F,
) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let message_id = message_id_for_binding(deps, session_id, entry_id);
    with_baggage(session_id, &message_id, f).await
}

fn message_id_for_binding(
    deps: &crate::deps::Deps,
    session_id: &str,
    entry_id: Option<&str>,
) -> String {
    deps.runtime
        .active_turns
        .get(session_id)
        .map(|r| r.clone())
        .or_else(|| entry_id.map(String::from))
        .unwrap_or_else(|| session_id.to_string())
}

/// Run an async block with `iii.session.id` / `iii.message.id` stamped into baggage.
pub async fn with_baggage<F, Fut, T>(session_id: &str, message_id: &str, f: F) -> T
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = T>,
{
    let baggage = [
        ("iii.session.id", session_id),
        ("iii.message.id", message_id),
    ];
    iii_observability::run_with_baggage(&baggage, f()).await
}

/// Correlation id for a Telegram update (matches harness idempotency / metadata).
pub fn telegram_message_id(update_id: i64) -> String {
    format!("tg-{update_id}")
}

/// Session id for tracing before the harness session exists for a chat.
pub fn pending_session_id(chat_id: i64) -> String {
    format!("pending-{chat_id}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracing_metadata_shape() {
        let meta = tracing_metadata("s1", "tg-42");
        assert_eq!(meta["session_id"], "s1");
        assert_eq!(meta["message_id"], "tg-42");
        assert_eq!(meta["surface"], "telegram");
    }

    #[test]
    fn telegram_message_id_format() {
        assert_eq!(telegram_message_id(123), "tg-123");
    }
}
