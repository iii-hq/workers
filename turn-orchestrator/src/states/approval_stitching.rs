//! Pure helpers for stitching resolved approval records into LLM turn messages.

use serde_json::Value;

/// Maximum characters of args/result JSON included verbatim in a stitched
/// system message. Anything longer is truncated with a `… (truncated)` marker.
pub const STITCH_MAX_CHARS: usize = 512;

/// Truncate `s` to at most `max` characters, appending `… (truncated)` if
/// truncation occurred. Operates on chars (not bytes) to stay safe with
/// multi-byte UTF-8.
pub fn truncate_for_message(s: &str, max: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= max {
        return s.to_string();
    }
    let head: String = chars[..max].iter().collect();
    format!("{head} … (truncated)")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn truncate_for_message_passthrough_when_under_limit() {
        let s = "{\"path\":\"/tmp/foo\"}";
        assert_eq!(truncate_for_message(s, STITCH_MAX_CHARS), s);
    }

    #[test]
    fn truncate_for_message_marks_truncation_when_over_limit() {
        let s = "a".repeat(STITCH_MAX_CHARS + 100);
        let out = truncate_for_message(&s, STITCH_MAX_CHARS);
        assert!(out.ends_with("… (truncated)"));
        assert!(out.len() <= STITCH_MAX_CHARS + " … (truncated)".len() + 1);
    }

    #[test]
    fn truncate_for_message_truncation_makes_json_visibly_incomplete() {
        let s = format!("{{\"k\":\"{}\"}}", "x".repeat(STITCH_MAX_CHARS));
        let out = truncate_for_message(&s, STITCH_MAX_CHARS);
        assert!(!out.ends_with("}"), "truncated JSON must not look complete");
        assert!(out.contains("… (truncated)"));
    }
}
