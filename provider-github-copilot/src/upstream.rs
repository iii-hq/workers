//! POST /v1/chat/completions (stream:true) → SSE → mpsc<AssistantMessageEvent>.
//! The receiver dropping aborts the upstream: every send error returns,
//! which drops the reqwest response mid-body and closes the connection.
use crate::errors::classify;
use crate::sse::{build_final, build_partial, handle_chunk, synthetic_error_event, PartialState};
use futures::StreamExt;
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use serde_json::Value;
use tokio::sync::mpsc;

/// Cap on an upstream error body quoted back to the caller. A gateway can
/// answer with a whole HTML page; the error frame should stay readable.
const MAX_ERROR_BODY: usize = 2_000;

/// Cap on the un-framed SSE buffer. An upstream that never sends a blank
/// line would otherwise grow it for the whole response while every chunk
/// rescans it. Exceeding this is a broken stream, not a slow one.
const MAX_SSE_BUFFER: usize = 1_048_576;

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
        // the read timeout.
        let closed = tx.clone();
        tokio::select! {
            _ = run_upstream(client, args, tx) => {}
            _ = closed.closed() => {}
        }
    });
    rx
}

/// Last `data:` payload in an SSE block, if any. Both `data: x` and `data:x`
/// are valid SSE framings; trailing `\r` from CRLF line endings is stripped.
fn data_line(block: &str) -> Option<&str> {
    block
        .lines()
        .filter_map(|l| {
            let rest = l.trim_end_matches('\r').strip_prefix("data:")?;
            Some(rest.strip_prefix(' ').unwrap_or(rest))
        })
        .next_back()
}

