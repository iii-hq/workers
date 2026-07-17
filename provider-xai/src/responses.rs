//! xAI Agent Tools path: the `/v1/responses` API (server-side tools like
//! `x_search` / `web_search`), distinct from the OpenAI-compatible
//! `/v1/chat/completions` path in `request.rs` + `sse.rs`.
//!
//! xAI deprecated the old `search_parameters` "Live Search"; real-time X/web
//! now comes from server-side tools on `/v1/responses`. This module builds the
//! request body and translates the Responses streaming events into the shared
//! `AssistantMessageEvent` sequence. Event shapes captured live from
//! api.x.ai (2026-07): `response.reasoning_summary_text.delta`,
//! `response.output_text.delta`, `response.output_text.annotation.added`
//! (`url_citation`), `response.custom_tool_call_input.*` (the searches), and
//! `response.completed` (final, with usage).

use crate::errors::classify;
use crate::sse::synthetic_error_event;
use crate::wire::messages::to_wire_messages;
use crate::wire::names::decode_tool_name;
use crate::{now_ms, PROVIDER_ID};
use futures::StreamExt;
use llm_router::types::content::ContentBlock;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind, StopReason, Usage};
use llm_router::types::messages::{AgentMessage, AssistantMessage, AssistantRoleTag};
use serde_json::{json, Value};
use tokio::sync::mpsc;

/// Derive the `/v1/responses` endpoint from the configured completions URL
/// (`…/v1/chat/completions` → `…/v1/responses`). When the configured URL does
/// not have the expected `/chat/completions` suffix (a custom origin), keep
/// that origin and swap the trailing path segment to `responses` rather than
/// silently jumping to the public xAI endpoint.
pub fn responses_url(api_url: &str) -> String {
    if let Some(base) = api_url.strip_suffix("/chat/completions") {
        return format!("{base}/responses");
    }
    match api_url.rsplit_once('/') {
        Some((base, _last)) if !base.is_empty() => format!("{base}/responses"),
        _ => "https://api.x.ai/v1/responses".to_string(),
    }
}

/// Build the `/v1/responses` request body. `tools` are xAI server-side tool
/// type strings (`x_search`, `web_search`, `code_interpreter`, …). The
/// `input` array reuses the shared chat-wire message shape (`{role, content}`),
/// which the Responses API accepts verbatim.
pub fn build_responses_body(
    model: &str,
    system_prompt: &str,
    messages: &[AgentMessage],
    tools: &[String],
    max_tokens: u64,
) -> Value {
    let input = to_wire_messages(messages, system_prompt);
    let tool_specs: Vec<Value> = tools.iter().map(|t| json!({ "type": t })).collect();
    json!({
        "model": model,
        "input": input,
        "tools": tool_specs,
        "max_output_tokens": max_tokens,
        "stream": true,
    })
}

/// Accumulated Responses-stream state; folded into the terminal message.
pub struct ResponsesState {
    thinking: String,
    text: String,
    /// `url_citation` targets gathered from annotations, in arrival order.
    citations: Vec<String>,
    /// Server-side tool calls seen (e.g. `x_search`), for a report line.
    tool_calls: Vec<String>,
    open_thinking: bool,
    open_text: bool,
    usage: Usage,
    usage_seen: bool,
    stop_reason: StopReason,
    warnings: Vec<String>,
}

impl ResponsesState {
    pub fn new(warnings: Vec<String>) -> Self {
        ResponsesState {
            thinking: String::new(),
            text: String::new(),
            citations: Vec::new(),
            tool_calls: Vec::new(),
            open_thinking: false,
            open_text: false,
            usage: Usage::default(),
            usage_seen: false,
            stop_reason: StopReason::End,
            warnings,
        }
    }
    pub fn stop_reason(&self) -> StopReason {
        self.stop_reason
    }
}

fn content(state: &ResponsesState) -> Vec<ContentBlock> {
    let mut out = Vec::new();
    if !state.thinking.is_empty() {
        out.push(ContentBlock::Thinking {
            text: state.thinking.clone(),
            signature: None,
        });
    }
    if !state.text.is_empty() {
        out.push(ContentBlock::Text {
            text: state.text.clone(),
        });
    }
    out
}

