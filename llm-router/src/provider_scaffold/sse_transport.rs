//! Transport-layer SSE helpers shared by provider upstream readers:
//! error-chain flattening, cross-chunk UTF-8 buffering, CRLF
//! normalization, and `\n\n`-delimited block draining. Decoding the
//! blocks into events stays provider-specific (the closure).
use crate::types::events::AssistantMessageEvent;
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

#[cfg(test)]
mod tests {
    use super::*;

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
