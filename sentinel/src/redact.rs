//! Value redaction, applied on capture.
//!
//! Dropping attributes whose *key* looks sensitive does not protect what
//! lives inside `exception.message`, a `status_description`, a log body or a
//! `db.statement`: a connection string in a failure message would cross to a
//! model provider intact. So every text value the worker captures passes
//! through here **before the first insert** — the raw value is never stored,
//! and the fingerprint is computed from the redacted form, which has the
//! useful side effect that a per-request token stops splitting one failure
//! into a thousand groups.
//!
//! This is a denylist of shapes, and a denylist is never complete: a secret
//! in an unknown format passes. `status.ingest.redactions` shows the redactor
//! working, `redaction.patterns` takes the shapes a particular deployment
//! knows about, and the investigation's system prompt still forbids
//! reproducing credentials — three layers, none of them a guarantee.

use regex::Regex;
use serde_json::Value;

/// Attribute-key fragments whose value is dropped outright. Shared with the
/// evidence filter so both halves agree on what "sensitive" means.
pub const SENSITIVE_KEY_FRAGMENTS: [&str; 8] = [
    "authorization",
    "token",
    "secret",
    "api_key",
    "apikey",
    "password",
    "passwd",
    "private_key",
];

#[derive(Debug)]
struct Rule {
    pattern: Regex,
    replacement: String,
}

#[derive(Debug)]
pub struct Redactor {
    rules: Vec<Rule>,
}

impl Default for Redactor {
    fn default() -> Self {
        Self::new(&[]).expect("the built-in patterns compile")
    }
}

impl Redactor {
    /// Built-in shapes plus the operator's own. An invalid extra pattern is
    /// an error the caller reports; the built-ins always run.
    pub fn new(extra_patterns: &[String]) -> Result<Self, String> {
        let mut rules = Vec::new();
        for (kind, pattern, replacement) in builtin_rules() {
            rules.push(Rule {
                pattern: Regex::new(pattern)
                    .map_err(|error| format!("built-in pattern {kind} is invalid: {error}"))?,
                replacement: replacement.to_string(),
            });
        }
        for (index, pattern) in extra_patterns.iter().enumerate() {
            rules.push(Rule {
                pattern: Regex::new(pattern)
                    .map_err(|error| format!("redaction.patterns[{index}] is invalid: {error}"))?,
                replacement: format!("[redacted:custom:{index}]"),
            });
        }
        Ok(Self { rules })
    }

    /// Redact one string, reporting how many values were rewritten.
    pub fn redact(&self, value: &str) -> (String, u64) {
        let mut current = value.to_string();
        let mut hits = 0;
        for rule in &self.rules {
            let found = rule.pattern.find_iter(&current).count() as u64;
            if found == 0 {
                continue;
            }
            hits += found;
            current = rule
                .pattern
                .replace_all(&current, rule.replacement.as_str())
                .into_owned();
        }
        (current, hits)
    }

    /// Redact in place, for the common case of a field being overwritten.
    pub fn redact_in_place(&self, value: &mut String) -> u64 {
        let (redacted, hits) = self.redact(value);
        if hits > 0 {
            *value = redacted;
        }
        hits
    }

    /// Redact every string in a JSON document — object keys are left alone,
    /// values are rewritten. This is what the read-only trace and log proxies
    /// apply to an engine response before the agent sees it.
    pub fn redact_json(&self, value: &mut Value) -> u64 {
        match value {
            Value::String(text) => self.redact_in_place(text),
            Value::Array(items) => items.iter_mut().map(|item| self.redact_json(item)).sum(),
            Value::Object(fields) => fields
                .iter_mut()
                .map(|(_, field)| self.redact_json(field))
                .sum(),
            _ => 0,
        }
    }

    /// Whether an attribute key is sensitive enough that its value is dropped
    /// rather than redacted.
    pub fn is_sensitive_key(key: &str) -> bool {
        let lowered = key.to_ascii_lowercase();
        SENSITIVE_KEY_FRAGMENTS
            .iter()
            .any(|fragment| lowered.contains(fragment))
    }
}

