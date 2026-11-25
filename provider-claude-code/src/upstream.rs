//! POST /v1/messages (stream:true) → SSE → mpsc<AssistantMessageEvent>.
//! The receiver dropping aborts the upstream: every send error returns,
//! which drops the reqwest response mid-body and closes the connection.
use crate::PROVIDER_ID;
use crate::errors::{REQUEST_REFUSED, SIGN_IN_REJECTED, USAGE_LIMIT_REACHED, classify};
use crate::limits::usage_limit_details;
use crate::request::auth_header;
use crate::sse::{
    PartialState, build_partial, handle_sse_event, synthetic_error_event,
    synthetic_error_event_from_state,
};
use futures::StreamExt;
use futures::future::BoxFuture;
use llm_router::provider_scaffold::sse_transport::{
    CloseFraming, StreamEnd, TailFlush, append_utf8_chunk, classify_stream_end, drain_sse_blocks,
    error_chain, tail_frame_is_partial, truncated_stream_error,
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
    /// A token from an account source: one more request after a 401 (D140).
    /// None keeps a single attempt (the vault / `~/.claude` path).
    pub reauth: Option<Reauth>,
}

/// What a 401 does to a token from an account source.
pub struct Reauth {
    /// Drops the refused token and asks the source once more: `Some(token)`
    /// when it has a different one. Runs only after a 401.
    pub ask_again: BoxFuture<'static, Option<String>>,
    /// What a 401 that survives says (it names the account).
    pub rejected: String,
}

