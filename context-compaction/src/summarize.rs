//! Compaction execution: load active path, split keep-recent / older-prefix,
//! summarise via provider-router, append a Compaction entry to session-tree.

use anyhow::{anyhow, Context};
use iii_sdk::{TriggerRequest, Value, III};
use serde_json::json;

use crate::config;

const COMPACTION_PROMPT: &str = include_str!("../prompts/compaction.txt");
const SUMMARIZER_TIMEOUT_MS: u64 = 120_000;

/// Test-only re-export so `lib::tests::lease_ttl_exceeds_summarizer_timeout`
/// can pin the LEASE_TTL_SECS > SUMMARIZER_TIMEOUT_MS/1000 invariant
/// without making the constant `pub` in non-test builds.
#[cfg(test)]
pub(crate) const SUMMARIZER_TIMEOUT_MS_FOR_TEST: u64 = SUMMARIZER_TIMEOUT_MS;

/// Top-level entry point. Reads the active session-tree path, decides
/// what to summarise, calls the summariser LLM, then writes the
/// `Compaction` entry. Errors are surfaced to the caller for tracing
/// but never panic.
pub async fn summarize_and_append(iii: &III, session_id: &str) -> anyhow::Result<()> {
    let messages = load_active_messages(iii, session_id).await?;
    let keep = config::keep_recent_turns();
    let (older, _recent) = split_messages(&messages, keep);
    if older.is_empty() {
        // Nothing to summarise — the session is shorter than
        // keep_recent_turns. Don't waste a summariser call.
        return Ok(());
    }
    let tokens_before = estimate_token_count(&older);
    let summary = summarize_with_router(iii, session_id, &older).await?;
    append_compaction(iii, session_id, summary, tokens_before).await?;
    Ok(())
}

/// Walk the session-tree's active path and return the in-transcript
/// `AgentMessage`s (the session worker already filters Compaction entries
/// out, so the model would never have seen them either).
async fn load_active_messages(iii: &III, session_id: &str) -> anyhow::Result<Vec<Value>> {
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "session-tree::messages".into(),
            payload: json!({ "session_id": session_id }),
            action: None,
            timeout_ms: Some(10_000),
        })
        .await
        .map_err(|e| anyhow!("session-tree::messages failed: {e}"))?;
    let messages = resp
        .get("messages")
        .cloned()
        .unwrap_or(resp)
        .as_array()
        .cloned()
        .ok_or_else(|| anyhow!("session-tree::messages returned non-array"))?;
    Ok(messages)
}

/// Split the message list so the trailing `keep_recent` turns stay
/// verbatim and the older prefix becomes the summariser's input. A
/// "turn" here is one assistant message plus any leading user / tool
/// results; we approximate by counting messages from the end, which is
/// close enough for the threshold-driven trigger.
fn split_messages(messages: &[Value], keep_recent: usize) -> (Vec<Value>, Vec<Value>) {
    if messages.len() <= keep_recent {
        return (Vec::new(), messages.to_vec());
    }
    let split_at = messages.len() - keep_recent;
    (
        messages[..split_at].to_vec(),
        messages[split_at..].to_vec(),
    )
}

/// Rough token-count proxy. The summariser doesn't need byte-accurate
/// numbers for the `tokens_before` field — it's metadata for later
/// analysis, not a billing input — so a 4-chars-per-token heuristic is
/// fine.
fn estimate_token_count(messages: &[Value]) -> u64 {
    let total_chars: usize = messages
        .iter()
        .map(|m| serde_json::to_string(m).map(|s| s.len()).unwrap_or(0))
        .sum();
    (total_chars / 4) as u64
}

async fn summarize_with_router(
    iii: &III,
    session_id: &str,
    older: &[Value],
) -> anyhow::Result<String> {
    let payload = build_summarizer_payload(session_id, older);
    let resp = iii
        .trigger(TriggerRequest {
            function_id: "router::stream_assistant".into(),
            payload,
            action: None,
            timeout_ms: Some(SUMMARIZER_TIMEOUT_MS),
        })
        .await
        .map_err(|e| anyhow!("router::stream_assistant failed: {e}"))?;

    extract_summary_text(&resp).context("summariser response had no text content")
}

/// Build the payload sent to `router::stream_assistant` for the
/// summarisation call. The provider/model are configurable so operators
/// can route to a cheap model without touching this code.
pub(crate) fn build_summarizer_payload(session_id: &str, older: &[Value]) -> Value {
    let provider = config::summarizer_provider().unwrap_or_else(|| "anthropic".to_string());
    let model = config::summarizer_model().unwrap_or_else(|| "claude-haiku-4-5".to_string());
    json!({
        "session_id": format!("{session_id}::compaction"),
        "provider": provider,
        "model": model,
        "system_prompt": COMPACTION_PROMPT,
        "messages": [
            {
                "role": "user",
                "content": [
                    { "type": "text", "text": render_user_prompt(older) }
                ],
                "timestamp": chrono::Utc::now().timestamp_millis()
            }
        ],
        "tools": []
    })
}

