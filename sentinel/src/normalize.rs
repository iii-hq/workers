//! Message normalization — the step that turns a thousand occurrences into
//! one group.
//!
//! The rules run in a fixed order and the order is load-bearing: masking
//! numbers before hex would eat the first digits of every hash and leave
//! `<n>bc1d…` behind. Identity numbers are protected before anything else
//! runs, because `HTTP 429` and `HTTP 500` are different failures and must
//! never collapse into one group — that protection has to survive the hex
//! rule too, not just the number rule.
//!
//! Changing these rules changes the fingerprint of every existing group, so
//! the fixtures under `tests/fixtures/normalize/` are versioned and
//! [`NORMALIZER_VERSION`] moves with them.

use std::sync::OnceLock;

use regex::{Captures, Regex};

/// Bump with any rule change, and add a matching fixture file. Groups
/// created under an older version keep their raw `message_sample`, which is
/// what a fingerprint migration replays.
pub const NORMALIZER_VERSION: u32 = 1;

/// Upper bound on a normalized message, in characters.
const MAX_CHARS: usize = 200;

/// Placeholder alphabet: deliberately outside `[0-9a-f]` so a protected span
/// cannot be re-matched by the hex rule, and outside quotes and slashes so
/// the string and path rules skip it too.
const ALPHABET: &[u8] = b"ghijklmnopqrstuvwxyz";
const OPEN: char = '\u{1}';
const CLOSE: char = '\u{2}';

pub struct Normalizer {
    /// Identity markers followed by a number, e.g. `HTTP 429`, `code=17`.
    identity: Option<Regex>,
}

impl Normalizer {
    /// Build from the configured identity markers. Invalid or empty markers
    /// are skipped rather than failing: a bad marker must not stop grouping.
    pub fn new(identity_numbers: &[String]) -> Self {
        let markers: Vec<String> = identity_numbers
            .iter()
            .map(|marker| marker.trim())
            .filter(|marker| !marker.is_empty())
            .map(regex::escape)
            .collect();
        let identity = (!markers.is_empty())
            .then(|| {
                Regex::new(&format!(
                    r"(?i)\b(?:{})\b[\s:=#-]*\d+(?:\.\d+)?",
                    markers.join("|")
                ))
                .ok()
            })
            .flatten();
        Self { identity }
    }

    /// The grouping form of a message.
    pub fn normalize(&self, message: &str) -> String {
        let (protected, spans) = self.protect_identity(message);

        let masked = uuid_re().replace_all(&protected, "<id>");
        let masked = ulid_re().replace_all(&masked, "<id>");
        let masked = hex_re().replace_all(&masked, "<hex>");
        let masked = path_re().replace_all(&masked, "<path>");
        let masked = quoted_re().replace_all(&masked, "<str>");
        let masked = number_re().replace_all(&masked, "<n>");

        let restored = restore(&masked, &spans);
        truncate_chars(&collapse_whitespace(&restored), MAX_CHARS)
    }

    /// Replace `HTTP 429` and friends with placeholders so no later rule can
    /// touch the number inside them.
    fn protect_identity<'a>(&self, message: &'a str) -> (std::borrow::Cow<'a, str>, Vec<String>) {
        let Some(identity) = &self.identity else {
            return (std::borrow::Cow::Borrowed(message), Vec::new());
        };
        let mut spans = Vec::new();
        let replaced = identity.replace_all(message, |caps: &Captures| {
            let placeholder = placeholder(spans.len());
            spans.push(caps[0].to_string());
            placeholder
        });
        (replaced, spans)
    }
}

/// `<exception_type>: <message>`, or the function id when there is no type.
/// Truncated to 120 characters, which is what a list row can show.
pub fn title(exception_type: Option<&str>, function_id: Option<&str>, normalized: &str) -> String {
    let prefix = exception_type
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .or_else(|| function_id.map(str::trim).filter(|value| !value.is_empty()));
    let title = match prefix {
        Some(prefix) if normalized.is_empty() => prefix.to_string(),
        Some(prefix) => format!("{prefix}: {normalized}"),
        None => normalized.to_string(),
    };
    truncate_chars(&title, 120)
}

fn placeholder(index: usize) -> String {
    let mut encoded = String::new();
    let mut value = index;
    loop {
        encoded.push(ALPHABET[value % ALPHABET.len()] as char);
        value /= ALPHABET.len();
        if value == 0 {
            break;
        }
    }
    format!("{OPEN}{encoded}{CLOSE}")
}

fn restore(message: &str, spans: &[String]) -> String {
    if spans.is_empty() {
        return message.to_string();
    }
    let mut restored = message.to_string();
    for (index, original) in spans.iter().enumerate() {
        restored = restored.replace(&placeholder(index), original);
    }
    restored
}

fn collapse_whitespace(message: &str) -> String {
    message.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn truncate_chars(value: &str, max: usize) -> String {
    match value.char_indices().nth(max) {
        Some((offset, _)) => value[..offset].to_string(),
        None => value.to_string(),
    }
}

fn uuid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b")
            .expect("uuid pattern compiles")
    })
}

