use std::collections::BTreeMap;

use serde_json::Value;

#[derive(Debug, Clone, Default)]
pub struct Placeholders {
    values: BTreeMap<&'static str, String>,
    /// Wall-clock ms captured at construction — the base for
    /// `{{now_plus_<N>ms}}` tokens, so every token in one expansion pass
    /// resolves against the same instant.
    now_ms: i64,
}

const NOW_PLUS_PREFIX: &str = "{{now_plus_";
const NOW_PLUS_SUFFIX: &str = "ms}}";

impl Placeholders {
    pub fn new(run_id: &str, session_id: &str) -> Self {
        let mut values = BTreeMap::new();
        values.insert("run_id", run_id.to_string());
        values.insert("session_id", session_id.to_string());
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or_default();
        Self { values, now_ms }
    }

    pub fn with_system_prompt_sha256(mut self, digest: &str) -> Self {
        self.values
            .insert("system_prompt_sha256", digest.to_string());
        self
    }

    /// `{{now_plus_<N>ms}}` as a whole string → wall-clock ms at expansion
    /// time plus N. This is how a fixture expresses a RUN-RELATIVE deadline
    /// (e.g. a binding's `expires_at`): fixtures are built eagerly at CLI
    /// parse, so a timestamp computed in `scenario()` would be stale by the
    /// time a late-suite scenario runs.
    fn now_plus_token(&self, text: &str) -> Option<i64> {
        let digits = text
            .strip_prefix(NOW_PLUS_PREFIX)?
            .strip_suffix(NOW_PLUS_SUFFIX)?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse::<i64>().ok().map(|n| self.now_ms + n)
    }

    pub fn expand_str(&self, text: &str) -> anyhow::Result<String> {
        let mut out = text.to_string();
        for (key, value) in &self.values {
            out = out.replace(&format!("{{{{{key}}}}}"), value);
        }
        // Embedded now-plus tokens expand to digits; a malformed one is left
        // for the unexpanded-placeholder guard below to name.
        while let Some(start) = out.find(NOW_PLUS_PREFIX) {
            let Some(end) = out[start..].find(NOW_PLUS_SUFFIX) else {
                break;
            };
            let end = start + end + NOW_PLUS_SUFFIX.len();
            let Some(resolved) = self.now_plus_token(&out[start..end]) else {
                break;
            };
            out.replace_range(start..end, &resolved.to_string());
        }
        if let Some(start) = out.find("{{") {
            let tail: String = out[start..].chars().take(40).collect();
            anyhow::bail!("unexpanded placeholder near {tail:?}");
        }
        Ok(out)
    }

    pub fn expand_value(&self, value: &mut Value) -> anyhow::Result<()> {
        // A string that IS a now-plus token becomes a NUMBER — the only way a
        // JSON fixture can place a run-relative timestamp where a schema
        // demands an integer (a lifecycle's `expires_at`).
        let numeric = match value {
            Value::String(s) => self.now_plus_token(s),
            _ => None,
        };
        if let Some(ms) = numeric {
            *value = Value::Number(ms.into());
            return Ok(());
        }
        match value {
            Value::String(s) => {
                *s = self.expand_str(s)?;
            }
            Value::Array(items) => {
                for item in items {
                    self.expand_value(item)?;
                }
            }
            Value::Object(map) => {
                let needs_key_rewrite = map.keys().any(|k| k.contains("{{"));
                if needs_key_rewrite {
                    let old = std::mem::take(map);
                    for (k, mut v) in old {
                        self.expand_value(&mut v)?;
                        map.insert(self.expand_str(&k)?, v);
                    }
                } else {
                    for (_, v) in map.iter_mut() {
                        self.expand_value(v)?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_whole_string_now_plus_token_becomes_a_number() {
        let placeholders = Placeholders::new("run", "session");
        let mut value = json!({ "lifecycle": { "expires_at": "{{now_plus_12000ms}}" } });
        placeholders.expand_value(&mut value).unwrap();
        let resolved = value["lifecycle"]["expires_at"]
            .as_i64()
            .expect("token must resolve to a NUMBER, not a quoted string");
        let now = placeholders.now_ms;
        assert_eq!(resolved, now + 12_000);
    }

    #[test]
    fn an_embedded_token_expands_to_digits_inside_the_string() {
        let placeholders = Placeholders::new("run", "session");
        let out = placeholders
            .expand_str("deadline at {{now_plus_500ms}} for {{run_id}}")
            .unwrap();
        assert!(out.contains("for run"), "got: {out}");
        assert!(!out.contains("{{"), "token must be consumed, got: {out}");
        let digits: String = out.chars().filter(|c| c.is_ascii_digit()).collect();
        assert!(digits.len() >= 13, "epoch-ms scale expected, got: {out}");
    }

    #[test]
    fn malformed_now_plus_tokens_still_trip_the_unexpanded_guard() {
        let placeholders = Placeholders::new("run", "session");
        for bad in [
            "{{now_plus_ms}}",    // no digits
            "{{now_plus_12s}}",   // wrong unit
            "{{now_plus_12000ms", // unterminated
        ] {
            assert!(
                placeholders.expand_str(bad).is_err(),
                "{bad:?} must be rejected, not silently passed through"
            );
            let mut value = Value::String(bad.to_string());
            assert!(placeholders.expand_value(&mut value).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn known_placeholders_still_expand() {
        let placeholders = Placeholders::new("run-1", "session-1").with_system_prompt_sha256("abc");
        let out = placeholders
            .expand_str("{{run_id}}/{{session_id}}/{{system_prompt_sha256}}")
            .unwrap();
        assert_eq!(out, "run-1/session-1/abc");
    }
}
