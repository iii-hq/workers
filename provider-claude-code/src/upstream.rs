//! POST /v1/messages (stream:true) → SSE → mpsc<AssistantMessageEvent>.
//! The receiver dropping aborts the upstream: every send error returns,
//! which drops the reqwest response mid-body and closes the connection.
use crate::errors::classify;
use crate::sse::{
    build_partial, handle_sse_event, synthetic_error_event, synthetic_error_event_from_state,
    PartialState,
};
use futures::StreamExt;
use llm_router::provider_scaffold::sse_transport::{
    append_utf8_chunk, drain_sse_blocks, error_chain,
};
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use serde_json::Value;
use tokio::sync::mpsc;

pub struct UpstreamArgs {
    pub api_url: String,
    pub model: String,
    pub body: Value,
    pub headers: Vec<(&'static str, String)>,
    /// Report-and-continue notices for the final message (spec § stream contract).
    pub warnings: Vec<String>,
}

pub fn spawn_upstream(
    client: reqwest::Client,
    args: UpstreamArgs,
) -> mpsc::Receiver<AssistantMessageEvent> {
    let (tx, rx) = mpsc::channel(64);
    tokio::spawn(async move {
        // Race the call against receiver-side closure: send errors alone only
        // observe a dropped receiver at the next send, so a silent upstream
        // (parked in a chunk read, nothing to send) would otherwise keep the
        // HTTP stream — and billed generation — alive until the next frame or
        // the read timeout. This makes the module contract above immediate.
        let closed = tx.clone();
        tokio::select! {
            _ = run_upstream(client, args, tx) => {}
            _ = closed.closed() => {}
        }
    });
    rx
}

/// Drain complete `\n\n`-delimited SSE blocks from `text`, decoding each
/// through this provider's SSE state machine. Returns true when a terminal
/// event was forwarded.
async fn drain_blocks(
    text: &mut String,
    state: &mut PartialState,
    model: &str,
    tx: &mpsc::Sender<AssistantMessageEvent>,
) -> bool {
    drain_sse_blocks(text, tx, &mut |block: &str| {
        handle_sse_event(block, state, model)
    })
    .await
}

async fn run_upstream(
    client: reqwest::Client,
    args: UpstreamArgs,
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
                    &format!("claude-code fetch failed: {}", error_chain(&e)),
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
            format!("claude-code http {status}")
        } else {
            text
        };
        let _ = tx
            .send(synthetic_error_event(&msg, &args.model, kind))
            .await;
        return;
    }

    let mut state = PartialState::new(args.warnings);
    if tx
        .send(AssistantMessageEvent::Start {
            partial: build_partial(&state, &args.model),
        })
        .await
        .is_err()
    {
        return;
    }

    let mut stream = resp.bytes_stream();
    let mut text = String::new();
    let mut byte_buf = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                let _ = tx
                    .send(synthetic_error_event_from_state(
                        &state,
                        &format!("stream read failed: {e}"),
                        &args.model,
                        ErrorKind::Transient,
                    ))
                    .await;
                return;
            }
        };
        append_utf8_chunk(&mut byte_buf, &mut text, &chunk);
        if drain_blocks(&mut text, &mut state, &args.model, &tx).await {
            return;
        }
    }
    // Flush any trailing bytes as UTF-8 (may be an incomplete SSE block).
    if !byte_buf.is_empty() {
        text.push_str(&String::from_utf8_lossy(&byte_buf));
        byte_buf.clear();
    }
    if !text.trim().is_empty() {
        let remainder = std::mem::take(&mut text);
        let _ = drain_blocks(&mut (remainder + "\n\n"), &mut state, &args.model, &tx).await;
    }

    if state.saw_message_stop {
        let _ = tx
            .send(AssistantMessageEvent::Done {
                message: build_partial(&state, &args.model),
            })
            .await;
    } else {
        let _ = tx
            .send(synthetic_error_event_from_state(
                &state,
                "claude-code stream ended before message_stop",
                &args.model,
                ErrorKind::Transient,
            ))
            .await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One-shot HTTP stub: accepts a single connection, consumes the request
    /// head, writes `response` verbatim, closes (read-until-close framing).
    async fn stub(response: &'static str) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 65536];
                let _ = sock.read(&mut buf).await;
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}/v1/messages")
    }

    fn args(api_url: String) -> UpstreamArgs {
        UpstreamArgs {
            api_url,
            model: "claude-test".into(),
            body: serde_json::json!({ "stream": true }),
            headers: vec![("x-api-key", "sk-test".into())],
            warnings: vec![],
        }
    }

    async fn drain(mut rx: mpsc::Receiver<AssistantMessageEvent>) -> Vec<AssistantMessageEvent> {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    }

    const HAPPY: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":12}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

    const TRUNCATED: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"partial\"}}\n\n";

    #[tokio::test(flavor = "multi_thread")]
    async fn builder_error_surfaces_its_source_not_just_builder_error() {
        // A header value with a newline is an invalid HeaderValue → reqwest
        // raises a builder error before connecting. The frame must name the
        // cause, not stop at the opaque "builder error".
        let mut a = args("http://127.0.0.1:1/v1/messages".into());
        a.headers = vec![("x-api-key", "sk-bad\ninjected".into())];
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                let msg = error.error_message.as_deref().unwrap_or_default();
                assert!(msg.starts_with("claude-code fetch failed: "), "got {msg:?}");
                assert_ne!(
                    msg, "claude-code fetch failed: builder error",
                    "source dropped"
                );
                assert!(msg.matches(':').count() >= 2, "no source segment: {msg:?}");
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[test]
    fn utf8_split_across_chunks_is_preserved() {
        let emoji = "Hello 🌍";
        let bytes = emoji.as_bytes();
        let split = "Hello ".len() + 1; // split inside the 4-byte emoji
        let mut byte_buf = Vec::new();
        let mut text = String::new();
        append_utf8_chunk(&mut byte_buf, &mut text, &bytes[..split]);
        assert_eq!(text, "Hello ");
        append_utf8_chunk(&mut byte_buf, &mut text, &bytes[split..]);
        assert_eq!(text, emoji);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn happy_stream_yields_start_through_done() {
        let url = stub(HAPPY).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert_eq!(message.usage.as_ref().unwrap().input, Some(12));
                assert_eq!(message.usage.as_ref().unwrap().output, Some(2));
            }
            other => panic!("want done, got {other:?}"),
        }
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn truncated_stream_yields_error_not_done() {
        let url = stub(TRUNCATED).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        match events.last() {
            Some(AssistantMessageEvent::Error { error }) => {
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
                assert!(error
                    .error_message
                    .as_deref()
                    .is_some_and(|m| m.contains("message_stop")));
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_401_yields_auth_expired_error_frame() {
        let url = stub(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"invalid x-api-key\"}}",
        )
        .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::AuthExpired));
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn connect_failure_yields_transient_error_frame() {
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/v1/messages", l.local_addr().unwrap())
        };
        let events = drain(spawn_upstream(reqwest::Client::new(), args(dead))).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn warnings_arrive_on_the_final_message() {
        let url = stub(HAPPY).await;
        let mut a = args(url);
        a.warnings = vec!["response_format ignored".into()];
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert_eq!(
                    message.warnings.as_deref(),
                    Some(&["response_format ignored".to_string()][..])
                );
            }
            other => panic!("want done, got {other:?}"),
        }
    }
}