/// Order matters: the widest, most structural shapes first, so a PEM block or
/// a credentialed URL is taken whole instead of being nibbled by the
/// key/value rule.
fn builtin_rules() -> Vec<(&'static str, &'static str, &'static str)> {
    vec![
        (
            "pem",
            r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----",
            "[redacted:pem]",
        ),
        (
            "url_credentials",
            r"(?i)\b([a-z][a-z0-9+.\-]*://)[^:/@\s]+:[^@\s]+@",
            "${1}[redacted:url_credentials]@",
        ),
        (
            "bearer",
            r"(?i)\bbearer\s+[A-Za-z0-9\-._~+/]+=*",
            "[redacted:bearer]",
        ),
        (
            "jwt",
            r"\beyJ[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\.[A-Za-z0-9_-]{4,}\b",
            "[redacted:jwt]",
        ),
        (
            "api_key",
            r"\b(?:sk-[A-Za-z0-9_\-]{8,}|gh[pousr]_[A-Za-z0-9]{16,}|xox[abprs]-[A-Za-z0-9\-]{10,}|AKIA[0-9A-Z]{16})\b",
            "[redacted:api_key]",
        ),
        (
            "key_value",
            r#"(?i)"?\b(?:authorization|api[_-]?key|access[_-]?key|secret[_-]?key|client[_-]?secret|private[_-]?key|password|passwd|pwd|token)\b"?\s*[:=]\s*"?[^\s,;"'&\[]+"?"#,
            "[redacted:key_value]",
        ),
        (
            "email",
            r"(?i)\b[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}\b",
            "[redacted:email]",
        ),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_bearer_token_never_survives_capture() {
        let (redacted, hits) = Redactor::default()
            .redact("upstream said 401 for Authorization: Bearer abc.def-ghi_jkl=");
        assert!(!redacted.contains("abc.def"), "{redacted}");
        assert_eq!(
            redacted, "upstream said 401 for Authorization: [redacted:bearer]",
            "the narrower rule wins and the wider one does not redact it again"
        );
        assert_eq!(hits, 1);
    }

    #[test]
    fn a_connection_string_keeps_its_shape_and_loses_its_credentials() {
        let (redacted, hits) =
            Redactor::default().redact("could not connect to postgres://admin:hunter2@db:5432/app");
        assert_eq!(
            redacted,
            "could not connect to postgres://[redacted:url_credentials]@db:5432/app"
        );
        assert_eq!(hits, 1);
    }

    #[test]
    fn prefixed_keys_jwts_and_pem_blocks_are_taken_whole() {
        let redactor = Redactor::default();
        for secret in [
            "sk-abcdefghijklmnop",
            "ghp_0123456789abcdefghij",
            "xoxb-1234567890-abcdef",
            "AKIAIOSFODNN7EXAMPLE",
            "eyJhbGciOi.eyJzdWIiOi.SflKxwRJSM",
        ] {
            let (redacted, hits) = redactor.redact(&format!("leaked {secret} here"));
            assert_eq!(hits, 1, "{secret} was not redacted: {redacted}");
            assert!(!redacted.contains(secret), "{redacted}");
        }

        let pem = "-----BEGIN RSA PRIVATE KEY-----\nMIIEow==\n-----END RSA PRIVATE KEY-----";
        let (redacted, hits) = redactor.redact(&format!("key: {pem}"));
        assert_eq!(hits, 1);
        assert!(!redacted.contains("MIIEow"), "{redacted}");
    }

    #[test]
    fn a_sensitive_key_value_pair_is_redacted_in_prose_and_in_json() {
        let redactor = Redactor::default();
        let (redacted, _) = redactor.redact("failed with api_key=abcd1234 and password: hunter2");
        assert!(!redacted.contains("abcd1234"), "{redacted}");
        assert!(!redacted.contains("hunter2"), "{redacted}");

        let (redacted, _) = redactor.redact(r#"{"client_secret": "shhh"}"#);
        assert!(!redacted.contains("shhh"), "{redacted}");
    }

    #[test]
    fn json_redaction_walks_values_and_leaves_keys_alone() {
        let redactor = Redactor::default();
        let mut document = json!({
            "spans": [{
                "attributes": { "db.statement": "connect postgres://u:p@h/db" },
                "events": [{ "exception.message": "Bearer abc.def-ghi" }],
            }],
            "count": 3,
        });
        let hits = redactor.redact_json(&mut document);
        assert_eq!(hits, 2);
        let rendered = document.to_string();
        assert!(!rendered.contains("u:p@h"), "{rendered}");
        assert!(rendered.contains("db.statement"), "keys survive");
        assert_eq!(document["count"], 3, "non-strings are untouched");
    }

    #[test]
    fn an_ordinary_message_is_returned_unchanged_and_uncounted() {
        let (redacted, hits) =
            Redactor::default().redact("expected version 41, found 42 for session s_7a1");
        assert_eq!(redacted, "expected version 41, found 42 for session s_7a1");
        assert_eq!(hits, 0);
    }

    #[test]
    fn an_operator_pattern_runs_after_the_built_ins_and_is_named_for_its_slot() {
        let redactor = Redactor::new(&[r"CUST-\d{4}".to_string()]).expect("valid pattern");
        let (redacted, hits) = redactor.redact("order CUST-9182 failed");
        assert_eq!(redacted, "order [redacted:custom:0] failed");
        assert_eq!(hits, 1);
    }

    #[test]
    fn an_invalid_operator_pattern_names_its_slot() {
        let error = Redactor::new(&["([unclosed".to_string()]).expect_err("invalid pattern");
        assert!(error.contains("redaction.patterns[0]"), "{error}");
    }

    #[test]
    fn sensitive_keys_are_recognised_case_insensitively() {
        assert!(Redactor::is_sensitive_key("Authorization"));
        assert!(Redactor::is_sensitive_key("http.request.header.api_key"));
        assert!(!Redactor::is_sensitive_key("function_id"));
    }
}
