//! Split long Telegram messages at the 4096-character limit.

pub const TELEGRAM_MAX_MESSAGE_LEN: usize = 4096;

/// Split `text` into chunks of at most `max_len` bytes (UTF-8 safe).
pub fn split_message(text: &str, max_len: usize) -> Vec<String> {
    if text.is_empty() {
        return Vec::new();
    }
    if text.len() <= max_len {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        if rest.len() <= max_len {
            chunks.push(rest.to_string());
            break;
        }
        let mut split_at = max_len;
        while split_at > 0 && !rest.is_char_boundary(split_at) {
            split_at -= 1;
        }
        if split_at == 0 {
            split_at = rest
                .char_indices()
                .nth(1)
                .map(|(i, _)| i)
                .unwrap_or(rest.len());
        }
        chunks.push(rest[..split_at].to_string());
        rest = &rest[split_at..];
    }
    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_single_chunk() {
        assert_eq!(split_message("hello", 4096), vec!["hello"]);
    }

    #[test]
    fn splits_long_text() {
        let text = "a".repeat(5000);
        let chunks = split_message(&text, 4096);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), 4096);
        assert_eq!(chunks[1].len(), 904);
    }

    #[test]
    fn utf8_safe_split() {
        let text = "🙂".repeat(3000);
        let chunks = split_message(&text, 4096);
        for chunk in &chunks {
            assert!(chunk.len() <= 4096);
            assert!(std::str::from_utf8(chunk.as_bytes()).is_ok());
        }
    }
}
