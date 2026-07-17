//! POST /v1/chat/completions (stream:true) → SSE → mpsc<AssistantMessageEvent>.
//! The receiver dropping aborts the upstream: every send error returns,
//! which drops the reqwest response mid-body and closes the connection.
use crate::errors::classify;
use crate::sse::{build_final, build_partial, handle_chunk, synthetic_error_event, PartialState};
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
        run_upstream(client, args, tx).await;
    });
    rx
}

/// Last `data: ` payload in an SSE block, if any.
fn data_line(block: &str) -> Option<&str> {
    block
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .next_back()
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
                    &format!("xai fetch failed: {}", error_chain(&e)),
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
            format!("xai http {status}")
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
    let mut buf = String::new();
    // Cross-chunk UTF-8 buffering: network chunks split multibyte
    // codepoints, and a per-chunk lossy conversion corrupts them to U+FFFD.
    let mut byte_buf = Vec::new();
    // Block decoder: [DONE] closes the stream (Stop + Done, Done terminal);
    // anything else parses and runs the chunk state machine.
    let decode =
        |data_block: &str, state: &mut PartialState, model: &str| -> Vec<AssistantMessageEvent> {
            let Some(data) = data_line(data_block) else {
                return vec![];
            };
            if data == "[DONE]" {
                return vec![
                    AssistantMessageEvent::Stop {
                        stop_reason: state.stop_reason(),
                        error_message: None,
                        error_kind: None,
                    },
                    AssistantMessageEvent::Done {
                        message: build_final(state, model),
                    },
                ];
            }
            let Ok(parsed) = serde_json::from_str::<Value>(data) else {
                return vec![];
            };
            handle_chunk(&parsed, state, model)
        };
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
        append_utf8_chunk(&mut byte_buf, &mut buf, &chunk);
        if drain_sse_blocks(&mut buf, &tx, &mut |block: &str| {
            decode(block, &mut state, &args.model)
        })
        .await
        {
            return; // terminal forwarded, or receiver dropped → abort upstream
        }
    }
    // Stream ended without [DONE] (connection close framing): still terminal.
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
            model: "grok-test".into(),
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
    async fn builder_error_surfaces_its_source_not_just_builder_error() {
        // A header value with a newline is an invalid HeaderValue → reqwest
        // raises a builder error before connecting. The frame must name the
        // cause, not stop at the opaque "builder error".
        let mut a = args("http://127.0.0.1:1/v1/chat/completions".into());
        a.headers = vec![("authorization", "Bearer sk-bad\ninjected".into())];
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                let msg = error.error_message.as_deref().unwrap_or_default();
                assert!(msg.starts_with("xai fetch failed: "), "got {msg:?}");
                // The source chain was appended past the bare "builder error".
                assert_ne!(msg, "xai fetch failed: builder error", "source dropped");
                assert!(msg.matches(':').count() >= 2, "no source segment: {msg:?}");
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
