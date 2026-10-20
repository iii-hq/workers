//! From a configured bucket entry to the values its provider client needs.
//!
//! [`Material`] is an entry's credential fields with `secret://` / `env://`
//! references resolved. It lives only as long as a build (the provider
//! clients keep their own copies), and every error names the field and the
//! reference — never a value.
//!
//! [`Stage`] says where using a bucket failed: building it (`config`,
//! `secrets`, `auth`) or, for `storage::testBucket`, reaching it
//! (`endpoint`, `auth`, `bucket`, `list`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};

use schemars::JsonSchema;
use serde::Serialize;

use crate::backend::BackendError;
use crate::config::BucketConfig;
use crate::secrets::{parse_ref, SecretError, SecretValue};

/// Where using a bucket failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// The entry itself is unusable (a bad field, an unreadable file).
    Config,
    /// A `secret://` / `env://` reference did not resolve.
    Secrets,
    /// The provider could not be reached: DNS, connect, TLS, timeout.
    Endpoint,
    /// The provider refused the credentials or the signature.
    Auth,
    /// Reached and authenticated, but the bucket is missing or elsewhere.
    Bucket,
    /// Any other failure listing the bucket.
    List,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Config => "config",
            Self::Secrets => "secrets",
            Self::Endpoint => "endpoint",
            Self::Auth => "auth",
            Self::Bucket => "bucket",
            Self::List => "list",
        }
    }
}

/// A bucket failure that is safe to show: it names fields, references,
/// hosts and buckets, never a key, token or credentials JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BucketError {
    pub stage: Stage,
    pub message: String,
}

impl BucketError {
    pub fn new(stage: Stage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }

    /// A failed backend build: rejected credentials are `auth`, the rest
    /// `config`. The message is scrubbed of every credential the build used.
    pub fn from_build(error: BackendError, material: &Material) -> Self {
        let (stage, message) = match error {
            BackendError::AuthFailed(message) => (Stage::Auth, message),
            BackendError::Provider { message, .. } => (Stage::Config, message),
            other => (Stage::Config, other.to_string()),
        };
        Self::new(stage, material.scrub(&message))
    }
}

impl std::fmt::Display for BucketError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// The canonical `secret://` / `env://` references an entry uses.
pub fn references(bucket: &BucketConfig) -> Vec<String> {
    bucket
        .credential_fields()
        .into_iter()
        .filter_map(|(_, v)| parse_ref(v).and_then(Result::ok).map(|r| r.key()))
        .collect()
}

/// An entry's credential fields with references resolved.
pub struct Material {
    values: HashMap<&'static str, SecretValue>,
}

impl std::fmt::Debug for Material {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut fields: Vec<_> = self.values.keys().collect();
        fields.sort();
        f.debug_struct("Material").field("fields", &fields).finish()
    }
}

impl Material {
    /// Resolve through `lookup` (the worker's [`crate::secrets::SecretCache`],
    /// or a fresh resolution for a probe). A reference `lookup` has no
    /// outcome for counts as unavailable.
    pub fn resolve(
        bucket: &BucketConfig,
        lookup: impl Fn(&str) -> Option<Result<SecretValue, SecretError>>,
    ) -> Result<Self, BucketError> {
        let mut values = HashMap::new();
        for (field, raw) in bucket.credential_fields() {
            let value = match parse_ref(raw) {
                None => SecretValue::new(raw.to_string()),
                Some(Err(e)) => {
                    return Err(BucketError::new(
                        Stage::Secrets,
                        format!("`{field}` {}", e.describe_malformed()),
                    ))
                }
                Some(Ok(reference)) => {
                    let key = reference.key();
                    match lookup(&key).unwrap_or(Err(SecretError::Unavailable)) {
                        Ok(value) => value,
                        Err(e) => {
                            return Err(BucketError::new(
                                Stage::Secrets,
                                format!("`{field}`: {}", e.describe(&key)),
                            ))
                        }
                    }
                }
            };
            values.insert(field, value);
        }
        Ok(Self { values })
    }

    /// Literal values only; any reference fails (tests, offline tools).
    pub fn literal(bucket: &BucketConfig) -> Result<Self, BucketError> {
        Self::resolve(bucket, |_| None)
    }

    /// No credential fields at all (a local bucket, the AWS default chain).
    pub fn empty() -> Self {
        Self {
            values: HashMap::new(),
        }
    }

    /// The usable value of a credential field (`access_key_id`, …).
    pub fn value(&self, field: &str) -> Option<&str> {
        self.values.get(field).map(SecretValue::expose)
    }

    /// Strip every sensitive value this material holds (and, for a
    /// credentials JSON, its key material) out of a message.
    pub fn scrub(&self, message: &str) -> String {
        let mut out = message.to_string();
        for (field, value) in &self.values {
            let value = value.expose();
            if *field == "credentials_json" {
                for piece in json_secrets(value) {
                    if piece.len() >= 4 {
                        out = out.replace(&piece, "***");
                    }
                }
            }
            // Ids and account ids are not secret, and short values would
            // shred unrelated text.
            if matches!(*field, "access_key_id" | "account_id") || value.len() < 4 {
                continue;
            }
            out = out.replace(value, "***");
        }
        out
    }

