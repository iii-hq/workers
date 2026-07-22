//! `JsonMatcherV1` evaluation.
//!
//! Matchers see copies: normalizers mutate the copies of both actual and
//! expected before comparison and never the stored evidence. `subset`
//! requires every expected object member or array element at the same
//! position — array order matters and elements cannot be skipped.

use serde_json::Value;

use crate::canonical::{canonical_json, sha256_of_bytes};
use crate::types::script::{JsonMatcherV1, JsonNormalizerV1, NormalizerOperation};

/// One field's verdict, stored verbatim in router-call evidence.
#[derive(Debug, Clone, serde::Serialize)]
pub struct FieldMatchResult {
    pub field: String,
    pub passed: bool,
    /// Human-readable mismatch description; empty on pass.
    pub detail: String,
}

/// Evaluate a matcher against a field that may be absent from the request.
pub fn evaluate(field: &str, matcher: &JsonMatcherV1, actual: Option<&Value>) -> FieldMatchResult {
    let fail = |detail: String| FieldMatchResult {
        field: field.to_string(),
        passed: false,
        detail,
    };
    let pass = FieldMatchResult {
        field: field.to_string(),
        passed: true,
        detail: String::new(),
    };
    match matcher {
        JsonMatcherV1::Absent => match actual {
            // JSON null is a present value, not an absent field: the harness
            // omits unset optionals entirely (skip_serializing_if), so a null
            // here is a contract deviation worth failing.
            None => pass,
            Some(v) => fail(format!("expected absent, got {}", head(v))),
        },
        JsonMatcherV1::Regex { pattern } => {
            let Some(actual) = actual else {
                return fail("regex matcher: field is absent".into());
            };
            let Some(text) = actual.as_str() else {
                return fail(format!(
                    "regex matcher: value is not a string: {}",
                    head(actual)
                ));
            };
            match regex::Regex::new(pattern) {
                Ok(re) if re.is_match(text) => pass,
                Ok(_) => fail(format!("regex {pattern:?} did not match {text:?}")),
                Err(e) => fail(format!("invalid regex {pattern:?}: {e}")),
            }
        }
        JsonMatcherV1::Sha256 { expected } => {
            let Some(actual) = actual else {
                return fail("sha256 matcher: field is absent".into());
            };
            let Some(text) = actual.as_str() else {
                return fail(format!(
                    "sha256 matcher: value is not a string: {}",
                    head(actual)
                ));
            };
            let computed = sha256_of_bytes(text.as_bytes());
            if computed == expected.to_ascii_lowercase() {
                pass
            } else {
                fail(format!(
                    "sha256 mismatch: expected {expected}, computed {computed}"
                ))
            }
        }
        JsonMatcherV1::Exact {
            expected,
            normalize,
        } => {
            let Some(actual) = actual else {
                return fail("exact matcher: field is absent".into());
            };
            let (actual, expected) = match normalized_pair(actual, expected, normalize.as_deref()) {
                Ok(pair) => pair,
                Err(e) => return fail(format!("normalizer failed: {e}")),
            };
            if actual == expected {
                pass
            } else {
                fail(format!(
                    "exact mismatch: expected {}, got {}",
                    head(&expected),
                    head(&actual)
                ))
            }
        }
        JsonMatcherV1::Subset {
            expected,
            normalize,
        } => {
            let Some(actual) = actual else {
                return fail("subset matcher: field is absent".into());
            };
            let (actual, expected) = match normalized_pair(actual, expected, normalize.as_deref()) {
                Ok(pair) => pair,
                Err(e) => return fail(format!("normalizer failed: {e}")),
            };
            match subset_mismatch(&expected, &actual, "") {
                None => pass,
                Some(detail) => fail(detail),
            }
        }
    }
}

fn normalized_pair(
    actual: &Value,
    expected: &Value,
    normalizers: Option<&[JsonNormalizerV1]>,
) -> anyhow::Result<(Value, Value)> {
    let mut actual = actual.clone();
    let mut expected = expected.clone();
    for n in normalizers.unwrap_or_default() {
        apply_normalizer(&mut actual, n)?;
        apply_normalizer(&mut expected, n)?;
    }
    Ok((actual, expected))
}