fn partial(state: &ResponsesState, model: &str) -> AssistantMessage {
    // Citations ride the warnings channel (report-and-continue): the shared
    // AssistantMessage has no dedicated citation field, and warnings already
    // surface non-fatal side-band info to the console.
    let mut warnings = state.warnings.clone();
    if !state.tool_calls.is_empty() {
        warnings.push(format!("tools used: {}", state.tool_calls.join(", ")));
    }
    if !state.citations.is_empty() {
        warnings.push(format!("citations: {}", state.citations.join(", ")));
    }
    AssistantMessage {
        role: AssistantRoleTag::Assistant,
        content: content(state),
        stop_reason: state.stop_reason,
        native_stop_reason: None,
        error_message: None,
        error_kind: None,
        warnings: (!warnings.is_empty()).then_some(warnings),
        usage: state.usage_seen.then(|| state.usage.clone()),
        model: model.to_string(),
        provider: PROVIDER_ID.to_string(),
        timestamp: now_ms(),
    }
}

pub fn build_final(state: &ResponsesState, model: &str) -> AssistantMessage {
    partial(state, model)
}

fn close_thinking(state: &mut ResponsesState, model: &str, out: &mut Vec<AssistantMessageEvent>) {
    if state.open_thinking {
        state.open_thinking = false;
        out.push(AssistantMessageEvent::ThinkingEnd {
            partial: partial(state, model),
        });
    }
}

/// Process one parsed Responses SSE event into 0+ AssistantMessageEvents.
/// `event_type` is the SSE `event:` name; `data` its JSON payload.
pub fn handle_event(
    event_type: &str,
    data: &Value,
    state: &mut ResponsesState,
    model: &str,
) -> Vec<AssistantMessageEvent> {
    let mut out = Vec::new();
    match event_type {
        "response.reasoning_summary_text.delta" => {
            let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
            if !delta.is_empty() {
                if !state.open_thinking {
                    state.open_thinking = true;
                    out.push(AssistantMessageEvent::ThinkingStart {
                        partial: partial(state, model),
                    });
                }
                state.thinking.push_str(delta);
                out.push(AssistantMessageEvent::ThinkingDelta {
                    partial: None,
                    delta: delta.to_string(),
                });
            }
        }
        "response.custom_tool_call_input.done" => {
            // Record which server-side tool ran (name may carry the search).
            if let Some(name) = data.get("name").and_then(Value::as_str) {
                state.tool_calls.push(decode_tool_name(name));
            }
        }
        "response.output_text.delta" => {
            let delta = data.get("delta").and_then(Value::as_str).unwrap_or("");
            if !delta.is_empty() {
                close_thinking(state, model, &mut out);
                if !state.open_text {
                    state.open_text = true;
                    out.push(AssistantMessageEvent::TextStart {
                        partial: partial(state, model),
                    });
                }
                state.text.push_str(delta);
                out.push(AssistantMessageEvent::TextDelta {
                    partial: None,
                    delta: delta.to_string(),
                });
            }
        }
        "response.output_text.annotation.added" => {
            if let Some(url) = data.pointer("/annotation/url").and_then(Value::as_str) {
                state.citations.push(url.to_string());
            }
        }
        "response.output_text.done" if state.open_text => {
            state.open_text = false;
            out.push(AssistantMessageEvent::TextEnd {
                partial: partial(state, model),
            });
        }
        "response.completed" => {
            close_thinking(state, model, &mut out);
            if let Some(u) = data.pointer("/response/usage").filter(|u| u.is_object()) {
                merge_usage(u, &mut state.usage);
                state.usage_seen = true;
                out.push(AssistantMessageEvent::Usage {
                    usage: state.usage.clone(),
                });
            }
        }
        _ => {}
    }
    out
}

/// Map the Responses usage object onto the shared `Usage`.
fn merge_usage(raw: &Value, into: &mut Usage) {
    let num = |k: &str| raw.get(k).and_then(Value::as_u64);
    if let Some(v) = num("input_tokens") {
        into.input = Some(v);
    }
    if let Some(v) = num("output_tokens") {
        into.output = Some(v);
    }
    if let Some(v) = raw
        .pointer("/input_tokens_details/cached_tokens")
        .and_then(Value::as_u64)
    {
        into.cache_read = Some(v);
    }
    if let Some(v) = raw
        .pointer("/output_tokens_details/reasoning_tokens")
        .and_then(Value::as_u64)
    {
        into.reasoning = Some(v);
    }
}

/// Byte offset of the first `\n\n` SSE frame terminator, if present.
fn find_frame_boundary(buf: &[u8]) -> Option<usize> {
    buf.windows(2).position(|w| w == b"\n\n")
}

/// Parse an SSE block into `(event_name, data_json_str)`. Responses events are
/// `event: <name>\ndata: <json>`.
fn event_and_data(block: &str) -> Option<(&str, &str)> {
    let mut ev = None;
    let mut data = None;
    for line in block.lines() {
        if let Some(v) = line.strip_prefix("event: ") {
            ev = Some(v.trim());
        } else if let Some(v) = line.strip_prefix("data: ") {
            data = Some(v);
        }
    }
    match (ev, data) {
        (Some(e), Some(d)) => Some((e, d)),
        _ => None,
    }
}

