//! The `"__redacted__"` round trip for stored connection entries.
//!
//! `database::getConnection` returns an entry as stored, references and
//! templates intact, except that a LITERAL sensitive value — a password, a
//! passphrase, an inline private key, the password inside a url — comes back
//! as [`REDACTED`]. A client edits the entry and sends it back to
//! `saveConnection` / `testConnection`; every field still holding the
//! placeholder takes the stored value again. So a stored literal never
//! leaves the worker, and a client that never saw it can still keep it.
//!
//! Not redacted: `secret://` / `env://` references and `${VAR}` templates
//! (they name a value, they are not one), file paths, and public
//! certificates (`ca_cert`, `client_cert`).

use serde_json::Value;

pub use crate::config::REDACTED;

/// Sensitive leaf fields, as JSON paths inside an entry.
const SECRET_FIELDS: [&[&str]; 3] = [&["password"], &["ssh", "password"], &["ssh", "passphrase"]];
/// Fields that hold a key OR a path to one: a path is not a secret.
const KEY_FIELDS: [&[&str]; 2] = [&["ssh", "private_key"], &["tls", "client_key"]];

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
fn names_a_value(value: &str) -> bool {
    let trimmed = value.trim();
    if let Some(reference) = crate::secrets::parse_ref(trimmed) {
        return reference.is_ok();
    }
    match trimmed.strip_prefix("${").and_then(|t| t.strip_suffix('}')) {
        Some(inner) if !inner.contains("${") && !inner.contains('}') => inner
            .split_once(':')
            .is_none_or(|(_, default)| default.is_empty()),
        _ => false,
    }
}

/// A file path rather than key material. A long run of base64 that merely
/// starts with `/` is a pasted key, not a path.
fn looks_like_path(value: &str) -> bool {
    let v = value.trim();
    if v.contains('\n') || v.contains("-----BEGIN") || v.len() > 4096 {
        return false;
    }
    let anchored = v.starts_with('/')
        || v.starts_with("~/")
        || v.starts_with("./")
        || v.starts_with("../")
        || (v.len() > 2 && v.as_bytes()[1] == b':' && matches!(v.as_bytes()[2], b'\\' | b'/'));
    if !anchored {
        return false;
    }
    let base64ish = v.chars().all(|c| {
        c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '.' | '~' | '\\' | ':')
    });
    let long_segment = v.split(['/', '\\']).any(|segment| segment.len() > 64);
    !(base64ish && v.len() > 64 && (v.contains('+') || v.ends_with('=') || long_segment))
}

/// Replace every literal sensitive value of an entry with [`REDACTED`].
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
    for path in KEY_FIELDS {
        if let Some(v) = get_mut(entry, path) {
            if v.as_str()
                .is_some_and(|s| !s.is_empty() && !names_a_value(s) && !looks_like_path(s))
            {
                *v = Value::String(REDACTED.into());
            }
        }
    }
    if let Some(url) = entry.get_mut("url") {
        if let Some(redacted) = url.as_str().and_then(redact_url_password) {
            *url = Value::String(redacted);
        }
    }
}

/// The url with a literal password (userinfo, or a `password` parameter)
/// replaced by the placeholder; `None` when there is nothing to hide.
fn redact_url_password(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    let mut changed = false;
    if let Some(password) = url.password() {
        if !names_a_value(&percent_decode(password)) {
            url.set_password(Some(REDACTED)).ok()?;
            changed = true;
        }
    }
    if url
        .query_pairs()
        .any(|(k, v)| k.eq_ignore_ascii_case("password") && !names_a_value(&v))
    {
        let pairs: Vec<(String, String)> = url
            .query_pairs()
            .map(|(k, v)| {
                if k.eq_ignore_ascii_case("password") {
                    (k.into_owned(), REDACTED.to_string())
                } else {
                    (k.into_owned(), v.into_owned())
                }
            })
            .collect();
        url.query_pairs_mut().clear().extend_pairs(pairs);
        changed = true;
    }
    changed.then(|| url.to_string())
}

/// Whether a url carries a literal password, in its userinfo or as a
/// `password` parameter (percent-encoding decoded before deciding).
fn url_has_literal_password(raw: &str) -> bool {
    let Ok(url) = url::Url::parse(raw) else {
        return false;
    };
    let userinfo = url
        .password()
        .map(percent_decode)
        .is_some_and(|p| p != REDACTED && !names_a_value(&p));
    let param = url
        .query_pairs()
        .any(|(k, v)| k.eq_ignore_ascii_case("password") && v != REDACTED && !names_a_value(&v));
    userinfo || param
}

fn percent_decode(s: &str) -> String {
    url::form_urlencoded::parse(format!("x={}", s.replace('+', "%2B")).as_bytes())
        .next()
        .map(|(_, v)| v.into_owned())
        .unwrap_or_else(|| s.to_string())
}

/// Whether any string anywhere in `value` is (or a url password is) the
/// placeholder.
pub fn contains_redacted(value: &Value) -> bool {
    match value {
        Value::String(s) => s == REDACTED || s.contains(&format!(":{REDACTED}@")),
        Value::Array(items) => items.iter().any(contains_redacted),
        Value::Object(map) => map.values().any(contains_redacted),
        _ => false,
    }
}