/// Apply one normalizer to one document. A pointer whose path does not exist
/// is a no-op: deleting a `timestamp` that a side already lacks must not fail
/// the comparison.
fn apply_normalizer(doc: &mut Value, normalizer: &JsonNormalizerV1) -> anyhow::Result<()> {
    validate_pointer(&normalizer.pointer)?;
    match normalizer.operation {
        NormalizerOperation::Delete => {
            if normalizer.pointer.is_empty() {
                anyhow::bail!("delete normalizer cannot target the document root");
            }
            let (parent_ptr, token) = split_pointer(&normalizer.pointer);
            let Some(parent) = doc.pointer_mut(parent_ptr) else {
                return Ok(());
            };
            match parent {
                Value::Object(map) => {
                    map.remove(&token);
                }
                Value::Array(items) => {
                    if let Ok(index) = token.parse::<usize>() {
                        if index < items.len() {
                            items.remove(index);
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        }
    }
}

/// Structural validation of an RFC 6901 pointer (used at script load time).
pub fn validate_pointer(pointer: &str) -> anyhow::Result<()> {
    if pointer.is_empty() {
        return Ok(());
    }
    if !pointer.starts_with('/') {
        anyhow::bail!("JSON pointer must be empty or start with '/': {pointer:?}");
    }
    for token in pointer[1..].split('/') {
        let mut chars = token.chars().peekable();
        while let Some(c) = chars.next() {
            if c == '~' && !matches!(chars.peek(), Some('0') | Some('1')) {
                anyhow::bail!("invalid ~ escape in JSON pointer token {token:?}");
            }
        }
    }
    Ok(())
}

/// Split a non-empty pointer into (parent pointer, unescaped last token).
fn split_pointer(pointer: &str) -> (&str, String) {
    let idx = pointer.rfind('/').expect("validated non-empty pointer");
    let token = pointer[idx + 1..].replace("~1", "/").replace("~0", "~");
    (&pointer[..idx], token)
}

/// `None` when `expected` is a positional subset of `actual`; otherwise a
/// description of the first mismatch.
fn subset_mismatch(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    subset_mismatch_with_arrays(expected, actual, path)
}

fn subset_mismatch_with_arrays(expected: &Value, actual: &Value, path: &str) -> Option<String> {
    match (expected, actual) {
        (Value::Object(exp), Value::Object(act)) => {
            for (k, ev) in exp {
                match act.get(k) {
                    Some(av) => {
                        if let Some(detail) =
                            subset_mismatch_with_arrays(ev, av, &format!("{path}/{k}"))
                        {
                            return Some(detail);
                        }
                    }
                    None => return Some(format!("subset: missing member at {path}/{k}")),
                }
            }
            None
        }
        (Value::Array(exp), Value::Array(act)) => {
            if exp.len() > act.len() {
                return Some(format!(
                    "subset: expected {} array elements at {path}, got {}",
                    exp.len(),
                    act.len()
                ));
            }
            for (i, ev) in exp.iter().enumerate() {
                if let Some(detail) =
                    subset_mismatch_with_arrays(ev, &act[i], &format!("{path}/{i}"))
                {
                    return Some(detail);
                }
            }
            None
        }
        (e, a) if e == a => None,
        (e, a) => Some(format!(
            "subset: expected {} at {path}, got {}",
            head(e),
            head(a)
        )),
    }
}

/// Truncated canonical rendering for mismatch messages (evidence files carry
/// the full values; this keeps verdict lines readable). The cut must land on
/// a char boundary: a byte slice through a multibyte character would panic
/// exactly while reporting a mismatch, with the router mutex held.
fn head(value: &Value) -> String {
    let text = canonical_json(value);
    if text.len() <= 200 {
        return text;
    }
    let mut cut = 200;
    while !text.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}… ({} bytes)", &text[..cut], text.len())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::script::{JsonNormalizerV1, NormalizerOperation};
    use serde_json::json;

    fn exact(expected: Value, normalize: Option<Vec<JsonNormalizerV1>>) -> JsonMatcherV1 {
        JsonMatcherV1::Exact {
            expected,
            normalize,
        }
    }

    #[test]
    fn absent_distinguishes_null_from_missing() {
        assert!(evaluate("f", &JsonMatcherV1::Absent, None).passed);
        assert!(!evaluate("f", &JsonMatcherV1::Absent, Some(&json!(null))).passed);
    }

    /// The truncation cut point lands mid-emoji: 199 leading bytes (quote +
    /// 198 ASCII) put byte 200 inside the 4-byte 😀. A byte slice here used
    /// to panic instead of reporting the mismatch.
    #[test]
    fn mismatch_truncation_respects_multibyte_boundaries() {
        let long = format!("{}😀tail", "a".repeat(198));
        let result = evaluate("f", &exact(json!("expected"), None), Some(&json!(long)));
        assert!(!result.passed);
        assert!(result.detail.contains("bytes)"), "{}", result.detail);
    }

    #[test]
    fn regex_only_matches_strings() {
        let m = JsonMatcherV1::Regex {
            pattern: "^t_[0-9a-f]{32}:[0-9]+$".into(),
        };
        let id = json!("t_0123456789abcdef0123456789abcdef:1");
        assert!(evaluate("request_id", &m, Some(&id)).passed);
        assert!(!evaluate("request_id", &m, Some(&json!(42))).passed);
        assert!(!evaluate("request_id", &m, Some(&json!("t_short:1"))).passed);
    }

    #[test]
    fn sha256_hashes_string_utf8() {
        let m = JsonMatcherV1::Sha256 {
            expected: sha256_of_bytes("hello".as_bytes()),
        };
        assert!(evaluate("system_prompt", &m, Some(&json!("hello"))).passed);
        assert!(!evaluate("system_prompt", &m, Some(&json!("other"))).passed);
    }

    #[test]
    fn exact_with_delete_normalizer_ignores_timestamps() {
        let normalize = vec![JsonNormalizerV1 {
            pointer: "/0/timestamp".into(),
            operation: NormalizerOperation::Delete,
        }];
        let m = exact(
            json!([{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }]),
            Some(normalize),
        );
        let actual = json!([{ "role": "user", "content": [{ "type": "text", "text": "hi" }], "timestamp": 123 }]);
        assert!(evaluate("messages", &m, Some(&actual)).passed);
    }

    #[test]
    fn subset_is_positional_for_arrays() {
        let m = JsonMatcherV1::Subset {
            expected: json!({ "direction": "write" }),
            normalize: None,
        };
        let actual = json!({ "channel_id": "c", "access_key": "k", "direction": "write" });
        assert!(evaluate("writer_ref", &m, Some(&actual)).passed);

        let arr = JsonMatcherV1::Subset {
            expected: json!([{ "a": 1 }, { "b": 2 }]),
            normalize: None,
        };
        // Positional: expected[1] is compared to actual[1], not searched for.
        assert!(
            !evaluate(
                "x",
                &arr,
                Some(&json!([{ "a": 1 }, { "c": 3 }, { "b": 2 }]))
            )
            .passed
        );
        assert!(
            evaluate(
                "x",
                &arr,
                Some(&json!([{ "a": 1, "extra": true }, { "b": 2 }, { "c": 3 }]))
            )
            .passed
        );
    }

    #[test]
    fn delete_normalizer_missing_path_is_noop_and_root_delete_fails() {
        let mut doc = json!({ "a": 1 });
        apply_normalizer(
            &mut doc,
            &JsonNormalizerV1 {
                pointer: "/missing/deep".into(),
                operation: NormalizerOperation::Delete,
            },
        )
        .unwrap();
        assert_eq!(doc, json!({ "a": 1 }));
        assert!(apply_normalizer(
            &mut doc,
            &JsonNormalizerV1 {
                pointer: String::new(),
                operation: NormalizerOperation::Delete,
            },
        )
        .is_err());
    }
}
