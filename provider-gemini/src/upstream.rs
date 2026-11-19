//! POST the configured Interactions endpoint (stream:true) → SSE →
//! mpsc<AssistantMessageEvent>.
//! The receiver dropping aborts the upstream: every send error returns,
//! which drops the reqwest response mid-body and closes the connection.
use crate::PROVIDER_ID;
use crate::errors::{classify, error_message};
use crate::sse::{PartialState, build_final, build_partial, decode_block, synthetic_error_event};
use futures::StreamExt;
use llm_router::provider_scaffold::sse_transport::{
    CloseFraming, StreamEnd, TailFlush, append_utf8_chunk, classify_stream_end, drain_sse_blocks,
    error_chain, flush_tail, truncated_stream_error,
};
use llm_router::types::events::{AssistantMessageEvent, ErrorKind};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct UpstreamArgs {
    pub api_url: String,
    pub model: String,
    pub body: Value,
    pub headers: Vec<(&'static str, String)>,
    /// Report-and-continue notices for the final message (spec § stream contract).
    pub warnings: Vec<String>,
    /// Budget for generation to start, independent of transport keepalives.
    pub first_token_timeout: Duration,
    /// Period of the empty delta sent while a thought or text block is open.
    pub heartbeat: Duration,
}

/// Well under the router's default 120 s idle budget.
pub const HEARTBEAT: Duration = Duration::from_secs(30);

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

async fn run_upstream(
    client: reqwest::Client,
    args: UpstreamArgs,
    tx: mpsc::Sender<AssistantMessageEvent>,
) {
    // Transport reads (including keepalives) are not proof that inference
    // started. One deadline covers response headers and all pre-content SSE.
    let first_token_deadline = tokio::time::sleep(args.first_token_timeout);
    tokio::pin!(first_token_deadline);
    let mut req = client.post(&args.api_url);
    for (name, value) in &args.headers {
        req = req.header(*name, value);
    }
    let response = tokio::select! {
        _ = &mut first_token_deadline => {
            let _ = tx.send(generation_timeout(&args.model, args.first_token_timeout)).await;
            return;
        }
        response = req.json(&args.body).send() => response,
    };
    let resp = match response {
        Ok(r) => r,
        Err(e) => {
            let _ = tx
                .send(synthetic_error_event(
                    &format!("gemini fetch failed: {}", error_chain(&e)),
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
        let status = Some(status.as_u16());
        let kind = classify(status, &text);
        let _ = tx
            .send(synthetic_error_event(
                &error_message(status, &text),
                &args.model,
                kind,
            ))
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
    // Gemini holds a whole function_call step back until its arguments are
    // generated, so upstream goes silent for minutes inside an open block.
    // Once content started, the router's idle clock ignores pings.
    let mut heartbeat =
        tokio::time::interval_at(tokio::time::Instant::now() + args.heartbeat, args.heartbeat);
    loop {
        let next = tokio::select! {
            // Keep the deadline armed until the first step starts or the
            // stream terminates.
            _ = &mut first_token_deadline, if !state.generation_started() => {
                let _ = tx.send(generation_timeout(&args.model, args.first_token_timeout)).await;
                return; // drop the response and close the queued upstream request
            }
            _ = heartbeat.tick() => {
                if let Some(event) = state.heartbeat()
                    && tx.send(event).await.is_err()
                {
                    return;
                }
                continue;
            }
            next = stream.next() => next,
        };
        let Some(chunk) = next else {
            break;
        };
        let chunk = match chunk {
            Ok(c) => c,
            Err(e) => {
                // Unlike a startup failure, a body read failure can interrupt
                // real output. Keep it in the authoritative terminal so the
                // Harness can preserve and resume the partial response.
                let mut error = build_final(&state, &args.model);
                error.stop_reason = llm_router::types::events::StopReason::Error;
                error.error_message = Some(format!("stream read failed: {}", error_chain(&e)));
                error.error_kind = Some(ErrorKind::Transient);
                let _ = tx.send(AssistantMessageEvent::Error { error }).await;
                return;
            }
        };
        append_utf8_chunk(&mut byte_buf, &mut buf, &chunk);
        if drain_sse_blocks(&mut buf, &tx, &mut |block: &str| {
            decode_block(block, &mut state, &args.model)
        })
        .await
        {
            return; // terminal forwarded, or receiver dropped → abort upstream
        }
    }
    // Body closed: one terminal frame, decided by the shared end-of-stream policy.
    let tail = flush_tail(&mut buf, &tx, &mut |block: &str| {
        decode_block(block, &mut state, &args.model)
    })
    .await;
    if tail == TailFlush::Terminal {
        return;
    }
    // The protocol always ends with `interaction.completed`; a body that
    // closes without it (even after `[DONE]`) is truncated.
    let event = match classify_stream_end(&state, tail, CloseFraming::Rejected) {
        StreamEnd::Complete => AssistantMessageEvent::Done {
            message: build_final(&state, &args.model),
        },
        StreamEnd::Truncated(truncation) => {
            truncated_stream_error(build_final(&state, &args.model), PROVIDER_ID, truncation)
        }
    };
    let _ = tx.send(event).await;
}

fn generation_timeout(model: &str, timeout: Duration) -> AssistantMessageEvent {
    synthetic_error_event(
        &format!(
            "Gemini model {model} did not start generating within {}ms. \
             The request may be queued upstream. Try again or choose another model.",
            timeout.as_millis()
        ),
        model,
        ErrorKind::Transient,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One-shot HTTP stub: accepts a single connection, consumes the request
    /// head, writes `response` verbatim, closes (read-until-close framing).
    async fn stub(response: &str) -> String {
        stub_chunks(vec![response.as_bytes().to_vec()]).await
    }

    /// Like `stub`, but writes each chunk separately with a pause between.
    async fn stub_chunks(chunks: Vec<Vec<u8>>) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 65536];
                let _ = sock.read(&mut buf).await;
                for chunk in chunks {
                    let _ = sock.write_all(&chunk).await;
                    let _ = sock.flush().await;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                let _ = sock.shutdown().await;
            }
        });
        format!("http://{addr}/interactions")
    }

    fn fixture(path: &str) -> String {
        std::fs::read_to_string(format!(
            "{}/tests/fixtures/{path}",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap()
    }

    async fn error_for(
        status_line: &str,
        body: &str,
    ) -> llm_router::types::messages::AssistantMessage {
        let url = stub(&format!(
            "HTTP/1.1 {status_line}\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{body}"
        ))
        .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        single_terminal_error(&events).clone()
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_error_bodies_in_all_three_shapes() {
        let e = error_for("400 Bad Request", &fixture("errors/400_sse_framed.txt")).await;
        assert_eq!(e.error_kind, Some(ErrorKind::Permanent));
        assert_eq!(
            e.error_message.as_deref(),
            Some("Request contains an invalid argument.")
        );
        let e = error_for("400 Bad Request", &fixture("errors/400_bad_key.json")).await;
        assert_eq!(e.error_kind, Some(ErrorKind::AuthExpired));
        let e = error_for("404 Not Found", &fixture("errors/404_unknown_model.json")).await;
        assert_eq!(e.error_kind, Some(ErrorKind::Permanent));
        assert!(
            e.error_message
                .unwrap()
                .starts_with("Model 'gemini-no-such-model-1' not found")
        );
        let e = error_for("400 Bad Request", &fixture("errors/400_overflow.json")).await;
        assert_eq!(e.error_kind, Some(ErrorKind::ContextOverflow));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn live_stream_ends_in_one_done() {
        let url = stub(&format!("{SSE_HEADERS}{}", fixture("sse/text.sse"))).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert_eq!(message.content.len(), 2);
            }
            other => panic!("want done, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn done_without_interaction_completed_is_truncated() {
        let body = fixture("sse/text.sse");
        let cut = body
            .split("\n\n")
            .filter(|b| !b.contains("interaction.completed"))
            .collect::<Vec<_>>()
            .join("\n\n");
        assert!(cut.contains("[DONE]"));
        let url = stub(&format!("{SSE_HEADERS}{cut}")).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let error = single_terminal_error(&events);
        let message = error.error_message.as_deref().unwrap_or_default();
        assert!(message.contains("reason=no_terminator"), "{message}");
        assert_eq!(error.content.len(), 2, "the partial is kept");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn utf8_split_across_chunks_survives() {
        let frame = concat!(
            "event: step.start\ndata: {\"index\":0,\"step\":{\"type\":\"model_output\"},\"event_type\":\"step.start\"}\n\n",
            "event: step.delta\ndata: {\"index\":0,\"delta\":{\"text\":\"olá ✓\",\"type\":\"text\"},\"event_type\":\"step.delta\"}\n\n",
            "event: interaction.completed\ndata: {\"interaction\":{\"status\":\"completed\"},\"event_type\":\"interaction.completed\"}\n\n",
        )
        .as_bytes();
        // Split inside the three-byte check mark.
        let split = frame.windows(3).position(|w| w == "✓".as_bytes()).unwrap() + 1;
        let mut first = SSE_HEADERS.as_bytes().to_vec();
        first.extend_from_slice(&frame[..split]);
        let url = stub_chunks(vec![first, frame[split..].to_vec()]).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => assert_eq!(
                message.content,
                vec![llm_router::types::content::ContentBlock::Text {
                    text: "olá ✓".into()
                }]
            ),
            other => panic!("want done, got {other:?}"),
        }
    }

    fn args(api_url: String) -> UpstreamArgs {
        UpstreamArgs {
            api_url,
            model: "gemini-test".into(),
            body: serde_json::json!({ "stream": true }),
            headers: vec![("x-goog-api-key", "sk-test".into())],
            warnings: vec![],
            first_token_timeout: Duration::from_secs(120),
            heartbeat: HEARTBEAT,
        }
    }

    async fn drain(mut rx: mpsc::Receiver<AssistantMessageEvent>) -> Vec<AssistantMessageEvent> {
        let mut out = Vec::new();
        while let Some(ev) = rx.recv().await {
            out.push(ev);
        }
        out
    }

    /// Keep a real HTTP connection alive without producing model content.
    /// The task completes only when the provider closes its upstream socket.
    async fn waiting_upstream(
        headers: &'static str,
        frame: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut buf = [0u8; 65536];
            assert!(sock.read(&mut buf).await.unwrap() > 0);
            sock.write_all(headers.as_bytes()).await.unwrap();
            let mut interval = tokio::time::interval(Duration::from_millis(10));
            loop {
                tokio::select! {
                    read = sock.read(&mut buf) => {
                        if !matches!(read, Ok(n) if n > 0) {
                            return;
                        }
                    }
                    _ = interval.tick() => {
                        if sock.write_all(frame.as_bytes()).await.is_err() {
                            return;
                        }
                    }
                }
            }
        });
        (format!("http://{addr}/interactions"), task)
    }

    const SSE_HEADERS: &str =
        "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n";

    #[tokio::test(flavor = "multi_thread")]
    async fn keepalives_cannot_extend_the_generation_deadline() {
        {
            let frame = ": keep-alive\n\n";
            let (url, peer) = waiting_upstream(SSE_HEADERS, frame).await;
            let mut input = args(url);
            input.first_token_timeout = Duration::from_millis(100);
            let events = tokio::time::timeout(
                Duration::from_secs(1),
                drain(spawn_upstream(reqwest::Client::new(), input)),
            )
            .await
            .expect("keepalives must not keep a generation alive past its startup budget");
            let error = single_terminal_error(&events);
            assert_eq!(error.error_kind, Some(ErrorKind::Transient));
            assert_eq!(error.model, "gemini-test");
            assert!(
                error.content.is_empty(),
                "a startup timeout is not model output"
            );
            assert!(
                error
                    .error_message
                    .as_deref()
                    .unwrap()
                    .contains("did not start generating")
            );
            tokio::time::timeout(Duration::from_secs(1), peer)
                .await
                .expect("timing out must close the upstream HTTP request")
                .unwrap();
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn generation_deadline_also_bounds_waiting_for_response_headers() {
        let (url, peer) = waiting_upstream("", "").await;
        let mut input = args(url);
        input.first_token_timeout = Duration::from_millis(100);
        let events = tokio::time::timeout(
            Duration::from_secs(1),
            drain(spawn_upstream(reqwest::Client::new(), input)),
        )
        .await
        .expect("waiting for HTTP headers must have the same startup bound");
        assert_eq!(events.len(), 1);
        assert_eq!(
            single_terminal_error(&events).error_kind,
            Some(ErrorKind::Transient)
        );
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .expect("request must close on timeout")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn silence_inside_an_open_thought_sends_empty_deltas() {
        // Repeating the same step.start yields no new frames.
        let thought = "event: step.start\ndata: {\"index\":0,\"step\":{\"type\":\"thought\"},\"event_type\":\"step.start\"}\n\n";
        let (url, _peer) = waiting_upstream(SSE_HEADERS, thought).await;
        let mut input = args(url);
        input.heartbeat = Duration::from_millis(50);
        let mut rx = spawn_upstream(reqwest::Client::new(), input);
        let mut beats = 0;
        while beats < 2 {
            match tokio::time::timeout(Duration::from_secs(1), rx.recv()).await {
                Ok(Some(AssistantMessageEvent::ThinkingDelta {
                    partial: None,
                    delta,
                })) if delta.is_empty() => beats += 1,
                Ok(Some(
                    AssistantMessageEvent::Start { .. }
                    | AssistantMessageEvent::ThinkingStart { .. },
                )) => {}
                other => panic!("want an empty thinking delta, got {other:?}"),
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn dropping_the_receiver_cancels_a_queued_generation() {
        let (url, peer) = waiting_upstream(SSE_HEADERS, ": keep-alive\n\n").await;
        let mut rx = spawn_upstream(reqwest::Client::new(), args(url));
        assert!(matches!(
            rx.recv().await,
            Some(AssistantMessageEvent::Start { .. })
        ));
        drop(rx);
        tokio::time::timeout(Duration::from_secs(1), peer)
            .await
            .expect("caller cancellation must not wait for the startup deadline")
            .unwrap();
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_402_is_a_permanent_error_frame() {
        let url = stub(
            "HTTP/1.1 402 Payment Required\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"code\":\"payment_required\",\"message\":\"billing\"}}",
        )
        .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::Permanent));
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_401_yields_auth_expired_error_frame() {
        let url = stub(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"code\":\"authentication\",\"message\":\"bad key\"}}",
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
        let mut a = args("http://127.0.0.1:1/interactions".into());
        a.headers = vec![("x-goog-api-key", "sk-bad\ninjected".into())];
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                let msg = error.error_message.as_deref().unwrap_or_default();
                assert!(msg.starts_with("gemini fetch failed: "), "got {msg:?}");
                // The source chain was appended past the bare "builder error".
                assert_ne!(msg, "gemini fetch failed: builder error", "source dropped");
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
            format!("http://{}/interactions", l.local_addr().unwrap())
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

    fn single_terminal_error(
        events: &[AssistantMessageEvent],
    ) -> &llm_router::types::messages::AssistantMessage {
        assert_eq!(
            events.iter().filter(|e| e.is_terminal()).count(),
            1,
            "{events:?}"
        );
        match events.last() {
            Some(AssistantMessageEvent::Error { error }) => error,
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn empty_stream_ends_as_one_error_not_an_empty_done() {
        let url =
            stub("HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\n")
                .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let error = single_terminal_error(&events);
        let message = error.error_message.as_deref().unwrap_or_default();
        assert!(message.contains("reason=empty"), "{message}");
    }
}
