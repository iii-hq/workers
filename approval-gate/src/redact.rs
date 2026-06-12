//! Recursive argument redaction for pending records and denial envelopes.
//! Pure and immutable — never mutates its input. Ported behavior-for-
//! behavior from the proven implementation
//! (harness/src/approval-gate/redact.ts). This is what makes pending
//! records safe to forward to notification channels.

use serde_json::Value;

pub const ARGS_EXCERPT_LEN_CAP: usize = 256;

/// Hard ceiling on recursion depth. Args are caller-influenced, so an
/// adversarial payload (deeply nested) must not overflow the stack —
/// past this depth the subtree collapses to a sentinel.
pub const MAX_REDACT_DEPTH: usize = 64;

const MAX_DEPTH_SENTINEL: &str = "<max-depth>";
const REDACTED: &str = "<redacted>";

const REDACT_KEYS: [&str; 11] = [
    "password",
    "token",
    "api_key",
    "apikey",
    "secret",
    "auth",
    "authorization",
    "access_key",
    "access_token",
    "refresh_token",
    "private_key",
];

fn is_secret_key(key: &str) -> bool {
    let lower = key.to_lowercase();
    if REDACT_KEYS.contains(&lower.as_str()) {
        return true;
    }
    REDACT_KEYS
        .iter()
        .any(|suffix| lower.ends_with(&format!("_{suffix}")))
}

/// Truncate by Unicode code point (`char`) so the cap is measured the
/// same way the prior art measured it — byte-length clipping would
/// falsely truncate multi-byte-heavy strings that are within the cap.
pub fn clip(s: &str) -> String {
    let mut chars = s.chars();
    let clipped: String = chars.by_ref().take(ARGS_EXCERPT_LEN_CAP).collect();
    if chars.next().is_none() {
        s.to_string()
    } else {
        format!("{clipped}…")
    }
}

/// Walk the value tree, redacting secret-keyed values and clipping long
/// strings. Returns a brand-new tree.
pub fn redact(value: &Value) -> Value {
    redact_at(value, 0)
}

fn redact_at(value: &Value, depth: usize) -> Value {
    match value {
        Value::String(s) => Value::String(clip(s)),
        Value::Array(_) | Value::Object(_) if depth >= MAX_REDACT_DEPTH => {
            Value::String(MAX_DEPTH_SENTINEL.to_string())
        }
        Value::Array(items) => {
            Value::Array(items.iter().map(|v| redact_at(v, depth + 1)).collect())
        }
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| {
                    let redacted = if is_secret_key(k) {
                        Value::String(REDACTED.to_string())
                    } else {
                        redact_at(v, depth + 1)
                    };
                    (k.clone(), redacted)
                })
                .collect(),
        ),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn redacts_secret_keys_case_insensitively() {
        let out = redact(&json!({ "Password": "hunter2", "cmd": "ls" }));
        assert_eq!(out, json!({ "Password": "<redacted>", "cmd": "ls" }));
    }

    #[test]
    fn redacts_suffix_matched_keys() {
        let out = redact(&json!({ "github_token": "ghp_x", "stripe_api_key": "sk_x" }));
        assert_eq!(
            out,
            json!({ "github_token": "<redacted>", "stripe_api_key": "<redacted>" })
        );
    }

    #[test]
    fn redacts_non_string_secret_values_too() {
        let out = redact(&json!({ "secret": { "nested": "v" }, "token": 42 }));
        assert_eq!(
            out,
            json!({ "secret": "<redacted>", "token": "<redacted>" })
        );
    }

    #[test]
    fn clips_long_strings_with_ellipsis() {
        let long = "a".repeat(300);
        let out = redact(&json!({ "text": long }));
        let clipped = out["text"].as_str().unwrap();
        assert_eq!(clipped.chars().count(), ARGS_EXCERPT_LEN_CAP + 1);
        assert!(clipped.ends_with('…'));
    }

    #[test]
    fn clip_counts_code_points_not_bytes() {
        // 200 astral chars = 800 bytes but only 200 code points: no clip.
        let astral = "𝄞".repeat(200);
        assert_eq!(clip(&astral), astral);
        let over = "𝄞".repeat(257);
        let clipped = clip(&over);
        assert_eq!(clipped.chars().count(), ARGS_EXCERPT_LEN_CAP + 1);
    }

    #[test]
    fn recurses_through_arrays() {
        let out = redact(&json!([{ "password": "x" }, "ok"]));
        assert_eq!(out, json!([{ "password": "<redacted>" }, "ok"]));
    }

    #[test]
    fn passes_scalars_through() {
        assert_eq!(redact(&json!(42)), json!(42));
        assert_eq!(redact(&json!(true)), json!(true));
        assert_eq!(redact(&Value::Null), Value::Null);
    }

    #[test]
    fn collapses_past_max_depth() {
        let mut v = json!("leaf");
        for _ in 0..(MAX_REDACT_DEPTH + 5) {
            v = json!([v]);
        }
        let mut out = redact(&v);
        let mut depth = 0usize;
        while let Value::Array(items) = out {
            out = items.into_iter().next().unwrap();
            depth += 1;
        }
        assert_eq!(out, json!("<max-depth>"));
        assert_eq!(depth, MAX_REDACT_DEPTH);
    }

    #[test]
    fn never_mutates_input() {
        let input = json!({ "password": "x", "nested": { "api_key": "y" } });
        let snapshot = input.clone();
        let _ = redact(&input);
        assert_eq!(input, snapshot);
    }
}