    fn digest(&self, hasher: &mut impl Hasher) {
        let mut fields: Vec<_> = self.values.iter().collect();
        fields.sort_by_key(|(field, _)| **field);
        for (field, value) in fields {
            field.hash(hasher);
            value.expose().hash(hasher);
        }
    }
}

/// The string values of a credentials JSON that must never be echoed.
fn json_secrets(raw: &str) -> Vec<String> {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str::<serde_json::Value>(raw) else {
        return Vec::new();
    };
    [
        "private_key",
        "private_key_id",
        "refresh_token",
        "client_secret",
    ]
    .iter()
    .filter_map(|k| map.get(*k).and_then(|v| v.as_str()).map(str::to_string))
    .collect()
}

/// The change-detection hash of a bucket: its name, entry and resolved
/// values. An unchanged fingerprint keeps the running client.
pub fn fingerprint(name: &str, bucket: &BucketConfig, material: &Material) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    serde_json::to_string(bucket)
        .unwrap_or_default()
        .hash(&mut hasher);
    material.digest(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::testing::FakeSecrets;

    fn bucket(v: serde_json::Value) -> BucketConfig {
        serde_json::from_value(v).unwrap()
    }

    #[tokio::test]
    async fn references_resolve_through_the_cache_and_literals_pass_through() {
        let fake = FakeSecrets::default();
        fake.set("secret://SK", Ok("sk-value"));
        let cache = fake.cache();
        let b = bucket(serde_json::json!({
            "provider": "s3", "region": "us-east-1",
            "access_key_id": "AKIA", "secret_access_key": "secret://SK"
        }));
        assert_eq!(references(&b), vec!["secret://SK".to_string()]);
        cache.ensure(references(&b)).await;
        let m = Material::resolve(&b, |k| cache.lookup(k)).unwrap();
        assert_eq!(m.value("access_key_id"), Some("AKIA"));
        assert_eq!(m.value("secret_access_key"), Some("sk-value"));
        assert!(!format!("{m:?}").contains("sk-value"));
    }

    #[test]
    fn an_unresolved_reference_is_a_secrets_failure_naming_field_and_reference() {
        let b = bucket(serde_json::json!({
            "provider": "r2", "account_id": "env://CF_ACCOUNT",
            "access_key_id": "ak", "secret_access_key": "secret://R2_SK"
        }));
        let err = Material::resolve(&b, |k| {
            (k == "env://CF_ACCOUNT").then(|| Ok(SecretValue::new("acc".into())))
        })
        .unwrap_err();
        assert_eq!(err.stage, Stage::Secrets);
        assert!(err.message.contains("`secret_access_key`"), "{err}");
        assert!(err.message.contains("secret://R2_SK"), "{err}");
        let malformed = bucket(serde_json::json!({
            "provider": "gcs", "credentials_json": "secret://has space"
        }));
        let err = Material::literal(&malformed).unwrap_err();
        assert_eq!(err.stage, Stage::Secrets);
        assert!(!err.message.contains("has space"), "{err}");
    }

    #[test]
    fn scrub_hides_secrets_and_credentials_json_key_material() {
        let creds = r#"{"type":"service_account","private_key":"-----BEGIN KEY-----abc","client_email":"a@b"}"#;
        let b = bucket(serde_json::json!({
            "provider": "gcs", "credentials_json": creds
        }));
        let m = Material::literal(&b).unwrap();
        let out = m.scrub("bad key -----BEGIN KEY-----abc in json");
        assert!(!out.contains("BEGIN KEY"), "{out}");
        let s3 = bucket(serde_json::json!({
            "provider": "s3", "region": "x", "access_key_id": "AKIAX", "secret_access_key": "topsecret"
        }));
        let m = Material::literal(&s3).unwrap();
        assert_eq!(
            m.scrub("AKIAX signed with topsecret"),
            "AKIAX signed with ***"
        );
    }

    #[test]
    fn the_fingerprint_follows_entry_and_resolved_values() {
        let b = bucket(serde_json::json!({
            "provider": "s3", "region": "us-east-1", "secret_access_key": "secret://SK", "access_key_id": "a"
        }));
        let with = |v: &str| {
            let v = v.to_string();
            Material::resolve(&b, move |_| Some(Ok(SecretValue::new(v.clone())))).unwrap()
        };
        assert_eq!(
            fingerprint("x", &b, &with("one")),
            fingerprint("x", &b, &with("one"))
        );
        assert_ne!(
            fingerprint("x", &b, &with("one")),
            fingerprint("x", &b, &with("two"))
        );
        assert_ne!(
            fingerprint("x", &b, &with("one")),
            fingerprint("y", &b, &with("one"))
        );
    }
}
