//! UTF-16 surrogate sanitisation and SSE event-block parsing.

/// Strip any encoding artefacts that would break downstream JSON serialisation.
///
/// Rust `&str` is guaranteed UTF-8 valid, so unpaired UTF-16 surrogates cannot
/// be present at this call site (they would have been replaced with U+FFFD
/// during decoding). The function is kept to mirror the upstream contract:
/// providers that build strings from raw `Vec<u16>` should normalise to UTF-8
/// before calling this. We additionally strip explicit U+FFFD characters that
/// some providers insert to mark dropped surrogate halves.
pub fn sanitize_surrogates(input: &str) -> String {
    input.chars().filter(|c| *c != '\u{FFFD}').collect()
}

/// One parsed SSE event: every `event:` line collapsed and every `data:` line
/// concatenated. Returns `None` for keep-alive comments.
pub struct SseEvent {
    pub event_type: Option<String>,
    pub data: String,
}

/// Parse a complete SSE block delimited by a blank line.
///
/// The block is the substring between two `\n\n` boundaries; the caller is
/// responsible for buffering raw bytes and splitting on blank lines.
pub fn parse_sse_block(block: &str) -> Option<SseEvent> {
    let mut event_type: Option<String> = None;
    let mut data = String::new();
    let mut had_data = false;
    for line in block.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("event:") {
            event_type = Some(value.trim().to_string());
            continue;
        }
        if let Some(value) = line.strip_prefix("data:") {
            if had_data {
                data.push('\n');
            }
            data.push_str(value.trim_start());
            had_data = true;
        }
    }
    if had_data {
        Some(SseEvent { event_type, data })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paired_emoji_preserved() {
        let s = "Hello 🙈 World";
        assert_eq!(sanitize_surrogates(s), s);
    }

    #[test]
    fn replacement_char_stripped() {
        // U+FFFD is what Rust's lossy UTF-8 decoders insert for invalid input.
        let s = "Text \u{FFFD}here";
        assert_eq!(sanitize_surrogates(s), "Text here");
    }

    #[test]
    fn parses_simple_event() {
        let block = "event: foo\ndata: hello";
        let parsed = parse_sse_block(block).unwrap();
        assert_eq!(parsed.event_type.as_deref(), Some("foo"));
        assert_eq!(parsed.data, "hello");
    }

    #[test]
    fn parses_multiline_data() {
        let block = "data: line1\ndata: line2";
        let parsed = parse_sse_block(block).unwrap();
        assert_eq!(parsed.data, "line1\nline2");
    }

    #[test]
    fn skips_comment_lines() {
        let block = ": ping\ndata: x";
        let parsed = parse_sse_block(block).unwrap();
        assert_eq!(parsed.data, "x");
    }

    #[test]
    fn no_data_returns_none() {
        let block = ": ping only";
        assert!(parse_sse_block(block).is_none());
    }
}
