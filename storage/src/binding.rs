//! Where a bucket entry's credentials are sent, and the rules that keep a
//! stored credential where it belongs.
//!
//! `storage::testBucket` sends a candidate's credentials wherever the
//! candidate points, and `storage::saveBucket` makes a live bucket do the
//! same on its next call. Neither may become a way to read a credential the
//! caller cannot read otherwise (the vault never returns values, a stored
//! literal comes back `"__redacted__"`, a file on the engine host is not
//! readable through the bus). So:
//!
//! - a value **kept** from a stored entry (`"__redacted__"` restored, a
//!   `${VAR}` template left unchanged) only goes to that entry's provider and
//!   destination ([`check_kept`]);
//! - a **bound** credential — a `secret://` / `env://` reference or a
//!   `credentials_file` that a stored entry uses, in any credential field —
//!   only goes, in the same field, to a destination a stored entry sends it
//!   to ([`check_bound`]);
//! - the engine host's **ambient** credentials (the AWS default chain, Google
//!   Application Default Credentials), used when an entry names none, only go
//!   to the provider itself or to an endpoint a stored entry already uses them
//!   with.
//!
//! Every rule reads entries as JSON, so stored entries that do not parse
//! still bind their references.

use std::collections::HashMap;

use serde_json::{Map, Value};

/// Every field that carries a credential, as a dotted path.
pub const CREDENTIAL_PATHS: [&str; 7] = [
    "access_key_id",
    "secret_access_key",
    "session_token",
    "account_id",
    "credentials_json",
    "credentials_file",
    "notifications.api_token",
];

/// The pseudo-field ambient credentials are reported under.
const AMBIENT_FIELD: &str = "(default credentials)";

/// A string at a dotted path.
pub fn get_str<'a>(entry: &'a Value, path: &str) -> Option<&'a str> {
    path.split('.')
        .try_fold(entry, |v, k| v.get(k))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|v| !v.is_empty())
}

fn provider(entry: &Value) -> &str {
    get_str(entry, "provider").unwrap_or("")
}

/// `scheme://host:port`, lowercased; anything unparseable stands for itself.
pub fn normalize_endpoint(raw: &str) -> String {
    match url::Url::parse(raw.trim()) {
        Ok(url) => format!(
            "{}://{}:{}",
            url.scheme(),
            url.host_str().unwrap_or("").to_ascii_lowercase(),
            url.port_or_known_default().unwrap_or(0)
        ),
        Err(_) => format!("unparsed:{}", raw.trim().to_ascii_lowercase()),
    }
}

/// Where a credential field of an entry is sent: its `endpoint_url`, or the
/// provider's own service (an R2 account's endpoint is named by its
/// `account_id`). A Cloudflare API token always goes to the Cloudflare API.
pub fn destination(entry: &Value, field: &str) -> String {
    if field == "notifications.api_token" {
        return "cloudflare-api".into();
    }
    if let Some(endpoint) = get_str(entry, "endpoint_url") {
        return normalize_endpoint(endpoint);
    }
    match provider(entry) {
        "s3" => "aws".into(),
        "gcs" => "google".into(),
        "r2" => format!(
            "r2-account:{}",
            get_str(entry, "account_id")
                .unwrap_or("")
                .to_ascii_lowercase()
        ),
        other => format!("{other}:default"),
    }
}

/// What makes a field's value bound: its canonical reference, or the
/// credentials file it names (canonicalized, so another spelling of the same
/// file is the same file). A literal is not bound.
fn binding_key(field: &str, value: &str) -> Option<String> {
    if field == "credentials_file" {
        let path = std::path::Path::new(value.trim());
        let canonical = std::fs::canonicalize(path)
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_else(|_| lexical(path));
        return Some(format!("file:{canonical}"));
    }
    crate::secrets::parse_ref(value)
        .and_then(Result::ok)
        .map(|r| r.key())
}

/// `a/./b/../c` → `a/c`, for a path that does not exist (yet).
fn lexical(path: &std::path::Path) -> String {
    use std::path::Component;
    let mut out: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            Component::RootDir => out.push(String::new()),
            other => out.push(other.as_os_str().to_string_lossy().to_string()),
        }
    }
    out.join("/")
}