/// Swap the bearer for `token`.
fn set_bearer(headers: &mut [(&'static str, String)], token: &str) {
    let (name, value) = auth_header(token);
    for header in headers.iter_mut().filter(|(n, _)| *n == name) {
        header.1 = value.clone();
    }
}

/// POST the request. It takes the parts, not `UpstreamArgs`: the `Reauth`
/// future in there is `Send` but not `Sync`.
async fn send(
    client: &reqwest::Client,
    api_url: &str,
    headers: &[(&'static str, String)],
    body: &Value,
) -> Result<reqwest::Response, reqwest::Error> {
    let mut req = client.post(api_url);
    for (name, value) in headers {
        req = req.header(*name, value);
    }
    req.json(body).send().await
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

/// An auth refusal says what happened; Anthropic's own reason, when it gave
/// one, follows in parentheses.
fn refused(summary: &str, body: &str) -> String {
    let reason = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.pointer("/error/message")?.as_str().map(str::to_owned))
        .filter(|reason| !reason.is_empty());
    match reason {
        Some(reason) => format!("{summary} ({reason})"),
        None => summary.to_string(),
    }
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
    mut args: UpstreamArgs,
    tx: mpsc::Sender<AssistantMessageEvent>,
) {
    let fetch_failed = |e: reqwest::Error, model: &str| {
        synthetic_error_event(
            &format!("claude-code fetch failed: {}", error_chain(&e)),
            model,
            ErrorKind::Transient,
        )
    };
    let mut resp = match send(&client, &args.api_url, &args.headers, &args.body).await {
        Ok(r) => r,
        Err(e) => {
            let _ = tx.send(fetch_failed(e, &args.model)).await;
            return;
        }
    };
    let mut rejected = SIGN_IN_REJECTED.to_string();
    if let Some(reauth) = args.reauth.take() {
        rejected = reauth.rejected;
        // A 401 drops the token and asks its source once more (D140).
        if resp.status().as_u16() == 401
            && let Some(token) = reauth.ask_again.await
        {
            set_bearer(&mut args.headers, &token);
            resp = match send(&client, &args.api_url, &args.headers, &args.body).await {
                Ok(r) => r,
                Err(e) => {
                    let _ = tx.send(fetch_failed(e, &args.model)).await;
                    return;
                }
            };
        }
    }

    let status = resp.status();
    if !status.is_success() {
        // Headers first: reading the body consumes the response.
        let details = usage_limit_details(status.as_u16(), resp.headers());
        let text = resp.text().await.unwrap_or_default();
        // A usage limit stays `rate_limited` on the wire (D147), whatever
        // the body's error type says.
        let kind = if details.is_some() {
            ErrorKind::RateLimited
        } else {
            classify(Some(status.as_u16()), &text)
        };
        // Only a 401 means the token itself was turned down; a new sign-in
        // does not fix a 403.
        let msg = if status.as_u16() == 401 {
            refused(&rejected, &text)
        } else if details.is_some() {
            refused(USAGE_LIMIT_REACHED, &text)
        } else if kind == ErrorKind::AuthExpired {
            refused(REQUEST_REFUSED, &text)
        } else if text.is_empty() {
            format!("claude-code http {status}")
        } else {
            text
        };
        let mut event = synthetic_error_event(&msg, &args.model, kind);
        if let AssistantMessageEvent::Error { error } = &mut event {
            error.error_details = details;
        }
        let _ = tx.send(event).await;
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
                        &format!("stream read failed: {}", error_chain(&e)),
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
    // A final block missing only its blank line still decodes; a cut frame is reported.
    let tail = if tail_frame_is_partial(&text) {
        TailFlush::Partial {
            bytes: text.trim().len(),
        }
    } else if text.trim().is_empty() {
        TailFlush::Clean
    } else {
        let remainder = std::mem::take(&mut text);
        if drain_blocks(&mut (remainder + "\n\n"), &mut state, &args.model, &tx).await {
            return;
        }
        TailFlush::Clean
    };

    // Messages always ends with message_stop, so close framing is never accepted.
    let event = match classify_stream_end(&state, tail, CloseFraming::Rejected) {
        StreamEnd::Complete => AssistantMessageEvent::Done {
            message: build_partial(&state, &args.model),
        },
        StreamEnd::Truncated(truncation) => {
            truncated_stream_error(build_partial(&state, &args.model), PROVIDER_ID, truncation)
        }
    };
    let _ = tx.send(event).await;
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
            reauth: None,
        }
    }

    const UNAUTHORIZED: &str = "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\nconnection: close\r\n\r\n{\"error\":{\"message\":\"token revoked\"}}";

    /// HTTP stub for many requests: `HAPPY` for `authorization: Bearer
    /// <good>`, 401 otherwise; records each request's bearer.
    async fn bearer_stub(
        good: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let record = seen.clone();
        tokio::spawn(async move {
            while let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0u8; 65536];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let head = String::from_utf8_lossy(&buf[..n]).to_lowercase();
                let bearer = head
                    .lines()
                    .find_map(|line| line.strip_prefix("authorization: bearer "))
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let response = if bearer == good { HAPPY } else { UNAUTHORIZED };
                record.lock().unwrap().push(bearer);
                let _ = sock.write_all(response.as_bytes()).await;
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/v1/messages"), seen)
    }

    fn bearer_args(url: String, token: &str, reauth: Option<Reauth>) -> UpstreamArgs {
        UpstreamArgs {
            headers: vec![
                auth_header(token),
                ("anthropic-version", "2023-06-01".into()),
            ],
            reauth,
            ..args(url)
        }
    }

    fn reauth_to(token: Option<&'static str>) -> Reauth {
        Reauth {
            ask_again: Box::pin(async move { token.map(str::to_string) }),
            rejected: "Claude Code rejected the sign-in of the account \"personal\".".into(),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_401_asks_the_account_source_once_more_and_streams_with_the_new_token() {
        let (url, seen) = bearer_stub("fresh").await;
        let a = bearer_args(url, "stale", Some(reauth_to(Some("fresh"))));
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        assert!(
            matches!(events.last(), Some(AssistantMessageEvent::Done { .. })),
            "{events:?}"
        );
        assert_eq!(*seen.lock().unwrap(), ["stale", "fresh"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_401_that_survives_the_second_ask_is_auth_expired_for_the_account() {
        let (url, seen) = bearer_stub("never").await;
        let a = bearer_args(url, "stale", Some(reauth_to(Some("also-refused"))));
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        assert_eq!(events.len(), 1, "{events:?}");
        let AssistantMessageEvent::Error { error } = &events[0] else {
            panic!("want error, got {events:?}")
        };
        assert_eq!(error.error_kind, Some(ErrorKind::AuthExpired));
        assert_eq!(
            error.error_message.as_deref(),
            Some("Claude Code rejected the sign-in of the account \"personal\". (token revoked)")
        );
        assert_eq!(*seen.lock().unwrap(), ["stale", "also-refused"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn no_new_token_means_no_second_request() {
        let (url, seen) = bearer_stub("never").await;
        let a = bearer_args(url, "stale", Some(reauth_to(None)));
        let events = drain(spawn_upstream(reqwest::Client::new(), a)).await;
        assert!(matches!(
            &events[..],
            [AssistantMessageEvent::Error { error }] if error.error_kind == Some(ErrorKind::AuthExpired)
        ));
        assert_eq!(*seen.lock().unwrap(), ["stale"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn the_vault_path_keeps_one_attempt_on_a_401() {
        let (url, seen) = bearer_stub("never").await;
        let events = drain(spawn_upstream(
            reqwest::Client::new(),
            bearer_args(url, "stale", None),
        ))
        .await;
        assert_eq!(events.len(), 1);
        assert_eq!(*seen.lock().unwrap(), ["stale"]);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_usage_limit_429_is_rate_limited_with_its_details() {
        let url = stub(
            "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\nanthropic-ratelimit-unified-status: rejected\r\nanthropic-ratelimit-unified-representative-claim: seven_day\r\nanthropic-ratelimit-unified-reset: 1800000000\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"usage limit\"}}",
        )
        .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let [AssistantMessageEvent::Error { error }] = &events[..] else {
            panic!("want one error, got {events:?}")
        };
        assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
        assert_eq!(
            error.error_details,
            Some(llm_router::types::events::ErrorDetails::usage_limit(
                Some(llm_router::types::events::UsageWindow::Week),
                Some(1_800_000_000_000)
            ))
        );
        assert_eq!(
            error.error_message.as_deref(),
            Some("This Claude Code account has reached its usage limit. (usage limit)")
        );
        assert!(!error.error_is_retryable());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_busy_429_stays_a_plain_retried_rate_limit() {
        let url = stub(
            "HTTP/1.1 429 Too Many Requests\r\ncontent-type: application/json\r\nanthropic-ratelimit-unified-status: allowed_warning\r\nretry-after: 3\r\nconnection: close\r\n\r\n{\"type\":\"error\",\"error\":{\"type\":\"rate_limit_error\",\"message\":\"slow down\"}}",
        )
        .await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let [AssistantMessageEvent::Error { error }] = &events[..] else {
            panic!("want one error, got {events:?}")
        };
        assert_eq!(error.error_kind, Some(ErrorKind::RateLimited));
        assert_eq!(error.error_details, None);
        assert!(error.error_is_retryable());
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
    async fn terminal_in_unterminated_final_chunk_yields_single_terminal() {
        // The connection closes right after message_stop, with no trailing
        // "\n\n", so the terminal is decoded only in the post-loop flush.
        // Exactly one terminal must reach the receiver — the flush's Done, not
        // a second synthetic "ended before message_stop".
        let body: &'static str = Box::leak(
            HAPPY
                .strip_suffix("\n\n")
                .unwrap()
                .to_string()
                .into_boxed_str(),
        );
        let url = stub(body).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert_eq!(
            events.iter().filter(|e| e.is_terminal()).count(),
            1,
            "exactly one terminal frame"
        );
        assert!(matches!(
            events.last(),
            Some(AssistantMessageEvent::Done { .. })
        ));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn truncated_stream_yields_error_not_done() {
        let url = stub(TRUNCATED).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        match events.last() {
            Some(AssistantMessageEvent::Error { error }) => {
                assert_eq!(error.error_kind, Some(ErrorKind::Transient));
                let message = error.error_message.as_deref().unwrap_or_default();
                assert!(message.contains("stream truncated"), "{message}");
                assert!(message.contains("reason=no_terminator"), "{message}");
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn interrupted_body_read_names_its_source_and_stays_transient() {
        // The server promises more bytes than it sends, then closes: hyper
        // fails the body read mid-stream and reqwest wraps that as the opaque
        // "error decoding response body". The frame must carry the cause.
        let sent = TRUNCATED.split_once("\r\n\r\n").unwrap().1;
        let response: &'static str = Box::leak(
            format!(
                "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: {}\r\n\r\n{sent}",
                sent.len() + 4096
            )
            .into_boxed_str(),
        );
        let url = stub(response).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let error = single_terminal_error(&events);
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        let message = error.error_message.as_deref().unwrap_or_default();
        assert!(message.starts_with("stream read failed: "), "{message}");
        assert_ne!(
            message, "stream read failed: error decoding response body",
            "source dropped"
        );
        assert!(
            message.matches(':').count() >= 2,
            "no source segment: {message:?}"
        );
        assert!(matches!(
            error.content.first(),
            Some(llm_router::types::content::ContentBlock::Text { text }) if text == "partial"
        ));
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
                assert_eq!(
                    error.error_message.as_deref(),
                    Some(
                        "Claude Code rejected the current sign-in. Run `claude` and /login, \
                         then try again. (invalid x-api-key)"
                    )
                );
            }
            other => panic!("want error, got {other:?}"),
        }
    }

    #[test]
    fn a_refusal_without_a_reason_still_says_what_happened() {
        assert_eq!(refused(SIGN_IN_REJECTED, ""), SIGN_IN_REJECTED);
        assert_eq!(
            refused(SIGN_IN_REJECTED, "{\"error\":{}}"),
            SIGN_IN_REJECTED
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn http_403_refuses_the_request_without_asking_for_a_new_sign_in() {
        let url = stub("HTTP/1.1 403 Forbidden\r\nconnection: close\r\n\r\n").await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert_eq!(events.len(), 1);
        match &events[0] {
            AssistantMessageEvent::Error { error } => {
                assert_eq!(error.error_kind, Some(ErrorKind::AuthExpired));
                assert_eq!(error.error_message.as_deref(), Some(REQUEST_REFUSED));
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

    const TRUNCATED_MID_ARGUMENTS: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Listing\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":1,\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_1\",\"name\":\"shell__fs__ls\",\"input\":{}}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":1,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"/tm\"}}\n\n";

    const TRUNCATED_MID_FRAME: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\" wor";

    const FINAL_BLOCK_WITHOUT_BLANK_LINE: &str = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\nevent: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\nevent: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello\"}}\n\nevent: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\nevent: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\nevent: message_stop\ndata: {\"type\":\"message_stop\"}";

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
    async fn stream_cut_inside_function_call_arguments_ends_as_one_error() {
        let url = stub(TRUNCATED_MID_ARGUMENTS).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let error = single_terminal_error(&events);
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        let message = error.error_message.as_deref().unwrap_or_default();
        assert!(message.contains("stream truncated"), "{message}");
        assert!(message.contains("reason=open_function_call"), "{message}");
        assert!(error.content.iter().any(
            |b| matches!(b, llm_router::types::content::ContentBlock::Text { text } if text == "Listing")
        ));
        let call = error.content.iter().find_map(|b| match b {
            llm_router::types::content::ContentBlock::FunctionCall { arguments, .. } => {
                Some(arguments)
            }
            _ => None,
        });
        let arguments = call.expect("function call block kept as evidence");
        assert!(
            arguments.get("_partial").is_some() || arguments.get("_raw").is_some(),
            "arguments must be marked degraded: {arguments}"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn stream_cut_inside_an_event_frame_ends_as_one_error() {
        let url = stub(TRUNCATED_MID_FRAME).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        let error = single_terminal_error(&events);
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        let message = error.error_message.as_deref().unwrap_or_default();
        assert!(message.contains("reason=partial_frame"), "{message}");
        assert!(error.content.iter().any(
            |b| matches!(b, llm_router::types::content::ContentBlock::Text { text } if text == "Hello")
        ));
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

    #[tokio::test(flavor = "multi_thread")]
    async fn final_block_without_blank_line_still_completes() {
        let url = stub(FINAL_BLOCK_WITHOUT_BLANK_LINE).await;
        let events = drain(spawn_upstream(reqwest::Client::new(), args(url))).await;
        assert_eq!(events.iter().filter(|e| e.is_terminal()).count(), 1);
        match events.last() {
            Some(AssistantMessageEvent::Done { message }) => {
                assert_eq!(message.native_stop_reason.as_deref(), Some("end_turn"));
            }
            other => panic!("want done, got {other:?}"),
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
