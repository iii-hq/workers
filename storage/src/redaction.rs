//! The `"__redacted__"` round trip for stored bucket entries, and the
//! `credentials` envelope of `storage::saveBucket` / `storage::testBucket`.
//!
//! `storage::getBucket` returns an entry as stored, references and
//! templates intact, except that a LITERAL sensitive value — a secret access
//! key, a session token, a service-account JSON, a Cloudflare API token —
//! comes back as [`REDACTED`]. A client edits the entry and sends it back;
//! every field still holding the placeholder takes the stored value again.
//! So a stored literal never leaves the worker, and a client that never saw
//! it can still keep it.
//!
//! Not redacted: `secret://` / `env://` references and `${VAR}` templates
//! (they name a value, they are not one), access key ids, account ids and
//! file paths.

use serde_json::{Map, Value};

pub use crate::config::REDACTED;

/// Sensitive leaf fields, as JSON paths inside an entry.
const SECRET_FIELDS: [&[&str]; 4] = [
    &["secret_access_key"],
    &["session_token"],
    &["credentials_json"],
    &["notifications", "api_token"],
];

/// `credentials` keys and the entry paths they stand for.
const CREDENTIAL_KEYS: [(&str, &[&str]); 5] = [
    ("access_key_id", &["access_key_id"]),
    ("secret_access_key", &["secret_access_key"]),
    ("session_token", &["session_token"]),
    ("credentials_json", &["credentials_json"]),
    ("notifications_api_token", &["notifications", "api_token"]),
];

fn get<'a>(entry: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(entry, |v, k| v.get(k))
}

fn get_mut<'a>(entry: &'a mut Value, path: &[&str]) -> Option<&'a mut Value> {
    path.iter().try_fold(entry, |v, k| v.get_mut(k))
}

/// A well-formed reference, or a pure `${VAR}` template without a default
/// (or with an empty one), names a value without being one. A malformed
/// reference (`secret:// hunter2`) and a template with a default
/// (`${VAR:hunter2}`, which IS used when the variable is unset) are literals.
pub fn names_a_value(value: &str) -> bool {
    let trimmed = value.trim();
    if let Some(reference) = crate::secrets::parse_ref(trimmed) {
        return reference.is_ok();
    }
    match trimmed.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
        Some(inner) if !inner.is_empty() && !inner.contains("${") && !inner.contains('}') => inner
            .split_once(':')
            .is_none_or(|(_, default)| default.is_empty()),
        _ => false,
    }
}

/// Replace every literal sensitive value of an entry with [`REDACTED`], and
/// drop any userinfo from its `endpoint_url` (a hand-written entry may carry
/// one; requests may not).
pub fn redact_entry(entry: &mut Value) {
    for path in SECRET_FIELDS {
        if let Some(v) = get_mut(entry, path) {
            if v.as_str()
                .is_some_and(|s| !s.is_empty() && !names_a_value(s))
            {
                *v = Value::String(REDACTED.into());
            }
        }
    }
    // Keys that are not fields (a hand-edited entry, `secret_access_key_old`)
    // are shown as the placeholder too; a save keeps them with it. An entry
    // without a known provider shows nothing but its provider.
    let provider_known = entry
        .get("provider")
        .and_then(Value::as_str)
        .and_then(crate::config::entry_keys)
        .is_some();
    if provider_known {
        for path in crate::config::unknown_entry_keys(entry) {
            set_at(entry, &path, Value::String(REDACTED.into()));
        }
    } else if let Some(fields) = entry.as_object_mut() {
        for (key, value) in fields.iter_mut() {
            if key != "provider" {
                *value = Value::String(REDACTED.into());
            }
        }
    }
    if let Some(endpoint) = entry.get_mut("endpoint_url") {
        if endpoint.as_str().is_some_and(has_userinfo) {
            let stripped = crate::config::redact_url(endpoint.as_str().unwrap_or_default());
            *endpoint = Value::String(stripped);
        }
    }
}

/// Whether a url carries `user[:password]@`.
pub fn has_userinfo(raw: &str) -> bool {
    url::Url::parse(raw.trim())
        .map(|url| !url.username().is_empty() || url.password().is_some())
        .unwrap_or(false)
}