/// The ambient credentials an entry falls back to, if any: an s3 entry
/// without both keys signs with the AWS default chain, a gcs entry without
/// `credentials_json` / `credentials_file` uses Google ADC.
fn ambient_key(entry: &Value) -> Option<&'static str> {
    match provider(entry) {
        "s3" if get_str(entry, "access_key_id").is_none()
            || get_str(entry, "secret_access_key").is_none() =>
        {
            Some("ambient:aws")
        }
        "gcs"
            if get_str(entry, "credentials_json").is_none()
                && get_str(entry, "credentials_file").is_none() =>
        {
            Some("ambient:gcp")
        }
        _ => None,
    }
}

/// One stored use of a bound credential.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Use {
    pub bucket: String,
    pub field: String,
    pub destination: String,
}

/// Every bound credential and ambient credential of the stored entries,
/// parsed or not.
pub fn uses(buckets: &Map<String, Value>) -> HashMap<String, Vec<Use>> {
    let mut out: HashMap<String, Vec<Use>> = HashMap::new();
    for (name, entry) in buckets {
        if let Some(key) = ambient_key(entry) {
            out.entry(key.to_string()).or_default().push(Use {
                bucket: name.clone(),
                field: AMBIENT_FIELD.into(),
                destination: destination(entry, AMBIENT_FIELD),
            });
        }
        for field in CREDENTIAL_PATHS {
            let Some(key) = get_str(entry, field).and_then(|v| binding_key(field, v)) else {
                continue;
            };
            out.entry(key).or_default().push(Use {
                bucket: name.clone(),
                field: field.into(),
                destination: destination(entry, field),
            });
        }
    }
    out
}

/// Fields whose own bound value may follow a bucket that is saved toward a
/// new destination, in the same field: only an identifier and a SigV4
/// signature (which signs the Host) leave, never the value itself.
const MAY_FOLLOW: [&str; 3] = ["access_key_id", "secret_access_key", "account_id"];

fn names(uses: &[Use]) -> String {
    let mut names: Vec<&str> = uses.iter().map(|u| u.bucket.as_str()).collect();
    names.sort_unstable();
    names.dedup();
    names.join("`, `")
}

/// Refuse a candidate that would send a bound credential, or the engine
/// host's ambient credentials, anywhere their stored uses do not.
///
/// `own` is the bucket being saved (`saveBucket`'s `previous_name`; `None`
/// for a test): when no other bucket shares them, its own `access_key_id`,
/// `secret_access_key` and `account_id` may follow it to a new destination
/// in the same field; its `session_token`, `credentials_json` and
/// `credentials_file`, which travel in clear or as bearer credentials, may
/// not. (Its API token always goes to the Cloudflare API, so its
/// destination never changes.) A value never changes field.
pub fn check_bound(
    candidate: &Value,
    uses: &HashMap<String, Vec<Use>>,
    own: Option<&str>,
) -> Result<(), String> {
    for field in CREDENTIAL_PATHS {
        let Some(value) = get_str(candidate, field) else {
            continue;
        };
        let Some(key) = binding_key(field, value) else {
            continue;
        };
        let target = destination(candidate, field);
        let Some(stored) = uses.get(&key) else {
            // A file on the engine host no stored bucket uses is the host's
            // credential, like the ambient ones: it only goes to Google.
            if field == "credentials_file" && target != "google" {
                return Err(
                    "`credentials_file`: a credentials file from the engine host only goes to \
                     Google, or where a configured bucket already sends it; for a custom \
                     endpoint put the JSON itself in credentials_json (as a secret:// reference)"
                        .into(),
                );
            }
            continue;
        };
        if stored
            .iter()
            .any(|u| u.field == field && u.destination == target)
        {
            continue;
        }
        let what = if key.starts_with("file:") {
            "this credentials file".to_string()
        } else {
            key.clone()
        };
        let own_same_field =
            own.is_some_and(|own| stored.iter().any(|u| u.bucket == own && u.field == field));
        // Only a value this bucket alone uses may follow it: another bucket
        // sharing it (a Duplicate) still binds it to where that one points.
        let others: Vec<Use> = stored
            .iter()
            .filter(|u| own != Some(u.bucket.as_str()))
            .cloned()
            .collect();
        if own_same_field && others.is_empty() {
            if MAY_FOLLOW.contains(&field) {
                continue;
            }
            return Err(format!(
                "`{field}`: {what} is sent in clear (or as a bearer credential) to where this \
                 bucket points; it cannot follow the bucket to another provider, endpoint or \
                 account. Enter the value again (as a new secret) for the new destination"
            ));
        }
        if own_same_field {
            return Err(format!(
                "`{field}`: {what} is shared with bucket `{}`, which still sends it where it \
                 points; enter a new value (a new secret) for this bucket's new destination",
                names(&others)
            ));
        }
        return Err(format!(
            "`{field}`: {what} belongs to bucket `{}` and is only sent where that bucket sends \
             it, in the same field; enter the value itself to use it elsewhere",
            names(stored)
        ));
    }
    if let Some(key) = ambient_key(candidate) {
        let target = destination(candidate, AMBIENT_FIELD);
        let own = if key == "ambient:aws" {
            "aws"
        } else {
            "google"
        };
        let allowed = target == own
            || uses
                .get(key)
                .is_some_and(|stored| stored.iter().any(|u| u.destination == target));
        if !allowed {
            let (service, fields) = if key == "ambient:aws" {
                ("AWS credentials", "access_key_id and secret_access_key")
            } else {
                ("Google credentials", "credentials_json or credentials_file")
            };
            return Err(format!(
                "no credentials given: the engine host's own {service} are only sent to the \
                 provider itself or to an endpoint a configured bucket already uses them with; \
                 set {fields}"
            ));
        }
    }
    Ok(())
}

