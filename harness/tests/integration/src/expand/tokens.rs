use std::collections::BTreeMap;

use serde_json::Value;

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