/// Where a string holds a `${…}` template, as dotted paths.
pub fn template_paths(value: &Value, prefix: &str) -> Vec<String> {
    match value {
        Value::String(s) if s.contains("${") => vec![prefix.to_string()],
        Value::Object(map) => map
            .iter()
            .flat_map(|(k, v)| {
                let path = if prefix.is_empty() {
                    k.clone()
                } else {
                    format!("{prefix}.{k}")
                };
                template_paths(v, &path)
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The value at a dotted path.
pub fn at<'a>(value: Option<&'a Value>, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(value?, |v, k| v.get(k))
}

/// Replace the value at a dotted path that exists.
pub fn set_at(entry: &mut Value, path: &str, value: Value) {
    if let Some(slot) = path.split('.').try_fold(entry, |v, k| v.get_mut(k)) {
        *slot = value;
    }
}

/// Credential fields holding a malformed `secret://` / `env://` reference.
pub fn malformed_references(entry: &Value) -> Vec<String> {
    crate::binding::CREDENTIAL_PATHS
        .iter()
        .filter(|path| {
            at(Some(entry), path)
                .and_then(Value::as_str)
                .and_then(crate::secrets::parse_ref)
                .is_some_and(|r| r.is_err())
        })
        .map(|path| path.to_string())
        .collect()
}

/// The refusal for a malformed reference (the value is not echoed).
pub fn malformed_refusal(field: &str) -> String {
    format!(
        "`{field}` holds a malformed reference; the name after secret:// or env:// must match \
         ^[A-Za-z_][A-Za-z0-9_.-]{{0,127}}$"
    )
}

/// Whether any string anywhere in `value` is the placeholder.
pub fn contains_redacted(value: &Value) -> bool {
    match value {
        Value::String(s) => s.trim() == REDACTED,
        Value::Array(items) => items.iter().any(contains_redacted),
        Value::Object(map) => map.values().any(contains_redacted),
        _ => false,
    }
}

/// Sensitive fields of a request entry that hold a literal (not a
/// reference, a template or the placeholder). Such values must not travel
/// inside a request's `bucket`: they go as references or in `credentials`.
pub fn literal_sensitive_fields(entry: &Value) -> Vec<String> {
    SECRET_FIELDS
        .iter()
        .filter(|path| {
            get(entry, path)
                .and_then(Value::as_str)
                .is_some_and(|s| !s.is_empty() && s.trim() != REDACTED && !names_a_value(s))
        })
        .map(|path| path.join("."))
        .collect()
}

/// The refusal for a literal credential inside `bucket`.
pub fn literal_refusal(field: &str) -> String {
    let key = CREDENTIAL_KEYS
        .iter()
        .find(|(_, path)| path.join(".") == field)
        .map(|(key, _)| *key)
        .unwrap_or(field);
    format!(
        "`bucket.{field}` holds a literal credential; send it as a secret:// or env:// \
         reference, or in the request's `credentials` (\"credentials\": {{ \"{key}\": \"…\" }})"
    )
}

/// Merge a `credentials` object (`access_key_id`, `secret_access_key`,
/// `session_token`, `credentials_json`, `notifications_api_token`) over
/// `entry`. Any other key, or a non-string value, is refused by name, so a
/// client cannot hide a whole entry inside it.
pub fn merge_credentials(entry: &mut Value, credentials: Option<&Value>) -> Result<(), String> {
    let Some(credentials) = credentials.filter(|c| !c.is_null()) else {
        return Ok(());
    };
    let Some(fields) = credentials.as_object() else {
        return Err("`credentials` must be an object".into());
    };
    for (key, value) in fields {
        let Some((_, path)) = CREDENTIAL_KEYS.iter().find(|(k, _)| k == key) else {
            return Err(format!(
                "`credentials.{key}` is not a credential field (access_key_id, \
                 secret_access_key, session_token, credentials_json, notifications_api_token)"
            ));
        };
        if value.is_null() {
            continue;
        }
        if !value.is_string() {
            return Err(format!("`credentials.{key}` must be a string"));
        }
        let Some(obj) = entry.as_object_mut() else {
            return Err("`bucket` must be an object".into());
        };
        match path {
            [field] => {
                obj.insert((*field).to_string(), value.clone());
            }
            [parent, field] => {
                let child = obj
                    .entry((*parent).to_string())
                    .or_insert_with(|| Value::Object(Map::new()));
                match child.as_object_mut() {
                    Some(child) => {
                        child.insert((*field).to_string(), value.clone());
                    }
                    None => return Err(format!("`bucket.{parent}` must be an object")),
                }
            }
            _ => unreachable!("credential paths are one or two levels deep"),
        }
    }
    Ok(())
}

/// Put the stored values back wherever `candidate` holds the placeholder,
/// returning the fields restored (`secret_access_key`, …). Fails, naming the
/// field, when there is no stored value to restore or the placeholder sits
/// anywhere else.
pub fn restore_entry(candidate: &mut Value, stored: Option<&Value>) -> Result<Vec<String>, String> {
    let mut restored = Vec::new();
    for path in SECRET_FIELDS {
        let Some(v) = get_mut(candidate, path) else {
            continue;
        };
        if v.as_str().map(str::trim) != Some(REDACTED) {
            continue;
        }
        let original = stored
            .and_then(|s| get(s, path))
            .filter(|o| o.as_str().is_some_and(|s| s.trim() != REDACTED))
            .cloned()
            .ok_or_else(|| {
                format!(
                    "`{}` is \"{REDACTED}\" but there is no stored value to keep; send the value",
                    path.join(".")
                )
            })?;
        *v = original;
        restored.push(path.join("."));
    }
    if contains_redacted(candidate) {
        return Err(format!(
            "\"{REDACTED}\" is only accepted in secret_access_key, session_token, \
             credentials_json and notifications.api_token"
        ));
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn literals_are_redacted_references_templates_and_ids_are_not() {
        let mut entry = json!({
            "provider": "r2", "account_id": "acc-literal", "access_key_id": "AKIA-literal",
            "secret_access_key": "hunter2",
            "notifications": { "queue_id": "q", "api_token": "cf-token" }
        });
        redact_entry(&mut entry);
        assert_eq!(entry["secret_access_key"], REDACTED);
        assert_eq!(entry["notifications"]["api_token"], REDACTED);
        assert_eq!(entry["account_id"], "acc-literal");
        assert_eq!(entry["access_key_id"], "AKIA-literal");
        let mut refs = json!({
            "provider": "s3", "region": "r",
            "secret_access_key": "secret://SK", "session_token": "${AWS_SESSION_TOKEN}"
        });
        let before = refs.clone();
        redact_entry(&mut refs);
        assert_eq!(refs, before);
    }

    #[test]
    fn templates_with_defaults_and_malformed_references_are_literals() {
        let entry = json!({
            "secret_access_key": "${SK:hunter2}",
            "session_token": "secret:// hunter2",
            "credentials_json": "${GCS_JSON}"
        });
        assert_eq!(
            literal_sensitive_fields(&entry),
            vec!["secret_access_key", "session_token"]
        );
        let mut shown = entry.clone();
        shown["provider"] = json!("s3");
        redact_entry(&mut shown);
        assert!(!shown.to_string().contains("hunter2"), "{shown}");
        let mut gcs = json!({ "provider": "gcs", "credentials_json": "${GCS_JSON}" });
        redact_entry(&mut gcs);
        assert_eq!(gcs["credentials_json"], "${GCS_JSON}");
        assert!(!names_a_value("${}"));
    }

    #[test]
    fn round_trip_keeps_stored_literals_and_takes_new_values() {
        let stored = json!({
            "provider": "s3", "region": "us-east-1", "access_key_id": "AKIA",
            "secret_access_key": "hunter2", "session_token": "tok"
        });
        let mut shown = stored.clone();
        redact_entry(&mut shown);
        assert!(contains_redacted(&shown));
        shown["region"] = json!("eu-west-1");
        shown["session_token"] = json!("secret://NEW_TOKEN");
        let restored = restore_entry(&mut shown, Some(&stored)).unwrap();
        assert_eq!(restored, vec!["secret_access_key"]);
        assert_eq!(shown["secret_access_key"], "hunter2");
        assert_eq!(shown["session_token"], "secret://NEW_TOKEN");
        assert_eq!(shown["region"], "eu-west-1");
    }

    #[test]
    fn a_placeholder_with_nothing_stored_or_elsewhere_is_refused() {
        let mut candidate =
            json!({ "provider": "s3", "region": "r", "secret_access_key": REDACTED });
        let err = restore_entry(&mut candidate, None).unwrap_err();
        assert!(err.contains("`secret_access_key`"), "{err}");
        let mut elsewhere = json!({ "provider": "s3", "region": REDACTED });
        let err = restore_entry(&mut elsewhere, Some(&json!({}))).unwrap_err();
        assert!(err.contains("only accepted"), "{err}");
        // A stored placeholder is not a value to restore either.
        let mut again = json!({ "provider": "s3", "region": "r", "secret_access_key": REDACTED });
        assert!(
            restore_entry(&mut again, Some(&json!({ "secret_access_key": REDACTED }))).is_err()
        );
    }

    #[test]
    fn credentials_merge_over_the_entry_and_nothing_else_rides_along() {
        let mut entry =
            json!({ "provider": "r2", "account_id": "a", "notifications": { "queue_id": "q" } });
        merge_credentials(
            &mut entry,
            Some(&json!({ "access_key_id": "ak", "secret_access_key": "sk", "notifications_api_token": "tok" })),
        )
        .unwrap();
        assert_eq!(entry["access_key_id"], "ak");
        assert_eq!(entry["secret_access_key"], "sk");
        assert_eq!(entry["notifications"]["api_token"], "tok");
        assert_eq!(entry["notifications"]["queue_id"], "q", "siblings are kept");
        let err = merge_credentials(&mut entry, Some(&json!({ "region": "evil" }))).unwrap_err();
        assert!(
            err.contains("`credentials.region` is not a credential field"),
            "{err}"
        );
        assert!(merge_credentials(&mut entry, Some(&json!({ "session_token": 5 }))).is_err());
        assert!(merge_credentials(&mut entry, Some(&json!("x"))).is_err());
        merge_credentials(&mut entry, None).unwrap();
        merge_credentials(&mut entry, Some(&Value::Null)).unwrap();
    }

    #[test]
    fn templates_malformed_references_and_userinfo_are_found() {
        let entry = json!({
            "provider": "s3", "region": "${REGION}", "endpoint_url": "https://u:pw@h",
            "session_token": "secret://a b", "access_key_id": "env://", "secret_access_key": "secret://OK",
            "notifications": { "sqs_queue_url": "x${Y}" }
        });
        let mut templates = template_paths(&entry, "");
        templates.sort();
        assert_eq!(templates, vec!["notifications.sqs_queue_url", "region"]);
        assert_eq!(
            malformed_references(&entry),
            vec!["access_key_id", "session_token"]
        );
        assert!(malformed_refusal("session_token").contains("malformed reference"));
        let mut shown = entry.clone();
        redact_entry(&mut shown);
        assert!(!shown.to_string().contains("pw"), "{shown}");
        assert!(has_userinfo("https://u@h") && !has_userinfo("https://h"));
    }

    #[test]
    fn unknown_keys_are_redacted_whatever_they_hold() {
        let mut entry = json!({ "provider": "s3", "region": "r", "secret_access_key_old": "OLD-SECRET",
                                "_secret_access_key": { "v": "X" }, "display": { "icon": "a", "note": "N" } });
        redact_entry(&mut entry);
        assert_eq!(entry["secret_access_key_old"], REDACTED);
        assert_eq!(entry["_secret_access_key"], REDACTED);
        assert_eq!(entry["display"]["note"], REDACTED);
        assert_eq!(entry["display"]["icon"], "a");
        assert_eq!(entry["region"], "r");
        let mut odd = json!({ "provider": "ftp", "password": "P" });
        redact_entry(&mut odd);
        assert_eq!(odd, json!({ "provider": "ftp", "password": REDACTED }));
    }

    #[test]
    fn the_refusal_names_the_credentials_key() {
        let msg = literal_refusal("notifications.api_token");
        assert!(msg.contains("`bucket.notifications.api_token`"), "{msg}");
        assert!(msg.contains("\"notifications_api_token\""), "{msg}");
    }
}