pub struct ResponsesUpstreamArgs {
    pub api_url: String,
    pub model: String,
    pub body: Value,
    pub headers: Vec<(&'static str, String)>,
    pub warnings: Vec<String>,
}

/// POST `/v1/responses` (stream) → SSE → mpsc<AssistantMessageEvent>. Mirrors
/// `upstream::spawn_upstream` but parses the Responses `event:`/`data:` frames.
pub fn spawn_responses_upstream(
    client: reqwest::Client,
    args: ResponsesUpstreamArgs,
) -> mpsc::Receiver<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        run_responses_upstream(client, args, tx).await;
    });
    rx
}

async fn run_responses_upstream(
    client: reqwest::Client,
    args: ResponsesUpstreamArgs,
    tx: mpsc::Sender<AssistantMessageEvent>,
) {
    let mut req = client.post(&args.api_url);
    for (name, value) in &args.headers {
        req = req.header(*name, value);
    }
    let resp = match req.json(&args.body).send().await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx
                .send(synthetic_error_event(
                    &format!("xai responses fetch failed: {e}"),
                    &args.model,
                    ErrorKind::Transient,
                ))
                .await;
            return;
        }
    };
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        let kind = classify(Some(status.as_u16()), &text);
        let msg = if text.is_empty() {
            format!("xai responses http {status}")
        } else {
            text
        };
        let _ = tx
            .send(synthetic_error_event(&msg, &args.model, kind))
            .await;
        return;
    }

    let mut state = ResponsesState::new(args.warnings);
    if tx
        .send(AssistantMessageEvent::Start {
            partial: partial(&state, &args.model),
        })
        .await
        .is_err()
    {
        return;
    }

    let mut stream = resp.bytes_stream();
    // Accumulate raw bytes and only decode a complete SSE frame (up to the
    // `\n\n` boundary) as UTF-8, so a multi-byte character split across two
    // network chunks is never corrupted — X content is unicode-heavy.
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(synthetic_error_event(
                        &format!("stream read failed: {e}"),
                        &args.model,
                        ErrorKind::Transient,
                    ))
                    .await;
                return;
            }
        };
        buf.extend_from_slice(&chunk);
        while let Some(idx) = find_frame_boundary(&buf) {
            let frame: Vec<u8> = buf.drain(..idx + 2).collect();
            let block = String::from_utf8_lossy(&frame);
            let Some((event_type, data)) = event_and_data(&block) else {
                continue;
            };
            let Ok(parsed) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            for ev in handle_event(event_type, &parsed, &mut state, &args.model) {
                if tx.send(ev).await.is_err() {
                    return;
                }
            }
            if event_type == "response.completed" {
                let _ = tx
                    .send(AssistantMessageEvent::Stop {
                        stop_reason: state.stop_reason(),
                        error_message: None,
                        error_kind: None,
                    })
                    .await;
                let _ = tx
                    .send(AssistantMessageEvent::Done {
                        message: build_final(&state, &args.model),
                    })
                    .await;
                return;
            }
        }
    }
    // Stream ended without `response.completed`: the turn was truncated, not
    // completed. Surface a transient error so callers can distinguish this from
    // a real terminal completion (and retry).
    let _ = tx
        .send(synthetic_error_event(
            "xai responses stream ended before completion",
            &args.model,
            ErrorKind::Transient,
        ))
        .await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn user(text: &str) -> AgentMessage {
        AgentMessage::User(llm_router::types::messages::UserMessage {
            role: llm_router::types::messages::UserRoleTag::User,
            content: vec![ContentBlock::Text { text: text.into() }],
            timestamp: 0,
        })
    }

    #[test]
    fn responses_url_derives_from_completions() {
        assert_eq!(
            responses_url("https://api.x.ai/v1/chat/completions"),
            "https://api.x.ai/v1/responses"
        );
        assert_eq!(
            responses_url("http://127.0.0.1:9/v1/chat/completions"),
            "http://127.0.0.1:9/v1/responses"
        );
        // custom origin without the chat/completions suffix: keep the origin,
        // swap the trailing segment — never silently jump to public xAI.
        assert_eq!(
            responses_url("https://proxy.internal/v1/foo"),
            "https://proxy.internal/v1/responses"
        );
    }

    #[test]
    fn body_carries_input_and_tools() {
        let b = build_responses_body(
            "grok-4.3",
            "be brief",
            &[user("hi")],
            &["x_search".into(), "web_search".into()],
            8192,
        );
        assert_eq!(b["model"], "grok-4.3");
        assert_eq!(b["stream"], true);
        assert_eq!(b["input"][0]["role"], "system");
        assert_eq!(b["input"][1]["role"], "user");
        assert_eq!(b["input"][1]["content"], "hi");
        assert_eq!(b["tools"][0]["type"], "x_search");
        assert_eq!(b["tools"][1]["type"], "web_search");
    }

    fn run(events: &[(&str, Value)]) -> (ResponsesState, Vec<AssistantMessageEvent>) {
        let mut st = ResponsesState::new(vec![]);
        let mut evs = Vec::new();
        for (t, d) in events {
            evs.extend(handle_event(t, d, &mut st, "grok-4.3"));
        }
        (st, evs)
    }

    fn tags(evs: &[AssistantMessageEvent]) -> Vec<&'static str> {
        evs.iter()
            .map(|e| match e {
                AssistantMessageEvent::ThinkingStart { .. } => "thinking_start",
                AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
                AssistantMessageEvent::ThinkingEnd { .. } => "thinking_end",
                AssistantMessageEvent::TextStart { .. } => "text_start",
                AssistantMessageEvent::TextDelta { .. } => "text_delta",
                AssistantMessageEvent::TextEnd { .. } => "text_end",
                AssistantMessageEvent::Usage { .. } => "usage",
                _ => "other",
            })
            .collect()
    }

    /// Contract pin (llm-router types::events): delta frames are slim —
    /// no cumulative partial per chunk — while block-boundary frames carry
    /// the authoritative snapshot. Readers reconstruct via
    /// llm_router::chat::accumulate.
    #[test]
    fn deltas_are_slim_and_boundary_snapshots_are_cumulative() {
        let (_, evs) = run(&[
            ("response.output_text.delta", json!({"delta":"He"})),
            ("response.output_text.delta", json!({"delta":"llo"})),
            ("response.output_text.done", json!({})),
        ]);
        for ev in &evs {
            match ev {
                AssistantMessageEvent::TextDelta { partial, .. }
                | AssistantMessageEvent::ThinkingDelta { partial, .. } => {
                    assert!(partial.is_none(), "delta frames must not carry partial");
                }
                _ => {}
            }
        }
        let Some(AssistantMessageEvent::TextEnd { partial }) = evs
            .iter()
            .find(|e| matches!(e, AssistantMessageEvent::TextEnd { .. }))
        else {
            panic!("want a text_end frame");
        };
        assert!(
            matches!(
                &partial.content[0],
                ContentBlock::Text { text } if text == "Hello"
            ),
            "the End snapshot must carry the cumulative block text"
        );
    }

    #[test]
    fn reasoning_then_text_with_citation_and_usage() {
        let (state, evs) = run(&[
            (
                "response.reasoning_summary_text.delta",
                json!({"delta":"think"}),
            ),
            (
                "response.custom_tool_call_input.done",
                json!({"name":"x_search"}),
            ),
            ("response.output_text.delta", json!({"delta":"World Cup"})),
            (
                "response.output_text.annotation.added",
                json!({"annotation":{"type":"url_citation","url":"https://x.com/i/status/1"}}),
            ),
            ("response.output_text.done", json!({})),
            (
                "response.completed",
                json!({"response":{"usage":{"input_tokens":10,"output_tokens":4,
                    "output_tokens_details":{"reasoning_tokens":2}}}}),
            ),
        ]);
        assert_eq!(
            tags(&evs),
            vec![
                "thinking_start",
                "thinking_delta",
                "thinking_end",
                "text_start",
                "text_delta",
                "text_end",
                "usage",
            ]
        );
        let f = build_final(&state, "grok-4.3");
        assert_eq!(
            f.content,
            vec![
                ContentBlock::Thinking {
                    text: "think".into(),
                    signature: None
                },
                ContentBlock::Text {
                    text: "World Cup".into()
                },
            ]
        );
        // citation surfaced on warnings
        assert!(f
            .warnings
            .unwrap()
            .iter()
            .any(|w| w.contains("https://x.com/i/status/1")));
        let u = f.usage.unwrap();
        assert_eq!(u.input, Some(10));
        assert_eq!(u.output, Some(4));
        assert_eq!(u.reasoning, Some(2));
        assert_eq!(state.tool_calls, vec!["x_search"]);
    }

    #[test]
    fn unknown_events_are_ignored() {
        let (_s, evs) = run(&[
            ("response.created", json!({})),
            ("response.in_progress", json!({})),
            (
                "response.output_item.added",
                json!({"item":{"type":"reasoning"}}),
            ),
        ]);
        assert!(evs.is_empty());
    }
}
