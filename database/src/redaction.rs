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

/// A reference or a pure `${VAR}` template names a value without being one.
fn names_a_value(value: &str) -> bool {
    let trimmed = value.trim();
    crate::secrets::parse_ref(trimmed).is_some()
        || (trimmed.starts_with("${")
            && trimmed.ends_with('}')
            && trimmed.matches("${").count() == 1)
}

/// A file path rather than key material.
fn looks_like_path(value: &str) -> bool {
    let v = value.trim();
    !v.contains('\n')
        && !v.contains("-----BEGIN")
        && (v.starts_with('/')
            || v.starts_with("~/")
            || v.starts_with("./")
            || v.starts_with("../")
            || (v.len() > 2 && v.as_bytes()[1] == b':' && matches!(v.as_bytes()[2], b'\\' | b'/')))
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

/// The url with a literal password replaced by the placeholder; `None` when
/// there is nothing to hide.
fn redact_url_password(raw: &str) -> Option<String> {
    let mut url = url::Url::parse(raw).ok()?;
    let password = url.password()?;
    let decoded = percent_decode(password);
    if names_a_value(&decoded) {
        return None;
    }
    url.set_password(Some(REDACTED)).ok()?;
    Some(url.to_string())
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

/// Put the stored values back wherever `candidate` holds the placeholder.
/// Fails, naming the field, when there is no stored value to restore.
pub fn restore_entry(candidate: &mut Value, stored: Option<&Value>) -> Result<(), String> {
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
            }
        }
    }
    if contains_redacted(candidate) {
        return Err(format!(
            "\"{REDACTED}\" is only accepted in password, passphrase, private_key, client_key \
             and the url's password"
        ));
    }
    Ok(())
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
    fn a_placeholder_with_nothing_stored_is_refused_by_field() {
        let mut candidate =
            json!({ "driver": "postgres", "host": "db", "user": "u", "password": REDACTED });
        let err = restore_entry(&mut candidate, None).unwrap_err();
        assert!(err.contains("`password`"), "{err}");
        let mut elsewhere = json!({ "driver": "postgres", "host": REDACTED, "user": "u" });
        assert!(restore_entry(&mut elsewhere, Some(&json!({}))).is_err());
    }
}
