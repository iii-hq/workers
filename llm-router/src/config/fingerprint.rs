//! Stable per-provider config-slice hashing for the paste-a-key diff.
use std::collections::BTreeMap;

use serde_json::Value;
use sha2::{Digest, Sha256};

fn stable_string(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let entries: Vec<String> = map
                .iter()
                .collect::<BTreeMap<_, _>>()
                .into_iter()
                .map(|(k, v)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(k).unwrap_or_default(),
                        stable_string(v)
                    )
                })
                .collect();
            format!("{{{}}}", entries.join(","))
        }
        Value::Array(items) => {
            format!(
                "[{}]",
                items
                    .iter()
                    .map(stable_string)
                    .collect::<Vec<_>>()
                    .join(",")
            )
        }
        other => other.to_string(),
    }
}

/// `providers` object → provider id → sha256 of its slice.
pub fn fingerprint_slices(providers: &Value) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Value::Object(map) = providers {
        for (id, slice) in map {
            let mut hasher = Sha256::new();
            hasher.update(stable_string(slice));
            out.insert(id.clone(), format!("{:x}", hasher.finalize()));
        }
    }
    out
}

/// Provider ids whose slice was added, changed, or removed.
pub fn changed_slices(
    prev: &BTreeMap<String, String>,
    next: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut ids: Vec<String> = prev.keys().chain(next.keys()).cloned().collect();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .filter(|id| prev.get(id) != next.get(id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn stable_under_key_order_and_diff_reports_added_changed_removed() {
        let a = fingerprint_slices(&json!({ "anthropic": { "api_key": "k", "max_tokens": 1 } }));
        let b = fingerprint_slices(&json!({ "anthropic": { "max_tokens": 1, "api_key": "k" } }));
        assert_eq!(a, b);
        let prev =
            fingerprint_slices(&json!({ "a": { "k": 1 }, "b": { "k": 2 }, "c": { "k": 3 } }));
        let next =
            fingerprint_slices(&json!({ "a": { "k": 1 }, "b": { "k": 99 }, "d": { "k": 4 } }));
        let mut changed = changed_slices(&prev, &next);
        changed.sort();
        assert_eq!(changed, vec!["b", "c", "d"]);
    }
}
