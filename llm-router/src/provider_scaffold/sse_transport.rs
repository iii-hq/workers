//! Transport-layer SSE helpers shared by provider upstream readers:
//! error-chain flattening, cross-chunk UTF-8 buffering, CRLF
//! normalization, and `\n\n`-delimited block draining. Decoding the
//! blocks into events stays provider-specific (the closure).
use crate::types::events::{AssistantMessageEvent, ErrorKind, StopReason};
use crate::types::messages::AssistantMessage;
use tokio::sync::mpsc;

/// Flatten an error and its `source()` chain into one string. reqwest's
/// top-level Display for a builder error is just "builder error"; the real
/// cause (invalid header value, bad URL) lives in the source chain, so without
/// this the message is undiagnosable.
pub fn error_chain(e: &dyn std::error::Error) -> String {
    let mut msg = e.to_string();
    let mut src = e.source();
    while let Some(s) = src {
        let next = s.to_string();
        // reqwest sometimes nests the same text; skip exact repeats.
        if !msg.ends_with(&next) {
            msg.push_str(": ");
            msg.push_str(&next);
        }
        src = s.source();
    }
    msg
}

/// Append chunk bytes to `text`, retaining any trailing incomplete UTF-8
/// sequence in `byte_buf`. Network chunks split multibyte codepoints; a
/// per-chunk lossy conversion corrupts them to U+FFFD.
pub fn append_utf8_chunk(byte_buf: &mut Vec<u8>, text: &mut String, chunk: &[u8]) {
    byte_buf.extend_from_slice(chunk);
    let mut consumed = 0usize;
    loop {
        match std::str::from_utf8(&byte_buf[consumed..]) {
            Ok(s) => {
                text.push_str(s);
                byte_buf.clear();
                return;
            }
            Err(e) => {
                let valid = e.valid_up_to();
                if valid > 0 {
                    // SAFETY: valid_up_to guarantees valid UTF-8 in this prefix.
                    text.push_str(unsafe {
                        std::str::from_utf8_unchecked(&byte_buf[consumed..consumed + valid])
                    });
                    consumed += valid;
                }
                match e.error_len() {
                    Some(invalid) => {
                        byte_buf.drain(..consumed + invalid);
                        text.push('\u{FFFD}');
                        consumed = 0;
                    }
                    None => {
                        if consumed > 0 {
                            byte_buf.drain(..consumed);
                        }
                        return;
                    }
                }
            }
        }
    }
}

/// SSE allows CRLF line endings; normalize so `\n\n` block framing holds.
pub fn normalize_crlf(text: &mut String) {
    if text.contains("\r\n") {
        *text = text.replace("\r\n", "\n");
    }
}

/// Drain complete `\n\n`-delimited SSE blocks from `text`, decoding each with
/// `handle_block` and forwarding the events. Returns true when a terminal
/// event was forwarded or the receiver is gone.
pub async fn drain_sse_blocks<F>(
    text: &mut String,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    handle_block: &mut F,
) -> bool
where
    F: FnMut(&str) -> Vec<AssistantMessageEvent>,
{
    normalize_crlf(text);
    while let Some(idx) = text.find("\n\n") {
        let block: String = text.drain(..idx + 2).collect();
        for ev in handle_block(&block) {
            let terminal = ev.is_terminal();
            if tx.send(ev).await.is_err() {
                return true;
            }
            if terminal {
                return true;
            }
        }
    }
    false
}

/// The slice of a provider's decoder state the end-of-stream policy needs.
pub trait StreamEndView {
    /// `finish_reason`, `message_stop`, or `response.completed` arrived.
    fn saw_terminator(&self) -> bool;
    /// Text, thinking, or a function call started.
    fn has_content(&self) -> bool;
    /// A call block never closed, or its arguments are not one complete JSON object.
    fn has_unfinished_call(&self) -> bool;
}

/// Whether a body close without a protocol terminator may count as complete
/// (Chat Completions gateways sometimes skip `[DONE]`; Messages never skips `message_stop`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseFraming {
    Accepted,
    Rejected,
}