fn render_user_prompt(older: &[Value]) -> String {
    let mut out = String::new();
    out.push_str("Summarise the conversation below following the system-prompt structure exactly. Keep identifiers verbatim.\n\n");
    out.push_str("<conversation>\n");
    for msg in older {
        if let Some(role) = msg.get("role").and_then(Value::as_str) {
            out.push_str(&format!("\n[{role}]\n"));
        }
        if let Some(content) = msg.get("content").and_then(Value::as_array) {
            for block in content {
                if let Some(text) = block.get("text").and_then(Value::as_str) {
                    out.push_str(text);
                    out.push('\n');
                } else if let Some(name) = block.get("name").and_then(Value::as_str) {
                    let args = block.get("input").cloned().unwrap_or_else(|| json!({}));
                    out.push_str(&format!(
                        "[tool_call] {name} {}\n",
                        serde_json::to_string(&args).unwrap_or_default()
                    ));
                }
            }
        } else if let Some(text) = msg.get("content").and_then(Value::as_str) {
            out.push_str(text);
            out.push('\n');
        }
    }
    out.push_str("</conversation>\n");
    out
}

fn extract_summary_text(resp: &Value) -> Option<String> {
    // router::stream_assistant returns an AssistantMessage shape with
    // `content: [ContentBlock]`. Concatenate text blocks.
    let content = resp.get("content").and_then(Value::as_array)?;
    let mut out = String::new();
    for block in content {
        if let Some(t) = block.get("text").and_then(Value::as_str) {
            if !out.is_empty() {
                out.push('\n');
            }
            out.push_str(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

async fn append_compaction(
    iii: &III,
    session_id: &str,
    summary: String,
    tokens_before: u64,
) -> anyhow::Result<()> {
    iii.trigger(TriggerRequest {
        function_id: "session-tree::compact".into(),
        payload: json!({
            "session_id": session_id,
            "summary": summary,
            "tokens_before": tokens_before,
            "file_ops": { "read_files": [], "modified_files": [] }
        }),
        action: None,
        timeout_ms: Some(10_000),
    })
    .await
    .map_err(|e| anyhow!("session-tree::compact failed: {e}"))?;

    // Stamp the orchestrator-watched watermark so the next turn's
    // `handle_streaming` reload check fires.
    crate::stamp_last_compaction(iii, session_id).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_messages_keeps_all_when_below_threshold() {
        let msgs = vec![json!({"role": "user"}), json!({"role": "assistant"})];
        let (older, recent) = split_messages(&msgs, 3);
        assert!(older.is_empty());
        assert_eq!(recent.len(), 2);
    }

    #[test]
    fn split_messages_splits_at_keep_boundary() {
        let msgs: Vec<Value> = (0..10).map(|i| json!({"i": i})).collect();
        let (older, recent) = split_messages(&msgs, 3);
        assert_eq!(older.len(), 7);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0]["i"], 7);
        assert_eq!(recent[2]["i"], 9);
    }

    #[test]
    fn render_user_prompt_includes_role_and_text() {
        let msgs = vec![
            json!({"role": "user", "content": [{"type": "text", "text": "hello"}]}),
            json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ];
        let out = render_user_prompt(&msgs);
        assert!(out.contains("[user]"));
        assert!(out.contains("[assistant]"));
        assert!(out.contains("hello"));
        assert!(out.contains("hi"));
        assert!(out.contains("<conversation>"));
        assert!(out.contains("</conversation>"));
    }

    #[test]
    fn render_user_prompt_handles_tool_calls() {
        let msgs = vec![json!({
            "role": "assistant",
            "content": [
                {"type": "tool_use", "name": "shell__run", "input": {"command": "ls"}}
            ]
        })];
        let out = render_user_prompt(&msgs);
        assert!(out.contains("[tool_call] shell__run"));
        assert!(out.contains("command"));
    }

    #[test]
    fn extract_summary_text_concatenates_text_blocks() {
        let resp = json!({
            "content": [
                {"type": "text", "text": "# Goal\nbuild a thing"},
                {"type": "text", "text": "# Plan state\ndone"}
            ]
        });
        let s = extract_summary_text(&resp).expect("has text");
        assert!(s.contains("Goal"));
        assert!(s.contains("Plan state"));
    }

    #[test]
    fn extract_summary_text_returns_none_when_empty() {
        let resp = json!({ "content": [] });
        assert!(extract_summary_text(&resp).is_none());
    }

    #[test]
    fn estimate_token_count_scales_with_message_size() {
        let small = vec![json!({"text": "x"})];
        let big = vec![json!({"text": "x".repeat(4000)})];
        assert!(estimate_token_count(&big) > estimate_token_count(&small));
    }

    #[test]
    fn build_summarizer_payload_uses_configured_or_default_model() {
        let payload = build_summarizer_payload("sess-1", &[json!({"role": "user", "content": []})]);
        assert!(payload["session_id"]
            .as_str()
            .unwrap()
            .contains("compaction"));
        assert!(payload["model"].is_string());
        assert!(payload["provider"].is_string());
        assert!(payload["system_prompt"]
            .as_str()
            .unwrap()
            .contains("anchored context summarization"));
    }
}
