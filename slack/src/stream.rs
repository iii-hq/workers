//! Stream assistant output back into Slack using native streaming
//! (`chat.startStream`/`appendStream`/`stopStream`), with a `chat.update`
//! fallback for workspaces without AI-app streaming.

use iii_sdk::errors::Error;
use serde_json::{json, Value};

use crate::clients::slack;
use crate::deps::{Deps, StreamState};
use crate::kv;

/// Handle a streamed assistant text revision for a session.
pub async fn on_assistant_text(deps: &Deps, session_id: &str, text: &str) -> Result<(), Error> {
    if text.is_empty() {
        return Ok(());
    }

    // Ensure we have a stream state seeded from the session's target.
    if !deps.runtime.streams.contains_key(session_id) {
        let Some(target) = kv::session_target(deps, session_id).await else {
            return Ok(());
        };
        deps.runtime.streams.insert(
            session_id.to_string(),
            StreamState {
                channel: target.channel,
                thread_ts: target.thread_ts,
                recipient_user_id: target.recipient_user_id,
                ts: None,
                last_text: String::new(),
            },
        );
    }

    // Clone the current snapshot (avoid holding the dashmap guard across awaits).
    let mut st = deps
        .runtime
        .streams
        .get(session_id)
        .map(|r| r.clone())
        .unwrap();

    if st.ts.is_none() {
        match start_stream(deps, &st).await {
            Ok(ts) => {
                st.ts = Some(ts);
                st.last_text = String::new();
            }
            Err(e) => {
                // Fallback: create a normal message and edit it as text grows.
                tracing::warn!(error = %e, "startStream failed; using chat.update fallback");
                let ts = post_message(deps, &st, text).await?;
                st.ts = Some(format!("update:{ts}"));
                st.last_text = text.to_string();
                deps.runtime.streams.insert(session_id.to_string(), st);
                return Ok(());
            }
        }
    }

    let ts = st.ts.clone().unwrap();
    if let Some(real_ts) = ts.strip_prefix("update:") {
        update_message(deps, &st, real_ts, text).await?;
        st.last_text = text.to_string();
    } else {
        let delta = delta(&st.last_text, text);
        if !delta.is_empty() {
            append_stream(deps, &st, &ts, &delta).await?;
            st.last_text = text.to_string();
        }
    }
    deps.runtime.streams.insert(session_id.to_string(), st);
    Ok(())
}

/// Finalize the stream at turn end.
pub async fn finalize(deps: &Deps, session_id: &str) -> Result<(), Error> {
    let Some((_, st)) = deps.runtime.streams.remove(session_id) else {
        return Ok(());
    };
    if let Some(ts) = &st.ts {
        if let Some(real_ts) = ts.strip_prefix("update:") {
            // Already a persistent message; nothing to finalize.
            let _ = real_ts;
        } else {
            let _ = slack::call(
                deps,
                "chat.stopStream",
                json!({ "channel": st.channel, "message_ts": ts, "thread_ts": st.thread_ts }),
            )
            .await;
        }
    }
    // Best-effort clear of the assistant-thread status (no-op outside assistant threads).
    let _ = slack::call(
        deps,
        "assistant.threads.setStatus",
        json!({ "channel_id": st.channel, "thread_ts": st.thread_ts, "status": "" }),
    )
    .await;
    Ok(())
}

async fn start_stream(deps: &Deps, st: &StreamState) -> Result<String, Error> {
    let mut params = json!({ "channel": st.channel, "thread_ts": st.thread_ts });
    if let Some(uid) = &st.recipient_user_id {
        params["recipient_user_id"] = json!(uid);
    }
    let resp = slack::call(deps, "chat.startStream", params).await?;
    resp.get("ts")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Handler("chat.startStream: missing ts".into()))
}

async fn append_stream(deps: &Deps, st: &StreamState, ts: &str, delta: &str) -> Result<(), Error> {
    slack::call(
        deps,
        "chat.appendStream",
        json!({
            "channel": st.channel,
            "message_ts": ts,
            "thread_ts": st.thread_ts,
            "chunks": [{ "type": "markdown_text", "text": delta }],
        }),
    )
    .await?;
    Ok(())
}

async fn post_message(deps: &Deps, st: &StreamState, text: &str) -> Result<String, Error> {
    let resp = slack::call(
        deps,
        "chat.postMessage",
        json!({ "channel": st.channel, "thread_ts": st.thread_ts, "markdown_text": text }),
    )
    .await?;
    resp.get("ts")
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Handler("chat.postMessage: missing ts".into()))
}

async fn update_message(deps: &Deps, st: &StreamState, ts: &str, text: &str) -> Result<(), Error> {
    slack::call(
        deps,
        "chat.update",
        json!({ "channel": st.channel, "ts": ts, "markdown_text": text }),
    )
    .await?;
    Ok(())
}

/// New suffix of `cur` beyond `prev` (or the whole string on a reset).
fn delta(prev: &str, cur: &str) -> String {
    match cur.strip_prefix(prev) {
        Some(suffix) => suffix.to_string(),
        None => cur.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_returns_suffix() {
        assert_eq!(delta("hel", "hello"), "lo");
        assert_eq!(delta("", "hi"), "hi");
        assert_eq!(delta("abc", "abc"), "");
    }

    #[test]
    fn delta_handles_reset() {
        assert_eq!(delta("abc", "xyz"), "xyz");
    }
}
