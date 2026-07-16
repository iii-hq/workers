//! `RouterCassetteV1` (spec § Proposed cassette contract): a sanitized
//! `RouterScriptV1` plus capture provenance. Slice 1 ships the schema, the
//! digest rule, and the sanitizer denylist scan — capture itself is manual
//! and non-gating and has no code path here yet.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::script::{RouterScriptV1, SchemaVersion1};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouterCassetteV1 {
    pub schema_version: SchemaVersion1,
    pub captured_at: String,
    pub engine_revision: String,
    pub harness_revision: String,
    pub router_revision: String,
    pub provider: String,
    pub model: String,
    pub script: RouterScriptV1,
    /// SHA-256 over canonical JSON of the sanitized object with this field
    /// omitted.
    pub sanitized_sha256: String,
}

impl RouterCassetteV1 {
    /// The digest input: this cassette as canonical JSON with
    /// `sanitized_sha256` removed.
    pub fn digest(&self) -> anyhow::Result<String> {
        let mut value = serde_json::to_value(self)?;
        value
            .as_object_mut()
            .expect("cassette serializes to an object")
            .remove("sanitized_sha256");
        Ok(crate::canonical::sha256_of_canonical(&value))
    }

    pub fn verify_digest(&self) -> anyhow::Result<()> {
        let computed = self.digest()?;
        if computed != self.sanitized_sha256 {
            anyhow::bail!(
                "cassette digest mismatch: recorded {} computed {computed}",
                self.sanitized_sha256
            );
        }
        Ok(())
    }
}

/// Sanitization denylist (spec: authorization headers, credentials, cookies,
/// personal data, nondeterministic IDs, provider-private metadata). The scan
/// walks every object key and string value; a hit rejects the cassette.
pub fn denylist_scan(value: &serde_json::Value) -> Vec<String> {
    const DENY_KEY_FRAGMENTS: [&str; 12] = [
        "authorization",
        "api_key",
        "apikey",
        "api-key",
        "secret",
        "token",
        "cookie",
        "password",
        "credential",
        "set-cookie",
        "session_key",
        "private",
    ];
    // String values that look like live credentials.
    const DENY_VALUE_PREFIXES: [&str; 4] = ["sk-", "Bearer ", "bearer ", "ghp_"];
    let mut hits = Vec::new();
    walk(value, "", &mut |path, v| {
        if let Some(key) = path.rsplit('/').next() {
            let lower = key.to_ascii_lowercase();
            if DENY_KEY_FRAGMENTS.iter().any(|f| lower.contains(f)) {
                hits.push(format!("denylisted key at {path}"));
            }
        }
        if let serde_json::Value::String(s) = v {
            if DENY_VALUE_PREFIXES.iter().any(|p| s.starts_with(p)) {
                hits.push(format!("credential-shaped value at {path}"));
            }
        }
    });
    hits
}

fn walk(value: &serde_json::Value, path: &str, f: &mut impl FnMut(&str, &serde_json::Value)) {
    f(path, value);
    match value {
        serde_json::Value::Object(map) => {
            for (k, v) in map {
                walk(v, &format!("{path}/{k}"), f);
            }
        }
        serde_json::Value::Array(items) => {
            for (i, v) in items.iter().enumerate() {
                walk(v, &format!("{path}/{i}"), f);
            }
        }
        _ => {}
    }
}