/// Why a stream counts as cut short.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Truncation {
    /// The body ended inside an SSE block or inside a `data:` JSON payload.
    PartialFrame { bytes: usize },
    /// A function call's arguments never closed.
    OpenFunctionCall,
    /// No content arrived before the body closed.
    Empty,
    /// The protocol requires a terminator and none arrived.
    NoTerminator,
}

impl Truncation {
    /// Stable, greppable tag for logs and error details.
    pub fn tag(self) -> &'static str {
        match self {
            Truncation::PartialFrame { .. } => "partial_frame",
            Truncation::OpenFunctionCall => "open_function_call",
            Truncation::Empty => "empty",
            Truncation::NoTerminator => "no_terminator",
        }
    }

    pub fn describe(self) -> String {
        match self {
            Truncation::PartialFrame { bytes } => {
                format!("the body ended inside an event frame ({bytes} undecoded bytes)")
            }
            Truncation::OpenFunctionCall => {
                "the body ended inside a function call's arguments".to_string()
            }
            Truncation::Empty => "the body ended before any output".to_string(),
            Truncation::NoTerminator => "the body ended before the completion event".to_string(),
        }
    }
}

/// Outcome of the end-of-stream policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamEnd {
    Complete,
    Truncated(Truncation),
}

/// How the undecoded tail of the body was handled by [`flush_tail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TailFlush {
    /// Decoding the tail forwarded a terminal event; the caller is done.
    Terminal,
    /// Nothing was left, or the tail decoded as a complete block.
    Clean,
    /// The tail is an unterminated frame; it was not decoded.
    Partial { bytes: usize },
}

