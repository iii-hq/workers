//! Stream assistant output back into Slack using native streaming
//! (`chat.startStream`/`appendStream`/`stopStream`), with a `chat.update`
//! fallback for workspaces without AI-app streaming.
//!
//! All state transitions for a session are serialized by a per-session lock
//! (`RuntimeState::stream_lock`) and gated by `message-updated` revision, so
//! concurrent revision events cannot lose appended text or open two streams
//! (mirrors telegram-bot's per-entry lock + revision monotonicity).

use iii_sdk::errors::Error;
use serde_json::{json, Value};

use crate::clients::slack;
use crate::deps::{Deps, StreamState};
use crate::kv;

/// Handle a streamed assistant text revision for a session. `revision` is the
/// `message-updated` revision (0 for the initial `message-added`).
pub async fn on_assistant_text(
    deps: &Deps,
    session_id: &str,
    text: &str,
    revision: u64,
) -> Result<(), Error> {
    if text.is_empty() {
        return Ok(());
    }

    let lock = deps.runtime.stream_lock(session_id);
    let _guard = lock.lock().await;

    // Seed stream state from the session's target on first sight.
    if !deps.runtime.streams.contains_key(session_id) {
        let Some(target) = kv::session_target(deps, session_id).await else {
            return Ok(());
        };
        let is_dm = channel_is_dm(&target.channel);
        deps.runtime.streams.insert(
            session_id.to_string(),
            StreamState {
                channel: target.channel,
                thread_ts: target.thread_ts,
                recipient_user_id: if is_dm {
                    None
                } else {
                    target.recipient_user_id
                },
                recipient_team_id: if is_dm { None } else { target.team },
                is_dm,
                ts: None,
                last_text: String::new(),
                last_revision: 0,
            },
        );
    }

    let mut st = deps
        .runtime
        .streams
        .get(session_id)
        .map(|r| r.clone())
        .unwrap();

    // Drop stale/out-of-order revisions once streaming has started.
    if should_skip_revision(st.ts.is_some(), revision, st.last_revision) {
        return Ok(());
    }

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
                st.last_revision = st.last_revision.max(revision);
                deps.runtime.streams.insert(session_id.to_string(), st);
                return Ok(());
            }
        }
    }

    let ts = st.ts.clone().unwrap();
    if let Some(real_ts) = ts.strip_prefix("update:") {
        // Edit transport can replace the whole message, including a rewrite.
        update_message(deps, &st, real_ts, text).await?;
        st.last_text = text.to_string();
    } else {
        // Native append can only add text; a non-prefix rewrite can't be
        // retracted, so skip it rather than duplicate content.
        match delta(&st.last_text, text) {
            Delta::Append(suffix) if !suffix.is_empty() => {
                append_stream(deps, &st, &ts, &suffix).await?;
                st.last_text = text.to_string();
            }
            Delta::Append(_) => {}
            Delta::Reset => {
                tracing::warn!(
                    session_id,
                    "assistant text rewritten mid-stream; skipping append"
                );
            }
        }
    }
    st.last_revision = st.last_revision.max(revision);
    deps.runtime.streams.insert(session_id.to_string(), st);
    Ok(())
}

/// Finalize the stream at turn end.
pub async fn finalize(deps: &Deps, session_id: &str) -> Result<(), Error> {
    let lock = deps.runtime.stream_lock(session_id);
    let _guard = lock.lock().await;

    let removed = deps.runtime.streams.remove(session_id);
    let Some((_, st)) = removed else {
        deps.runtime.stream_locks.remove(session_id);
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
    deps.runtime.stream_locks.remove(session_id);
    Ok(())
}

async fn start_stream(deps: &Deps, st: &StreamState) -> Result<String, Error> {
    let mut params = json!({ "channel": st.channel, "thread_ts": st.thread_ts });
    // Streaming into a channel (not a DM) requires both recipient fields.
    if !st.is_dm {
        if let Some(uid) = &st.recipient_user_id {
            params["recipient_user_id"] = json!(uid);
        }
        if let Some(team) = &st.recipient_team_id {
            params["recipient_team_id"] = json!(team);
        }
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

/// How `cur` relates to the already-sent `prev`.
enum Delta {
    /// `cur` extends `prev`; carries the new suffix.
    Append(String),
    /// `cur` is not an extension of `prev` (a rewrite).
    Reset,
}

fn delta(prev: &str, cur: &str) -> Delta {
    match cur.strip_prefix(prev) {
        Some(suffix) => Delta::Append(suffix.to_string()),
        None => Delta::Reset,
    }
}

/// True for a Slack DM channel id (`D…`); DMs take no streaming recipients.
fn channel_is_dm(channel: &str) -> bool {
    channel.starts_with('D')
}

/// Whether a streamed revision should be skipped as stale/out-of-order. Once a
/// stream message exists, only strictly-newer revisions apply; revision 0 (the
/// initial `message-added`) is never skipped.
fn should_skip_revision(stream_started: bool, revision: u64, last_revision: u64) -> bool {
    stream_started && revision != 0 && revision <= last_revision
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_returns_suffix() {
        assert!(matches!(delta("hel", "hello"), Delta::Append(s) if s == "lo"));
        assert!(matches!(delta("", "hi"), Delta::Append(s) if s == "hi"));
        assert!(matches!(delta("abc", "abc"), Delta::Append(s) if s.is_empty()));
    }

    #[test]
    fn delta_handles_reset() {
        assert!(matches!(delta("abc", "xyz"), Delta::Reset));
        // A shorter rewrite (truncation) is also a reset, not a prefix.
        assert!(matches!(delta("abcdef", "abc"), Delta::Reset));
    }

    #[test]
    fn channel_is_dm_detects_dm_ids() {
        assert!(channel_is_dm("D012ABC"));
        assert!(!channel_is_dm("C012ABC")); // public channel
        assert!(!channel_is_dm("G012ABC")); // private channel / mpim
        assert!(!channel_is_dm(""));
    }

    #[test]
    fn revision_guard_skips_stale_once_started() {
        // Before the stream message exists, nothing is skipped.
        assert!(!should_skip_revision(false, 5, 9));
        // Initial message-added (revision 0) is never skipped.
        assert!(!should_skip_revision(true, 0, 7));
        // Newer revision applies.
        assert!(!should_skip_revision(true, 8, 7));
        // Equal or older revision is stale -> skipped.
        assert!(should_skip_revision(true, 7, 7));
        assert!(should_skip_revision(true, 3, 7));
    }
}
