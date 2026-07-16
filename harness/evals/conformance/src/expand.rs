//! Placeholder expansion. Authored fixtures are run-agnostic; the runner
//! stamps run-scoped identity at **Arm** by replacing `{{run_id}}`,
//! `{{session_id}}`, and `{{system_prompt_sha256}}` inside every fixture
//! string. Unknown `{{...}}` tokens are rejected so a typo cannot silently
//! ship an unexpanded literal to the subject.

use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Default)]
pub struct Placeholders {
    values: BTreeMap<&'static str, String>,
}

impl Placeholders {
    pub fn new(run_id: &str, session_id: &str) -> Self {
        let mut values = BTreeMap::new();
        values.insert("run_id", run_id.to_string());
        values.insert("session_id", session_id.to_string());
        Self { values }
    }

    /// Available only after the expected system prompt is rendered at Arm.
    pub fn with_system_prompt_sha256(mut self, digest: &str) -> Self {
        self.values
            .insert("system_prompt_sha256", digest.to_string());
        self
    }

    pub fn expand_str(&self, text: &str) -> anyhow::Result<String> {
        let mut out = text.to_string();
        for (key, value) in &self.values {
            out = out.replace(&format!("{{{{{key}}}}}"), value);
        }
        if let Some(start) = out.find("{{") {
            let tail: String = out[start..].chars().take(40).collect();
            anyhow::bail!("unexpanded placeholder near {tail:?}");
        }
        Ok(out)
    }

    pub fn expand_value(&self, value: &mut Value) -> anyhow::Result<()> {
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
                // Keys may carry placeholders too (e.g. a run-scoped function
                // id used as a map key in evidence expectations).
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
    fn expands_strings_keys_and_rejects_unknown_tokens() {
        let p = Placeholders::new("r1", "s_abc");
        let mut v = json!({
            "idempotency_key": "{{run_id}}:streamed-text",
            "session_id": "{{session_id}}",
            "{{run_id}}::record": { "count": 1 }
        });
        p.expand_value(&mut v).unwrap();
        assert_eq!(v["idempotency_key"], "r1:streamed-text");
        assert_eq!(v["session_id"], "s_abc");
        assert!(v.get("r1::record").is_some());

        let mut bad = json!("{{unknown_token}}");
        assert!(p.expand_value(&mut bad).is_err());
    }
}