/// Earliest complete-SSE-block boundary in the byte buffer: `(end_index,
/// separator_len)` for `\n\n` or `\r\n\r\n`, whichever comes first. Byte-level
/// so a multibyte UTF-8 character split across network chunks is never
/// decoded early (several vendors stream CJK-heavy content).
fn find_block_end(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, None) => a,
        (None, b) => b,
    }
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
                    &format!("copilot fetch failed: {e}"),
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
            format!("copilot http {status}")
        } else if text.len() > MAX_ERROR_BODY {
            format!("{}…", &text[..text.floor_char_boundary(MAX_ERROR_BODY)])
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
        return; // receiver gone before the first frame
    }

    let mut stream = resp.bytes_stream();
    // Byte buffer, decoded only per complete block: a multibyte character or
    // block separator split across chunks stays buffered until whole.
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
        if buf.len() > MAX_SSE_BUFFER {
            let _ = tx
                .send(synthetic_error_event(
                    "upstream sent no SSE frame boundary within 1 MiB",
                    &args.model,
                    ErrorKind::Transient,
                ))
                .await;
            return;
        }
        while let Some((end, sep_len)) = find_block_end(&buf) {
            let raw: Vec<u8> = buf.drain(..end + sep_len).collect();
            let block = String::from_utf8_lossy(&raw);
            let Some(data) = data_line(&block) else {
                continue;
            };
            if data == "[DONE]" {
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
            let Ok(parsed) = serde_json::from_str::<Value>(data) else {
                continue;
            };
            for ev in handle_chunk(&parsed, &mut state, &args.model) {
                let terminal = ev.is_terminal();
                if tx.send(ev).await.is_err() {
                    return; // receiver dropped → abort upstream
                }
                if terminal {
                    return; // exactly one terminal event
                }
            }
        }
    }
    // Stream ended without [DONE] (connection close framing): still terminal,
    // and still preceded by Stop so both endings look the same downstream.
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
        format!("http://{addr}/v1/chat/completions")
    }

    fn args(api_url: String) -> UpstreamArgs {
        UpstreamArgs {
            api_url,
            model: "gpt-test".into(),
            body: serde_json::json!({ "stream": true }),
            headers: vec![("authorization", "Bearer sk-test".into())],
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

    const HAPPY: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"}}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: {\"choices\":[],\"usage\":{\"prompt_tokens\":12,\"completion_tokens\":2,\"prompt_tokens_details\":{\"cached_tokens\":4}}}\n\ndata: [DONE]\n\n";

    #[tokio::test(flavor = "multi_thread")]
    async fn happy_stream_yields_start_through_stop_and_done() {
        let url = stub(HAPPY).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert!(matches!(
            events.first(),
            Some(AssistantMessageEvent::Start { .. })
        ));
        assert!(
            matches!(
                events[events.len() - 2],
                AssistantMessageEvent::Stop {
                    stop_reason: llm_router::types::events::StopReason::End,
                    ..
                }
            ),
            "stop precedes done"
        );
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert_eq!(message.usage.as_ref().unwrap().input, Some(12));
                assert_eq!(message.usage.as_ref().unwrap().output, Some(2));
                assert_eq!(message.usage.as_ref().unwrap().cache_read, Some(4));
                assert_eq!(message.native_stop_reason.as_deref(), Some("stop"));
            }
            other => panic!("want done, got {other:?}"),
        }
        // exactly one terminal
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_401_yields_auth_expired_error_frame() {
        let url = stub(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"Incorrect API key provided.\",\"type\":\"invalid_request_error\",\"code\":\"invalid_api_key\"}}",
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
        // bind-then-drop guarantees a dead port
        let dead = {
            let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            format!("http://{}/v1/chat/completions", l.local_addr().unwrap())
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
    async fn oversized_error_body_is_truncated_in_the_frame() {
        let big = "x".repeat(MAX_ERROR_BODY * 2);
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 500 Internal Server Error\r\ncontent-type: text/html\r\nconnection: close\r\n\r\n{big}"
            )
            .into_boxed_str(),
        );
        let url = stub(response).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                let msg = error.error_message.as_deref().unwrap_or_default();
                assert!(msg.len() <= MAX_ERROR_BODY + 8, "got {} bytes", msg.len());
                assert!(msg.ends_with('…'));
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_end_without_done_sentinel_still_emits_done() {
        let url = stub(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"}}]}\n\n",
        )
        .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert!(
                    matches!(&message.content[0], llm_router::types::content::ContentBlock::Text { text } if text == "Hi")
                );
            }
            other => panic!("want done, got {other:?}"),
        }
    }

    #[test]
    fn block_end_handles_lf_and_crlf_separators() {
        assert_eq!(find_block_end(b"data: x\n\nrest"), Some((7, 2)));
        assert_eq!(find_block_end(b"data: x\r\n\r\nrest"), Some((7, 4)));
        // earliest boundary wins when both framings appear
        assert_eq!(find_block_end(b"a\n\nb\r\n\r\nc"), Some((1, 2)));
        assert_eq!(find_block_end(b"a\r\n\r\nb\n\nc"), Some((1, 4)));
        // incomplete block: no boundary yet
        assert_eq!(find_block_end(b"data: partial\n"), None);
        assert_eq!(find_block_end(b"data: partial\r\n"), None);
    }

    #[test]
    fn data_line_accepts_bare_and_spaced_prefixes_and_crlf() {
        assert_eq!(data_line("data: hello\n"), Some("hello"));
        assert_eq!(data_line("data:hello\n"), Some("hello"));
        assert_eq!(data_line("data: hello\r\n"), Some("hello"));
        assert_eq!(data_line(": comment\nretry: 1000\n"), None);
        // last data line of the block wins
        assert_eq!(data_line("data: a\ndata: b\n"), Some("b"));
    }

    const CRLF_CJK: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\ndata:{\"choices\":[{\"index\":0,\"delta\":{\"content\":\"你好，世界\"}}]}\r\n\r\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\r\n\r\ndata: [DONE]\r\n\r\n";

    #[tokio::test(flavor = "multi_thread")]
    async fn crlf_framing_and_multibyte_content_arrive_intact() {
        let url = stub(CRLF_CJK).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert!(
                    matches!(&message.content[0], llm_router::types::content::ContentBlock::Text { text } if text == "你好，世界")
                );
                assert_eq!(message.native_stop_reason.as_deref(), Some("stop"));
            }
            other => panic!("want done, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn warnings_arrive_on_the_final_message() {
        let url = stub(HAPPY).await;
        let mut a = args(url);
        a.warnings = vec!["thinking_level ignored".into()];
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert_eq!(
                    message.warnings.as_deref(),
                    Some(&["thinking_level ignored".to_string()][..])
                );
            }
            other => panic!("want done, got {other:?}"),
        }
    }
}
