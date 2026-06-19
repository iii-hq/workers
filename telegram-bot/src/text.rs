//! Shared text helpers.

/// Truncate `s` to at most `max_bytes` UTF-8 bytes, appending an ellipsis when shortened.
pub fn truncate_ellipsis(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_ellipsis_respects_utf8_boundary() {
        let s = "hello 🌍 world";
        let out = truncate_ellipsis(s, 8);
        assert!(out.ends_with('…'));
        assert!(std::str::from_utf8(out.as_bytes()).is_ok());
    }
}