/// True when `tail` has no `data:` line or its payload is not complete JSON (nor `[DONE]`).
pub fn tail_frame_is_partial(tail: &str) -> bool {
    let tail = tail.trim();
    if tail.is_empty() {
        return false;
    }
    let Some(data) = tail
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .next_back()
    else {
        return true;
    };
    let data = data.trim();
    if data == "[DONE]" {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(data).is_err()
}

/// Decode the body's undelimited tail; an unterminated frame is reported, not dropped.
pub async fn flush_tail<F>(
    text: &mut String,
    tx: &mpsc::Sender<AssistantMessageEvent>,
    handle_block: &mut F,
) -> TailFlush
where
    F: FnMut(&str) -> Vec<AssistantMessageEvent>,
{
    normalize_crlf(text);
    if text.trim().is_empty() {
        text.clear();
        return TailFlush::Clean;
    }
    if tail_frame_is_partial(text) {
        let bytes = text.trim().len();
        text.clear();
        return TailFlush::Partial { bytes };
    }
    let mut block = std::mem::take(text);
    block.push_str("\n\n");
    if drain_sse_blocks(&mut block, tx, handle_block).await {
        TailFlush::Terminal
    } else {
        TailFlush::Clean
    }
}

/// One end-of-stream policy for every provider.
pub fn classify_stream_end(
    state: &dyn StreamEndView,
    tail: TailFlush,
    framing: CloseFraming,
) -> StreamEnd {
    if state.saw_terminator() {
        return StreamEnd::Complete;
    }
    if let TailFlush::Partial { bytes } = tail {
        return StreamEnd::Truncated(Truncation::PartialFrame { bytes });
    }
    if state.has_unfinished_call() {
        return StreamEnd::Truncated(Truncation::OpenFunctionCall);
    }
    if !state.has_content() {
        return StreamEnd::Truncated(Truncation::Empty);
    }
    match framing {
        CloseFraming::Accepted => StreamEnd::Complete,
        CloseFraming::Rejected => StreamEnd::Truncated(Truncation::NoTerminator),
    }
}

/// The terminal error frame for a cut stream; keeps the partial, names the phase, logs no payload.
pub fn truncated_stream_error(
    mut partial: AssistantMessage,
    provider: &str,
    truncation: Truncation,
) -> AssistantMessageEvent {
    let message = format!(
        "{provider} stream truncated: {} [phase=sse-decode reason={}]",
        truncation.describe(),
        truncation.tag()
    );
    tracing::warn!(
        provider,
        model = %partial.model,
        phase = "sse-decode",
        reason = truncation.tag(),
        content_blocks = partial.content.len(),
        "provider stream truncated"
    );
    partial.stop_reason = StopReason::Error;
    partial.error_message = Some(message);
    partial.error_kind = Some(ErrorKind::Transient);
    AssistantMessageEvent::Error { error: partial }
}

/// True when `args_json` is not (yet) one complete JSON object.
pub fn arguments_incomplete(args_json: &str) -> bool {
    if args_json.trim().is_empty() {
        return false;
    }
    !matches!(
        serde_json::from_str::<serde_json::Value>(args_json),
        Ok(serde_json::Value::Object(_))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct View {
        terminator: bool,
        content: bool,
        open_call: bool,
    }

    impl StreamEndView for View {
        fn saw_terminator(&self) -> bool {
            self.terminator
        }
        fn has_content(&self) -> bool {
            self.content
        }
        fn has_unfinished_call(&self) -> bool {
            self.open_call
        }
    }

    fn view(terminator: bool, content: bool, open_call: bool) -> View {
        View {
            terminator,
            content,
            open_call,
        }
    }

    #[test]
    fn tail_partial_detection() {
        assert!(!tail_frame_is_partial(""));
        assert!(!tail_frame_is_partial("\n  \n"));
        assert!(!tail_frame_is_partial("data: [DONE]"));
        assert!(!tail_frame_is_partial("data: {\"choices\":[]}"));
        assert!(!tail_frame_is_partial("event: x\ndata: {\"a\":1}\n"));
        assert!(tail_frame_is_partial(
            "data: {\"choices\":[{\"delta\":{\"content\":\"he"
        ));
        assert!(tail_frame_is_partial("dat"));
        assert!(tail_frame_is_partial("event: content_block_delta\n"));
    }

    #[tokio::test]
    async fn flush_tail_decodes_a_block_missing_its_blank_line() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut text = "data: {\"a\":1}".to_string();
        let mut seen = Vec::new();
        let flush = flush_tail(&mut text, &tx, &mut |block: &str| {
            seen.push(block.to_string());
            vec![AssistantMessageEvent::Ping]
        })
        .await;
        assert_eq!(flush, TailFlush::Clean);
        assert_eq!(seen, vec!["data: {\"a\":1}\n\n"]);
        assert!(text.is_empty());
        assert!(matches!(rx.try_recv(), Ok(AssistantMessageEvent::Ping)));
    }

    #[tokio::test]
    async fn flush_tail_reports_an_unterminated_frame_without_decoding() {
        let (tx, _rx) = mpsc::channel(8);
        let mut text =
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"function\":{\"argu".to_string();
        let bytes = text.len();
        let mut calls = 0;
        let flush = flush_tail(&mut text, &tx, &mut |_block: &str| {
            calls += 1;
            vec![]
        })
        .await;
        assert_eq!(flush, TailFlush::Partial { bytes });
        assert_eq!(calls, 0);
        assert!(text.is_empty());
    }

    #[tokio::test]
    async fn flush_tail_surfaces_a_terminal_from_the_tail() {
        let (tx, _rx) = mpsc::channel(8);
        let mut text = "data: [DONE]".to_string();
        let flush = flush_tail(&mut text, &tx, &mut |_block: &str| {
            vec![AssistantMessageEvent::Done {
                message: empty("m"),
            }]
        })
        .await;
        assert_eq!(flush, TailFlush::Terminal);
    }

    #[test]
    fn terminator_wins_over_everything() {
        let end = classify_stream_end(
            &view(true, false, true),
            TailFlush::Partial { bytes: 9 },
            CloseFraming::Rejected,
        );
        assert_eq!(end, StreamEnd::Complete);
    }

    #[test]
    fn partial_frame_is_a_cut() {
        let end = classify_stream_end(
            &view(false, true, false),
            TailFlush::Partial { bytes: 9 },
            CloseFraming::Accepted,
        );
        assert_eq!(
            end,
            StreamEnd::Truncated(Truncation::PartialFrame { bytes: 9 })
        );
    }

    #[test]
    fn open_function_call_is_a_cut_even_with_close_framing() {
        let end = classify_stream_end(
            &view(false, true, true),
            TailFlush::Clean,
            CloseFraming::Accepted,
        );
        assert_eq!(end, StreamEnd::Truncated(Truncation::OpenFunctionCall));
    }

    #[test]
    fn empty_body_is_a_cut() {
        let end = classify_stream_end(
            &view(false, false, false),
            TailFlush::Clean,
            CloseFraming::Accepted,
        );
        assert_eq!(end, StreamEnd::Truncated(Truncation::Empty));
    }

    #[test]
    fn text_only_close_depends_on_framing() {
        let accepted = classify_stream_end(
            &view(false, true, false),
            TailFlush::Clean,
            CloseFraming::Accepted,
        );
        assert_eq!(accepted, StreamEnd::Complete);
        let rejected = classify_stream_end(
            &view(false, true, false),
            TailFlush::Clean,
            CloseFraming::Rejected,
        );
        assert_eq!(rejected, StreamEnd::Truncated(Truncation::NoTerminator));
    }

    #[test]
    fn arguments_completeness() {
        assert!(!arguments_incomplete(""));
        assert!(!arguments_incomplete("{}"));
        assert!(!arguments_incomplete("{\"path\":\"/tmp\"}"));
        assert!(arguments_incomplete("{\"path\":\"/tm"));
        assert!(arguments_incomplete("[1,2]"));
        assert!(arguments_incomplete("\"text\""));
    }

    #[test]
    fn truncated_error_keeps_partial_content_and_names_the_phase() {
        let mut partial = empty("gpt-test");
        partial.content = vec![crate::types::content::ContentBlock::Text {
            text: "partial".into(),
        }];
        let ev = truncated_stream_error(partial, "openai", Truncation::OpenFunctionCall);
        let AssistantMessageEvent::Error { error } = ev else {
            panic!("want error");
        };
        assert_eq!(error.stop_reason, StopReason::Error);
        assert_eq!(error.error_kind, Some(ErrorKind::Transient));
        assert_eq!(error.content.len(), 1);
        let message = error.error_message.unwrap();
        assert!(
            message.starts_with("openai stream truncated: "),
            "{message}"
        );
        assert!(message.contains("phase=sse-decode"), "{message}");
        assert!(message.contains("reason=open_function_call"), "{message}");
    }

    fn empty(model: &str) -> AssistantMessage {
        crate::chat::synthesize::empty_partial(model, "test-provider", 0)
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

    #[test]
    fn utf8_split_across_three_chunks_is_preserved() {
        // 4-byte emoji delivered one byte at a time — the worst case a
        // network stream can produce.
        let s = "héllo 🌍!";
        let bytes = s.as_bytes();
        let mut byte_buf = Vec::new();
        let mut text = String::new();
        for b in bytes {
            append_utf8_chunk(&mut byte_buf, &mut text, std::slice::from_ref(b));
        }
        assert_eq!(text, s);
        assert!(byte_buf.is_empty());
    }

    #[test]
    fn invalid_bytes_become_replacement_chars_without_stalling() {
        let mut byte_buf = Vec::new();
        let mut text = String::new();
        append_utf8_chunk(&mut byte_buf, &mut text, b"ok\xFF\xFEok");
        assert_eq!(text, "ok\u{FFFD}\u{FFFD}ok");
        assert!(byte_buf.is_empty());
    }

    #[test]
    fn crlf_blocks_are_reframed() {
        let mut text = "data: a\r\n\r\ndata: b".to_string();
        normalize_crlf(&mut text);
        assert_eq!(text, "data: a\n\ndata: b");
    }

    #[tokio::test]
    async fn drains_blocks_and_stops_on_terminal() {
        let (tx, mut rx) = mpsc::channel(8);
        let mut text = "one\n\ntwo\n\nrest".to_string();
        let mut seen = Vec::new();
        let done = drain_sse_blocks(&mut text, &tx, &mut |block: &str| {
            seen.push(block.to_string());
            vec![AssistantMessageEvent::Ping]
        })
        .await;
        assert!(!done);
        assert_eq!(seen, vec!["one\n\n", "two\n\n"]);
        assert_eq!(text, "rest");
        assert!(matches!(rx.try_recv(), Ok(AssistantMessageEvent::Ping)));
    }
}