fn ulid_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Crockford base32, 26 characters, no I/L/O/U.
    RE.get_or_init(|| {
        Regex::new(r"\b[0-7][0-9ABCDEFGHJKMNPQRSTVWXYZ]{25}\b").expect("ulid pattern compiles")
    })
}

fn hex_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\b[0-9a-f]{8,}\b").expect("hex pattern compiles"))
}

fn path_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?:[A-Za-z]:\\[^\s\x01\x02]*|/[^\s\x01\x02:]*(?:/[^\s\x01\x02:]*)+)")
            .expect("path pattern compiles")
    })
}

fn quoted_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"'[^']*'|"[^"]*""#).expect("quoted pattern compiles"))
}

fn number_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\d+(?:\.\d+)?").expect("number pattern compiles"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::FingerprintConfigV1;

    fn normalizer() -> Normalizer {
        Normalizer::new(&FingerprintConfigV1::default().identity_numbers)
    }

    #[test]
    fn http_statuses_stay_apart_while_ordinary_numbers_collapse() {
        let n = normalizer();
        assert_eq!(
            n.normalize("request failed: HTTP 429"),
            "request failed: HTTP 429"
        );
        assert_ne!(
            n.normalize("request failed: HTTP 429"),
            n.normalize("request failed: HTTP 500")
        );
        assert_eq!(
            n.normalize("retried 3 times"),
            n.normalize("retried 17 times")
        );
    }

    #[test]
    fn identity_numbers_survive_the_hex_rule_too() {
        let n = normalizer();
        // Eight digits would otherwise read as a hex blob and be masked
        // before the number rule ever sees them.
        assert_eq!(n.normalize("status 12345678"), "status 12345678");
        assert_eq!(n.normalize("blob 12345678"), "blob <hex>");
    }

    #[test]
    fn identifiers_hashes_paths_and_quoted_values_are_masked() {
        let n = normalizer();
        assert_eq!(
            n.normalize("session 0191f0c4-7b2a-7c3d-9e8f-1a2b3c4d5e6f lost"),
            "session <id> lost"
        );
        assert_eq!(
            n.normalize("ulid 01J8M2QK9ZC3TSVX4YB7HD5NEA seen"),
            "ulid <id> seen"
        );
        assert_eq!(
            n.normalize("commit 4662b0dfeed1 missing"),
            "commit <hex> missing"
        );
        assert_eq!(
            n.normalize("could not read /home/me/workspaces/workers/x.rs"),
            "could not read <path>"
        );
        assert_eq!(
            n.normalize(r"could not read C:\Users\me\x.rs"),
            "could not read <path>"
        );
        assert_eq!(
            n.normalize("page \"kanban\" is not registered"),
            "page <str> is not registered"
        );
        assert_eq!(
            n.normalize("page 'kanban' is not registered"),
            n.normalize("page 'security-scan' is not registered"),
            "a quoted value is data, not identity — the documented trade-off"
        );
    }

    #[test]
    fn whitespace_collapses_and_the_result_is_bounded() {
        let n = normalizer();
        assert_eq!(n.normalize("  too   many \n spaces "), "too many spaces");
        let long = "x".repeat(500);
        assert_eq!(n.normalize(&long).chars().count(), MAX_CHARS);
    }

    #[test]
    fn a_truncation_boundary_never_splits_a_character() {
        let n = normalizer();
        let wide = "ç".repeat(400);
        let normalized = n.normalize(&wide);
        assert_eq!(normalized.chars().count(), MAX_CHARS);
        assert!(normalized.chars().all(|c| c == 'ç'));
    }

    #[test]
    fn many_identity_numbers_in_one_message_all_survive() {
        let n = normalizer();
        // Enough markers to push the placeholder encoding past one letter,
        // and short enough to stay inside the 200-character bound.
        let message = (0..25)
            .map(|index| format!("code {index}"))
            .collect::<Vec<_>>()
            .join(" ");
        let normalized = n.normalize(&message);
        assert!(normalized.contains("code 20"), "{normalized}");
        assert!(normalized.contains("code 24"), "{normalized}");
        assert!(
            !normalized.contains('\u{1}'),
            "placeholders leaked: {normalized}"
        );
    }

    #[test]
    fn without_markers_every_number_is_masked() {
        let n = Normalizer::new(&[]);
        assert_eq!(n.normalize("HTTP 429"), "HTTP <n>");
    }

    #[test]
    fn titles_prefer_the_exception_type_and_fall_back_to_the_function() {
        assert_eq!(
            title(
                Some("CasMismatch"),
                Some("state::compare-and-set"),
                "expected <n>"
            ),
            "CasMismatch: expected <n>"
        );
        assert_eq!(
            title(None, Some("state::compare-and-set"), "expected <n>"),
            "state::compare-and-set: expected <n>"
        );
        assert_eq!(title(None, None, "expected <n>"), "expected <n>");
        assert_eq!(title(Some("CasMismatch"), None, ""), "CasMismatch");
        assert_eq!(
            title(Some("E"), None, &"x".repeat(200)).chars().count(),
            120
        );
    }
}
