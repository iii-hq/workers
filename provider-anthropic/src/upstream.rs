//! POST /v1/messages (stream:true) → SSE → mpsc<AssistantMessageEvent>.
//! The receiver dropping aborts the upstream: every send error returns,
//! which drops the reqwest response mid-body and closes the connection.
use crate::errors::classify;
use crate::sse::{
    build_final, build_partial, handle_sse_event, synthetic_error_event, PartialState,
};
use futures::StreamExt;
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
                    &format!("anthropic fetch failed: {e}"),
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
            format!("anthropic http {status}")
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
        buf.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(idx) = buf.find("\n\n") {
            let block: String = buf.drain(..idx + 2).collect();
            for ev in handle_sse_event(&block, &mut state, &args.model) {
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
        // exactly one terminal
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
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
        // bind-then-drop guarantees a dead port
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
