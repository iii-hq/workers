//! Configuration parsing for the storage worker.

use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub buckets: HashMap<String, BucketConfig>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProvidersConfig {
    pub local: Option<LocalProviderConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalProviderConfig {
    #[serde(default = "default_local_data_dir")]
    pub data_dir: String,
}

fn default_local_data_dir() -> String {
    "./data/storage".to_string()
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum BucketConfig {
    S3(S3BucketConfig),
    Gcs(GcsBucketConfig),
    R2(R2BucketConfig),
    Local(LocalBucketConfig),
}

#[derive(Clone, Deserialize)]
pub struct S3BucketConfig {
    pub bucket: Option<String>,
    pub region: String,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    /// When set, overrides the AWS-default endpoint. Use for self-hosted
    /// S3-compatible stores (MinIO, Ceph, SeaweedFS) or local testing.
    pub endpoint_url: Option<String>,
    pub notifications: Option<S3Notifications>,
}

impl std::fmt::Debug for S3BucketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("S3BucketConfig")
            .field("bucket", &self.bucket)
            .field("region", &self.region)
            .field("access_key_id", &self.access_key_id)
            .field(
                "secret_access_key",
                &self.secret_access_key.as_deref().map(redact_secret),
            )
            .field(
                "session_token",
                &self.session_token.as_deref().map(redact_secret),
            )
            .field("endpoint_url", &self.endpoint_url)
            .field("notifications", &self.notifications)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct S3Notifications {
    pub sqs_queue_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GcsBucketConfig {
    pub bucket: Option<String>,
    pub credentials_file: Option<String>,
    /// When set, overrides the `https://storage.googleapis.com` default.
    /// Used by fake-gcs-server in the e2e harness; safe for any
    /// custom-endpoint GCS-compatible deployment.
    pub endpoint_url: Option<String>,
    pub notifications: Option<GcsNotifications>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct GcsNotifications {
    pub pubsub_subscription: String,
}

#[derive(Clone, Deserialize)]
pub struct R2BucketConfig {
    pub bucket: Option<String>,
    pub account_id: String,
    pub access_key_id: String,
    pub secret_access_key: String,
    /// When set, overrides the `<account_id>.r2.cloudflarestorage.com`
    /// default. For testing against MinIO or a private R2-compatible store.
    /// Emits a tracing::warn! at startup so accidental production use is
    /// visible in logs.
    pub endpoint_url: Option<String>,
    pub notifications: Option<R2Notifications>,
}

impl std::fmt::Debug for R2BucketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("R2BucketConfig")
            .field("bucket", &self.bucket)
            .field("account_id", &self.account_id)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &redact_secret(&self.secret_access_key))
            .field("endpoint_url", &self.endpoint_url)
            .field("notifications", &self.notifications)
            .finish()
    }
}

#[derive(Clone, Deserialize)]
pub struct R2Notifications {
    pub queue_id: String,
    pub api_token: String,
}

impl std::fmt::Debug for R2Notifications {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("R2Notifications")
            .field("queue_id", &self.queue_id)
            .field("api_token", &redact_secret(&self.api_token))
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct LocalBucketConfig {
    pub bucket: Option<String>,
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml)?;
        let cfg: WorkerConfig =
            serde_yml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        cfg.validate()?;
        Ok(cfg)
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.buckets.is_empty() {
            return Err("config must declare at least one bucket".into());
        }
        // Note: a `provider: local` bucket without `providers.local` is valid;
        // the default data_dir is materialised at backend init time, not here.
        // Trigger ↔ bucket cross-validation runs at trigger registration time
        // (handler.rs) — the worker config never sees the trigger spec since
        // triggers are registered dynamically via the SDK.
        for name in self.buckets.keys() {
            validate_bucket_name(name).map_err(|e| format!("bucket `{name}`: {e}"))?;
        }
        Ok(())
    }
}

/// Validate a worker-facing bucket name. Matches a tightened S3 bucket naming
/// subset: starts with `[a-z0-9]`, body in `[a-z0-9_-]`, max 63 chars.
pub fn validate_bucket_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("bucket name is empty".into());
    }
    if name.len() > 63 {
        return Err(format!("bucket name `{name}` exceeds 63 characters"));
    }
    let mut chars = name.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_lowercase() || first.is_ascii_digit()) {
        return Err(format!("bucket name `{name}` must start with [a-z0-9]"));
    }
    for c in chars {
        if !(c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-') {
            return Err(format!(
                "bucket name `{name}` contains invalid character `{c}` (only [a-z0-9_-] allowed)"
            ));
        }
    }
    Ok(())
}