/// Values kept from the stored entry `name` (restored `"__redacted__"`,
/// unchanged `${VAR}` templates) only go to that entry's provider and
/// destination.
pub fn check_kept(
    candidate: &Value,
    stored: &Value,
    name: &str,
    kept: &[String],
) -> Result<(), String> {
    for field in kept {
        if !CREDENTIAL_PATHS.contains(&field.as_str()) {
            continue;
        }
        if provider(candidate) != provider(stored)
            || destination(candidate, field) != destination(stored, field)
        {
            return Err(format!(
                "`{field}` keeps bucket `{name}`'s stored value (\"__redacted__\" or its ${{VAR}} \
                 template), but this sends it to a different provider, endpoint or account; \
                 re-enter the value to use it there"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn check_bound_test(candidate: &Value, uses: &HashMap<String, Vec<Use>>) -> Result<(), String> {
        check_bound(candidate, uses, None)
    }

    #[test]
    fn a_saved_bucket_takes_only_its_signing_keys_to_a_new_destination() {
        let stored = json!({
            "mine": { "provider": "s3", "region": "r", "endpoint_url": "http://10.0.0.1:9000",
                      "access_key_id": "secret://MINE_AK", "secret_access_key": "secret://MINE_SK",
                      "session_token": "secret://MINE_TOK" },
            "gcs": { "provider": "gcs", "endpoint_url": "http://fake:4443", "credentials_json": "secret://MINE_JSON" }
        })
        .as_object()
        .unwrap()
        .clone();
        let uses = uses(&stored);
        let moved = |extra: Value| {
            let mut v = json!({ "provider": "s3", "region": "r", "endpoint_url": "http://10.0.0.2:9000",
                                "access_key_id": "secret://MINE_AK", "secret_access_key": "secret://MINE_SK" });
            for (k, x) in extra.as_object().unwrap() {
                v[k] = x.clone();
            }
            v
        };
        // (b) the keys follow "my MinIO moved", in the same fields.
        check_bound(&moved(json!({})), &uses, Some("mine")).unwrap();
        // (c) a session token does not.
        let err = check_bound(
            &moved(json!({ "session_token": "secret://MINE_TOK" })),
            &uses,
            Some("mine"),
        )
        .unwrap_err();
        assert!(
            err.contains("`session_token`") && err.contains("Enter the value again"),
            "{err}"
        );
        // (a) never across fields, own bucket included, even in place.
        let swapped = json!({ "provider": "s3", "region": "r", "endpoint_url": "http://10.0.0.1:9000",
                              "access_key_id": "secret://MINE_SK", "secret_access_key": "secret://MINE_AK" });
        let err = check_bound(&swapped, &uses, Some("mine")).unwrap_err();
        assert!(err.contains("in the same field"), "{err}");
        // (c) credentials_json does not follow a moved gcs endpoint…
        let gcs = json!({ "provider": "gcs", "endpoint_url": "http://127.0.0.1:49262", "credentials_json": "secret://MINE_JSON" });
        assert!(check_bound(&gcs, &uses, Some("gcs")).is_err());
        // A value shared with another bucket (a Duplicate) stays with it.
        let mut shared = stored.clone();
        shared.insert(
            "dup".into(),
            json!({ "provider": "s3", "region": "r", "endpoint_url": "http://10.0.0.1:9000",
                    "access_key_id": "secret://MINE_AK", "secret_access_key": "secret://OTHER" }),
        );
        let shared_uses = super::uses(&shared);
        let err = check_bound(&moved(json!({})), &shared_uses, Some("mine")).unwrap_err();
        assert!(
            err.contains("`access_key_id`") && err.contains("shared with bucket `dup`"),
            "{err}"
        );
        let dup_moved = json!({ "provider": "s3", "region": "r", "endpoint_url": "http://127.0.0.1:49262",
                                "access_key_id": "secret://MINE_AK", "secret_access_key": "secret://OTHER" });
        let err = check_bound(&dup_moved, &shared_uses, Some("dup")).unwrap_err();
        assert!(err.contains("shared with bucket `mine`"), "{err}");
        // …and another bucket never takes them anywhere new.
        assert!(check_bound(&moved(json!({})), &uses, Some("other")).is_err());
        assert!(check_bound(&moved(json!({})), &uses, None).is_err());
    }

    fn stored() -> Map<String, Value> {
        json!({
            "prod": { "provider": "s3", "region": "r", "endpoint_url": "https://S3.prod.example:443",
                      "access_key_id": "AKIA", "secret_access_key": "secret://PROD_SK" },
            "cf": { "provider": "r2", "account_id": "secret://CF_ACCOUNT", "access_key_id": "env://CF_AK",
                    "secret_access_key": "secret://CF_SK",
                    "notifications": { "queue_id": "q", "api_token": "secret://CF_TOKEN" } },
            "docs": { "provider": "gcs", "credentials_file": "/etc/iii/../iii/sa.json" },
            "broken": { "provider": "s3", "endpoint_url": "ftp://x", "secret_access_key": "secret://BROKEN_SK" },
            "ambient": { "provider": "s3", "region": "r", "endpoint_url": "http://minio.lan:9000" }
        })
        .as_object()
        .unwrap()
        .clone()
    }

    fn s3(endpoint: &str, field: &str, value: &str) -> Value {
        let mut v = json!({ "provider": "s3", "region": "r", "endpoint_url": endpoint,
                            "access_key_id": "x", "secret_access_key": "secret://FRESH" });
        v[field] = json!(value);
        v
    }

    #[test]
    fn a_bound_reference_cannot_move_through_any_field() {
        let uses = uses(&stored());
        let evil = "http://127.0.0.1:49262";
        for field in ["secret_access_key", "access_key_id", "session_token"] {
            let err = check_bound_test(&s3(evil, field, "secret://PROD_SK"), &uses).unwrap_err();
            assert!(err.contains("belongs to bucket `prod`"), "{field}: {err}");
        }
        // The same field toward the same endpoint (a Duplicate) is fine;
        // the endpoint compares by scheme, host and port.
        check_bound(
            &s3(
                "https://s3.prod.example",
                "secret_access_key",
                "secret://PROD_SK",
            ),
            &uses,
            None,
        )
        .unwrap();
        // Same endpoint, another field: refused (the id is sent in clear).
        assert!(check_bound(
            &s3(
                "https://s3.prod.example",
                "access_key_id",
                "secret://PROD_SK"
            ),
            &uses,
            None
        )
        .is_err());
        // A reference nobody uses is free.
        check_bound_test(&s3(evil, "session_token", "secret://NEW"), &uses).unwrap();
    }

    #[test]
    fn entries_that_do_not_parse_still_bind_and_files_bind_by_their_real_path() {
        let uses = uses(&stored());
        let err = check_bound(
            &s3("http://127.0.0.1:1", "session_token", "secret://BROKEN_SK"),
            &uses,
            None,
        )
        .unwrap_err();
        assert!(err.contains("`broken`"), "{err}");
        let gcs = |file: &str, endpoint: Option<&str>| {
            let mut v = json!({ "provider": "gcs", "credentials_file": file });
            if let Some(e) = endpoint {
                v["endpoint_url"] = json!(e);
            }
            v
        };
        let err = check_bound(
            &gcs("/etc/iii/sa.json", Some("http://127.0.0.1:49262")),
            &uses,
            None,
        )
        .unwrap_err();
        assert!(
            err.contains("credentials file") && !err.contains("/etc"),
            "{err}"
        );
        assert!(check_bound_test(&gcs("/etc/./iii/sa.json", Some("http://evil")), &uses).is_err());
        check_bound_test(&gcs("/etc/iii/sa.json", None), &uses).unwrap();
        // A host file no bucket uses only goes to Google.
        let err = check_bound(
            &gcs("/var/other-sa.json", Some("http://127.0.0.1:49262")),
            &uses,
            None,
        )
        .unwrap_err();
        assert!(err.contains("only goes to Google"), "{err}");
        check_bound_test(&gcs("/var/other-sa.json", None), &uses).unwrap();
    }

    #[test]
    fn r2_account_ids_and_api_tokens_are_bound() {
        let uses = uses(&stored());
        // The account id becomes a hostname: another account is another
        // destination for every r2 field.
        let r2 = json!({ "provider": "r2", "account_id": "secret://CF_SK", "access_key_id": "a",
                         "secret_access_key": "secret://FRESH" });
        assert!(check_bound_test(&r2, &uses).is_err());
        let r2 = json!({ "provider": "r2", "account_id": "secret://CF_ACCOUNT", "access_key_id": "env://CF_AK",
                         "secret_access_key": "secret://CF_SK" });
        check_bound_test(&r2, &uses).unwrap();
        // The API token only ever goes to the Cloudflare API, in its field.
        let s3_with_token = s3("http://evil", "secret_access_key", "secret://CF_TOKEN");
        assert!(check_bound_test(&s3_with_token, &uses).is_err());
    }

    #[test]
    fn ambient_credentials_stay_with_the_provider_or_a_known_endpoint() {
        let uses = uses(&stored());
        let bare = |endpoint: Option<&str>| {
            let mut v = json!({ "provider": "s3", "region": "r" });
            if let Some(e) = endpoint {
                v["endpoint_url"] = json!(e);
            }
            v
        };
        check_bound_test(&bare(None), &uses).unwrap();
        check_bound_test(&bare(Some("http://MINIO.lan:9000")), &uses).unwrap();
        let err = check_bound_test(&bare(Some("http://127.0.0.1:49262")), &uses).unwrap_err();
        assert!(err.contains("engine host's own AWS credentials"), "{err}");
        let adc = json!({ "provider": "gcs", "endpoint_url": "http://127.0.0.1:49262" });
        assert!(check_bound_test(&adc, &uses)
            .unwrap_err()
            .contains("Google credentials"));
        check_bound_test(&json!({ "provider": "gcs" }), &uses).unwrap();
        // A save of the ambient bucket itself keeps its endpoint.
        check_bound(&bare(Some("http://minio.lan:9000")), &uses, Some("ambient")).unwrap();
    }

    #[test]
    fn kept_values_stay_with_their_entry() {
        let stored = json!({ "provider": "s3", "region": "r", "endpoint_url": "https://h",
                             "session_token": "lit" });
        let moved = json!({ "provider": "s3", "region": "r", "endpoint_url": "http://h",
                            "session_token": "lit" });
        assert!(check_kept(&moved, &stored, "b", &["session_token".into()]).is_err());
        check_kept(&stored, &stored, "b", &["session_token".into()]).unwrap();
        let other_provider = json!({ "provider": "r2", "endpoint_url": "https://h", "account_id": "a",
                                     "access_key_id": "a", "secret_access_key": "lit" });
        assert!(check_kept(&other_provider, &stored, "b", &["secret_access_key".into()]).is_err());
        // A kept non-credential (a region template) is not bound.
        check_kept(&moved, &stored, "b", &["region".into()]).unwrap();
    }
}