/// Sensitive fields of a request entry that hold a literal (not a
/// reference, a template, the placeholder, or — for key fields — a path),
/// including a password inside `url`. Such values must not travel inside a
/// request's `connection`: trace payloads redact `password` by key name but
/// not `passphrase`, `client_key` or a url's password.
pub fn literal_sensitive_fields(entry: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for path in SECRET_FIELDS {
        if get(entry, path)
            .and_then(Value::as_str)
            .is_some_and(|s| !s.is_empty() && s != REDACTED && !names_a_value(s))
        {
            out.push(path.join("."));
        }
    }
    for path in KEY_FIELDS {
        if get(entry, path).and_then(Value::as_str).is_some_and(|s| {
            !s.is_empty() && s != REDACTED && !names_a_value(s) && !looks_like_path(s)
        }) {
            out.push(path.join("."));
        }
    }
    if entry
        .get("url")
        .and_then(Value::as_str)
        .is_some_and(url_has_literal_password)
    {
        out.push("url".into());
    }
    out
}

/// Put the stored values back wherever `candidate` holds the placeholder,
/// returning the fields restored (`password`, `ssh.passphrase`, `url`, …).
/// Fails, naming the field, when there is no stored value to restore.
pub fn restore_entry(candidate: &mut Value, stored: Option<&Value>) -> Result<Vec<String>, String> {
    let mut restored = Vec::new();
    for path in SECRET_FIELDS.iter().chain(KEY_FIELDS.iter()) {
        let Some(v) = get_mut(candidate, path) else {
            continue;
        };
        if v.as_str() != Some(REDACTED) {
            continue;
        }
        let original = stored
            .and_then(|s| get(s, path))
            .filter(|o| o.as_str().is_some_and(|s| s != REDACTED))
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
    if let Some(url_value) = candidate.get_mut("url") {
        if let Some(raw) = url_value.as_str() {
            if raw.contains(REDACTED) {
                let mut url = url::Url::parse(raw)
                    .ok()
                    .filter(|u| u.password() == Some(REDACTED))
                    .ok_or_else(|| {
                        format!("`url` may hold \"{REDACTED}\" only as its password; send the url")
                    })?;
                let stored_password = stored
                    .and_then(|s| s.get("url"))
                    .and_then(Value::as_str)
                    .and_then(|u| url::Url::parse(u).ok())
                    .and_then(|u| u.password().map(str::to_string))
                    .filter(|p| p != REDACTED)
                    .ok_or_else(|| {
                        format!(
                            "`url` has the password \"{REDACTED}\" but there is no stored url \
                             password to keep; send the password"
                        )
                    })?;
                // Already percent-encoded: set it back verbatim.
                url.set_password(Some(&percent_decode(&stored_password)))
                    .map_err(|_| "`url`: cannot restore the stored password".to_string())?;
                *url_value = Value::String(url.to_string());
                restored.push("url".into());
            }
        }
    }
    if contains_redacted(candidate) {
        return Err(format!(
            "\"{REDACTED}\" is only accepted in password, passphrase, private_key, client_key \
             and the url's password"
        ));
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn literals_are_redacted_references_templates_and_paths_are_not() {
        let mut entry = json!({
            "driver": "postgres", "host": "db", "user": "app",
            "password": "hunter2",
            "tls": { "mode": "verify-full", "ca_cert": "-----BEGIN CERTIFICATE-----\nAAA\n-----END CERTIFICATE-----\n",
                     "client_cert": "/etc/ssl/client.pem", "client_key": "-----BEGIN PRIVATE KEY-----\nKKK\n-----END PRIVATE KEY-----\n" },
            "ssh": { "host": "b", "user": "ops", "password": "secret://SSH_PW",
                     "private_key": "/home/me/.ssh/id_ed25519", "passphrase": "${SSH_PASSPHRASE}" }
        });
        redact_entry(&mut entry);
        assert_eq!(entry["password"], REDACTED);
        assert_eq!(entry["tls"]["client_key"], REDACTED);
        assert!(entry["tls"]["ca_cert"]
            .as_str()
            .unwrap()
            .contains("BEGIN CERTIFICATE"));
        assert_eq!(entry["tls"]["client_cert"], "/etc/ssl/client.pem");
        assert_eq!(entry["ssh"]["password"], "secret://SSH_PW");
        assert_eq!(entry["ssh"]["private_key"], "/home/me/.ssh/id_ed25519");
        assert_eq!(entry["ssh"]["passphrase"], "${SSH_PASSPHRASE}");
        assert!(!entry.to_string().contains("hunter2"));
        assert!(!entry.to_string().contains("KKK"));
    }

    #[test]
    fn url_passwords_are_redacted_and_restored() {
        let stored = json!({ "url": "postgres://app:p%40ss@db:5432/shop" });
        let mut shown = stored.clone();
        redact_entry(&mut shown);
        assert_eq!(
            shown["url"],
            format!("postgres://app:{REDACTED}@db:5432/shop")
        );
        // The client changes the host and keeps the password.
        let mut back = json!({ "url": format!("postgres://app:{REDACTED}@db2:5432/shop") });
        restore_entry(&mut back, Some(&stored)).unwrap();
        assert_eq!(back["url"], "postgres://app:p%40ss@db2:5432/shop");
        // A url naming a reference as its password is not a secret.
        let mut env = json!({ "url": "postgres://app:${PGPASS}@db/shop" });
        redact_entry(&mut env);
        assert!(env["url"].as_str().unwrap().contains("PGPASS"));
    }

    #[test]
    fn round_trip_keeps_stored_literals_and_takes_new_values() {
        let stored = json!({
            "driver": "mysql", "host": "db", "user": "app", "password": "hunter2",
            "ssh": { "host": "b", "user": "ops", "private_key": "-----BEGIN OPENSSH PRIVATE KEY-----\nXYZ\n-----END OPENSSH PRIVATE KEY-----\n", "passphrase": "pp" }
        });
        let mut shown = stored.clone();
        redact_entry(&mut shown);
        assert!(contains_redacted(&shown));
        shown["host"] = json!("db2");
        shown["ssh"]["passphrase"] = json!("new-pp");
        restore_entry(&mut shown, Some(&stored)).unwrap();
        assert_eq!(shown["password"], "hunter2");
        assert_eq!(shown["ssh"]["private_key"], stored["ssh"]["private_key"]);
        assert_eq!(shown["ssh"]["passphrase"], "new-pp");
        assert_eq!(shown["host"], "db2");
        assert!(!contains_redacted(&shown));
    }

    #[test]
    fn literals_are_listed_references_paths_templates_and_placeholders_are_not() {
        let entry = json!({
            "url": "postgres://u:pw@h/db",
            "password": "secret://P",
            "tls": { "client_key": "-----BEGIN PRIVATE KEY-----\nK\n-----END PRIVATE KEY-----" },
            "ssh": { "password": "${SSH_PW}", "private_key": "/home/me/.ssh/id", "passphrase": "pp" }
        });
        assert_eq!(
            literal_sensitive_fields(&entry),
            vec!["ssh.passphrase", "tls.client_key", "url"]
        );
        let clean = json!({ "url": format!("postgres://u:{REDACTED}@h/db"), "password": REDACTED });
        assert!(literal_sensitive_fields(&clean).is_empty());
    }

    #[test]
    fn templates_with_defaults_and_malformed_references_are_literals() {
        let entry = json!({
            "password": "${DB_PW:hunter2}",
            "ssh": { "password": "${SSH_PW}", "passphrase": "secret:// hunter2", "private_key": "${KEY:}" },
            "url": "postgres://u:%24%7BPGPW%3Ahunter2%7D@h/db"
        });
        assert_eq!(
            literal_sensitive_fields(&entry),
            vec!["password", "ssh.passphrase", "url"]
        );
        let mut shown = entry.clone();
        redact_entry(&mut shown);
        assert!(!shown.to_string().contains("hunter2"), "{shown}");
        assert_eq!(shown["ssh"]["password"], "${SSH_PW}");
    }

    #[test]
    fn a_password_parameter_is_a_url_password() {
        let entry = json!({ "url": "postgres://u@h/db?password=hunter2&sslmode=require" });
        assert_eq!(literal_sensitive_fields(&entry), vec!["url"]);
        let mut shown = entry.clone();
        redact_entry(&mut shown);
        assert!(!shown.to_string().contains("hunter2"), "{shown}");
        assert!(shown["url"].as_str().unwrap().contains("sslmode=require"));
    }

    #[test]
    fn a_pasted_key_starting_with_a_slash_is_not_a_path() {
        let key = format!(
            "/{}",
            "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC7+".repeat(3)
        );
        let entry = json!({ "tls": { "client_key": key }, "ssh": { "private_key": "/home/me/.ssh/id_ed25519" } });
        assert_eq!(literal_sensitive_fields(&entry), vec!["tls.client_key"]);
    }

    #[test]
    fn restore_reports_what_it_restored() {
        let stored = json!({ "url": "postgres://u:pw@h/db", "ssh": { "passphrase": "pp" } });
        let mut candidate = json!({ "url": format!("postgres://u:{REDACTED}@h2/db"), "ssh": { "passphrase": REDACTED } });
        let restored = restore_entry(&mut candidate, Some(&stored)).unwrap();
        assert_eq!(restored, vec!["ssh.passphrase", "url"]);
    }

    #[test]
    fn a_placeholder_with_nothing_stored_is_refused_by_field() {
        let mut candidate =
            json!({ "driver": "postgres", "host": "db", "user": "u", "password": REDACTED });
        let err = restore_entry(&mut candidate, None).unwrap_err();
        assert!(err.contains("`password`"), "{err}");
        let mut elsewhere = json!({ "driver": "postgres", "host": REDACTED, "user": "u" });
        assert!(restore_entry(&mut elsewhere, Some(&json!({}))).is_err());
    }
}