/// Expand `${NAME}` against the process environment. Returns `Err` listing every
/// referenced env var that isn't set, so config load fails up-front instead of
/// silently producing empty credentials that the SDK rejects later with a
/// confusing auth error.
fn expand_env(input: &str) -> Result<String, String> {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    let mut missing: Vec<String> = Vec::new();
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        // Track and continue so we report every missing var in a
                        // single error rather than fail-fast on the first one.
                        if !missing.iter().any(|n| n == name) {
                            missing.push(name.to_string());
                        }
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    if missing.is_empty() {
        Ok(out)
    } else {
        Err(format!(
            "config references env var(s) not set: {}",
            missing.join(", ")
        ))
    }
}

/// Best-effort URL redaction — strips userinfo from parseable URLs, leaves
/// the rest unchanged. Used for any provider URL we print at any log level.
pub fn redact_url(input: &str) -> String {
    use url::Url;
    if let Ok(parsed) = Url::parse(input) {
        let mut redacted = parsed;
        let _ = redacted.set_password(None);
        if !redacted.username().is_empty() {
            let _ = redacted.set_username("");
        }
        return redacted.into();
    }
    input.to_string()
}

/// Mask a secret value for log output. Preserves length so operators can spot
/// a truncated env var, never the value itself.
pub fn redact_secret(s: &str) -> String {
    format!("***[{}]***", s.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_local_bucket() {
        let yaml = r#"
buckets:
  scratch:
    provider: local
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        assert_eq!(c.buckets.len(), 1);
        match &c.buckets["scratch"] {
            BucketConfig::Local(_) => {}
            other => panic!("expected Local, got {other:?}"),
        }
    }

    #[test]
    fn parses_s3_bucket() {
        let yaml = r#"
buckets:
  uploads:
    provider: s3
    bucket: my-app-uploads
    region: us-east-1
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        match &c.buckets["uploads"] {
            BucketConfig::S3(s) => {
                assert_eq!(s.bucket.as_deref(), Some("my-app-uploads"));
                assert_eq!(s.region, "us-east-1");
            }
            other => panic!("expected S3, got {other:?}"),
        }
    }

    #[test]
    fn s3_bucket_config_parses_endpoint_url() {
        let yaml = r#"
buckets:
  uploads:
    provider: s3
    region: us-east-1
    endpoint_url: http://127.0.0.1:9000
    bucket: scratch
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        match &c.buckets["uploads"] {
            BucketConfig::S3(s) => {
                assert_eq!(s.endpoint_url.as_deref(), Some("http://127.0.0.1:9000"));
            }
            other => panic!("expected S3, got {other:?}"),
        }
    }

    #[test]
    fn parses_r2_bucket_with_required_fields() {
        let yaml = r#"
buckets:
  avatars:
    provider: r2
    account_id: acc123
    access_key_id: key
    secret_access_key: secret
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        match &c.buckets["avatars"] {
            BucketConfig::R2(r) => {
                assert_eq!(r.account_id, "acc123");
            }
            other => panic!("expected R2, got {other:?}"),
        }
    }

    #[test]
    fn r2_bucket_config_parses_endpoint_url() {
        let yaml = r#"
buckets:
  scratch-r2:
    provider: r2
    account_id: fake
    access_key_id: ak
    secret_access_key: sk
    endpoint_url: http://minio.local:9000
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        match &c.buckets["scratch-r2"] {
            BucketConfig::R2(r) => {
                assert_eq!(r.endpoint_url.as_deref(), Some("http://minio.local:9000"));
            }
            other => panic!("expected R2, got {other:?}"),
        }
    }

    #[test]
    fn parses_gcs_bucket() {
        let yaml = r#"
buckets:
  documents:
    provider: gcs
    bucket: my-app-documents
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        assert!(matches!(c.buckets["documents"], BucketConfig::Gcs(_)));
    }

    #[test]
    fn gcs_bucket_config_parses_endpoint_url() {
        let yaml = r#"
buckets:
  scratch-gcs:
    provider: gcs
    endpoint_url: http://127.0.0.1:4443
    credentials_file: ./fixtures/gcs-fake-creds.json
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        match &c.buckets["scratch-gcs"] {
            BucketConfig::Gcs(g) => {
                assert_eq!(g.endpoint_url.as_deref(), Some("http://127.0.0.1:4443"));
            }
            other => panic!("expected GCS, got {other:?}"),
        }
    }

    #[test]
    fn empty_buckets_block_errors() {
        let err = WorkerConfig::from_yaml("buckets: {}\n").unwrap_err();
        assert!(err.contains("at least one bucket"), "got: {err}");
    }

    #[test]
    fn unknown_provider_errors() {
        let err = WorkerConfig::from_yaml("buckets:\n  x:\n    provider: ftp\n").unwrap_err();
        assert!(
            err.contains("ftp") || err.contains("unknown") || err.contains("variant"),
            "got: {err}"
        );
    }

    #[test]
    fn r2_missing_required_field_errors() {
        // Missing access_key_id — serde should reject.
        let err = WorkerConfig::from_yaml(
            "buckets:\n  x:\n    provider: r2\n    account_id: a\n    secret_access_key: s\n",
        )
        .unwrap_err();
        assert!(err.contains("access_key_id"), "got: {err}");
    }

    #[test]
    fn local_data_dir_defaults_when_absent() {
        let yaml = r#"
buckets:
  scratch:
    provider: local
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        // providers.local may be absent; the default is materialised at backend-build time.
        assert!(c.providers.local.is_none());
    }

    #[test]
    fn local_data_dir_explicit() {
        let yaml = r#"
providers:
  local:
    data_dir: /tmp/storage
buckets:
  scratch:
    provider: local
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        assert_eq!(
            c.providers.local.as_ref().unwrap().data_dir,
            "/tmp/storage"
        );
    }

    #[test]
    fn env_var_expansion_in_strings() {
        std::env::set_var("IIISTORE_TEST_REGION", "eu-west-1");
        let yaml = r#"
buckets:
  uploads:
    provider: s3
    bucket: ub
    region: "${IIISTORE_TEST_REGION}"
"#;
        let c = WorkerConfig::from_yaml(yaml).unwrap();
        match &c.buckets["uploads"] {
            BucketConfig::S3(s) => assert_eq!(s.region, "eu-west-1"),
            _ => panic!(),
        }
        std::env::remove_var("IIISTORE_TEST_REGION");
    }


    #[test]
    fn missing_env_var_fails_config_load() {
        // Make sure the var really isn't in the environment.
        std::env::remove_var("IIISTORE_TEST_DEFINITELY_UNSET_VAR");
        let yaml = r#"
buckets:
  uploads:
    provider: s3
    bucket: ub
    region: "${IIISTORE_TEST_DEFINITELY_UNSET_VAR}"
"#;
        let err = WorkerConfig::from_yaml(yaml).unwrap_err();
        assert!(
            err.contains("IIISTORE_TEST_DEFINITELY_UNSET_VAR"),
            "got: {err}"
        );
        assert!(err.contains("not set"), "got: {err}");
    }

    #[test]
    fn validate_bucket_name_accepts_normal_names() {
        assert!(validate_bucket_name("uploads").is_ok());
        assert!(validate_bucket_name("user_avatars").is_ok());
        assert!(validate_bucket_name("logs-2024").is_ok());
        assert!(validate_bucket_name("0a").is_ok());
    }

    #[test]
    fn validate_bucket_name_rejects_uppercase_and_punctuation() {
        assert!(validate_bucket_name("Uploads").is_err());
        assert!(validate_bucket_name("a.b").is_err());
        assert!(validate_bucket_name("a b").is_err());
        assert!(validate_bucket_name("").is_err());
        assert!(validate_bucket_name(&"a".repeat(64)).is_err());
        assert!(validate_bucket_name("-leading").is_err());
        assert!(validate_bucket_name("_leading").is_err());
    }

    #[test]
    fn redact_url_strips_userinfo() {
        assert_eq!(
            redact_url("https://AKIA:secret@s3.amazonaws.com/x"),
            "https://s3.amazonaws.com/x"
        );
    }

    #[test]
    fn redact_url_passthrough_on_unparsable() {
        assert_eq!(redact_url("not a url"), "not a url");
    }

    #[test]
    fn redact_secret_replaces_value_with_masked_string() {
        assert_eq!(redact_secret("AKIAIOSFODNN7EXAMPLE"), "***[20]***");
        assert_eq!(redact_secret(""), "***[0]***");
    }
}
