//! Canonical JSON: object keys sorted recursively, no insignificant
//! whitespace, serde_json's shortest-round-trip number formatting. Reports
//! and digests are byte-deterministic because every JSON artifact funnels
//! through here.

use serde_json::Value;
use sha2::{Digest, Sha256};

/// A copy of `value` with all object keys sorted recursively.
pub fn sort_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted: Vec<(&String, &Value)> = map.iter().collect();
            sorted.sort_by_key(|(k, _)| k.as_str());
            Value::Object(
                sorted
                    .into_iter()
                    .map(|(k, v)| (k.clone(), sort_keys(v)))
                    .collect(),
            )
        }
        Value::Array(items) => Value::Array(items.iter().map(sort_keys).collect()),
        other => other.clone(),
    }
}

/// Compact canonical JSON text.
pub fn canonical_json(value: &Value) -> String {
    serde_json::to_string(&sort_keys(value)).expect("canonical serialization cannot fail")
}

/// Pretty canonical JSON with a trailing newline — the artifact file format.
pub fn canonical_json_pretty(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(&sort_keys(value))
        .expect("canonical serialization cannot fail");
    text.push('\n');
    text
}

/// Lowercase-hex SHA-256 of the canonical JSON of `value`.
pub fn sha256_of_canonical(value: &Value) -> String {
    hex(&Sha256::digest(canonical_json(value).as_bytes()))
}

/// Lowercase-hex SHA-256 of raw bytes (e.g. the UTF-8 of a matched string).
pub fn sha256_of_bytes(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(digest: &[u8]) -> String {
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn key_order_does_not_change_canonical_bytes() {
        let a: Value = serde_json::from_str(r#"{"b":1,"a":{"d":2,"c":[{"y":1,"x":2}]}}"#).unwrap();
        let b: Value = serde_json::from_str(r#"{"a":{"c":[{"x":2,"y":1}],"d":2},"b":1}"#).unwrap();
        assert_eq!(canonical_json(&a), canonical_json(&b));
        assert_eq!(sha256_of_canonical(&a), sha256_of_canonical(&b));
    }

    #[test]
    fn array_order_is_preserved() {
        assert_ne!(
            canonical_json(&json!([1, 2])),
            canonical_json(&json!([2, 1]))
        );
    }
}
