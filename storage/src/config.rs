//! Configuration parsing for the storage worker.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct WorkerConfig {
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub buckets: HashMap<String, BucketConfig>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct ProvidersConfig {
    pub local: Option<LocalProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LocalProviderConfig {
    #[serde(default = "default_local_data_dir")]
    pub data_dir: String,
    /// Optional direct-transfer HTTP server. Inline getObject/putObject work
    /// without it; signed upload/download endpoints require it.
    pub http: Option<LocalHttpConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LocalHttpConfig {
    /// Socket the worker listens on. Use `0.0.0.0` to expose it to a LAN/VPN.
    #[serde(default = "default_local_http_bind_address")]
    pub bind_address: String,
    /// Browser-facing base URL. This may differ from the listener when a VPN,
    /// reverse proxy, or another machine provides access.
    pub public_url: Option<String>,
}

fn default_local_data_dir() -> String {
    "./data/storage".to_string()
}

fn default_local_http_bind_address() -> String {
    "127.0.0.1:0".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "lowercase")]
pub enum BucketConfig {
    S3(S3BucketConfig),
    Gcs(GcsBucketConfig),
    R2(R2BucketConfig),
    Local(LocalBucketConfig),
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct S3BucketConfig {
    pub bucket: Option<String>,
    pub region: String,
    pub access_key_id: Option<String>,
    pub secret_access_key: Option<String>,
    pub session_token: Option<String>,
    /// When set, overrides the AWS-default endpoint. Use for self-hosted
    /// S3-compatible stores (MinIO, Ceph, SeaweedFS) or local testing.
    pub endpoint_url: Option<String>,
    /// Force path-style addressing (`http://host/bucket/key`) instead of
    /// virtual-hosted style (`http://bucket.host/key`). Required for most
    /// S3-compatible stores (MinIO, Ceph, SeaweedFS, LocalStack). Defaults
    /// to false to preserve current AWS behavior.
    #[serde(default)]
    pub force_path_style: Option<bool>,
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
            .field("force_path_style", &self.force_path_style)
            .field("notifications", &self.notifications)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct S3Notifications {
    pub sqs_queue_url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GcsBucketConfig {
    pub bucket: Option<String>,
    pub credentials_file: Option<String>,
    /// When set, overrides the `https://storage.googleapis.com` default.
    /// Used by fake-gcs-server in the e2e harness; safe for any
    /// custom-endpoint GCS-compatible deployment.
    pub endpoint_url: Option<String>,
    pub notifications: Option<GcsNotifications>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GcsNotifications {
    pub pubsub_subscription: String,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
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

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
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

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LocalBucketConfig {
    pub bucket: Option<String>,
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml)?;
        let cfg: WorkerConfig =
            serde_yaml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
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
        self.validate_bucket_names()
    }

    /// Per-bucket-name validation only — no "at least one bucket" requirement.
    /// The live configuration (fetched from the configuration worker) and the
    /// built-in default may legitimately declare zero buckets on a fresh
    /// install, in which case the worker runs with no backends until the
    /// operator configures one.
    fn validate_bucket_names(&self) -> Result<(), String> {
        for (name, bucket) in &self.buckets {
            validate_bucket_name(name).map_err(|e| format!("bucket `{name}`: {e}"))?;
            if let BucketConfig::Local(local) = bucket {
                if let Some(underlying) = local.bucket.as_deref() {
                    validate_bucket_name(underlying)
                        .map_err(|e| format!("bucket `{name}` local bucket `{underlying}`: {e}"))?;
                }
            }
        }
        if let Some(http) = self
            .providers
            .local
            .as_ref()
            .and_then(|local| local.http.as_ref())
        {
            http.bind_address
                .parse::<std::net::SocketAddr>()
                .map_err(|e| {
                    format!(
                        "providers.local.http.bind_address `{}` is not a socket address: {e}",
                        http.bind_address
                    )
                })?;
            if let Some(public_url) = http.public_url.as_deref() {
                let parsed = url::Url::parse(public_url).map_err(|e| {
                    format!("providers.local.http.public_url `{public_url}` is invalid: {e}")
                })?;
                if !matches!(parsed.scheme(), "http" | "https") {
                    return Err(format!(
                        "providers.local.http.public_url `{public_url}` must use http or https"
                    ));
                }
                if parsed.query().is_some() || parsed.fragment().is_some() {
                    return Err(
                        "providers.local.http.public_url must not contain a query or fragment"
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }

    /// Parse a config from a JSON value already env-expanded by the
    /// configuration worker. Unlike [`from_yaml`], zero buckets is allowed.
    pub fn from_json(value: &Value) -> Result<Self, String> {
        let cfg: WorkerConfig =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        cfg.validate_bucket_names()?;
        Ok(cfg)
    }

    pub fn to_json(&self) -> Value {
        serde_json::to_value(self).expect("WorkerConfig serializes")
    }

    pub fn json_schema() -> Value {
        let root = schemars::schema_for!(WorkerConfig);
        let mut schema =
            serde_json::to_value(&root.schema).expect("WorkerConfig JSON Schema serializes");
        if let Some(obj) = schema.as_object_mut() {
            if !root.definitions.is_empty() {
                obj.insert(
                    "definitions".into(),
                    serde_json::to_value(&root.definitions).expect("definitions serialize"),
                );
            }
        }
        schema
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

/// Load and validate a `WorkerConfig` from `path`.
///
/// Free-function alias for [`WorkerConfig::from_file`] that returns
/// [`anyhow::Result`], matching the binary-worker spec (`docs/sops/binary-worker.md`
/// §5). Errors carry the file path as context.
pub fn load_config(path: &str) -> anyhow::Result<WorkerConfig> {
    WorkerConfig::from_file(path)
        .map_err(|e| anyhow::anyhow!(e))
        .map_err(|e| e.context(format!("loading config from {path}")))
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
        assert_eq!(c.providers.local.as_ref().unwrap().data_dir, "/tmp/storage");
    }

    #[test]
    fn local_http_config_parses_network_public_url() {
        let yaml = r#"
providers:
  local:
    data_dir: /tmp/storage
    http:
      bind_address: 0.0.0.0:49200
      public_url: https://storage.vpn.example
buckets:
  scratch:
    provider: local
"#;
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        let http = cfg
            .providers
            .local
            .as_ref()
            .and_then(|local| local.http.as_ref())
            .unwrap();
        assert_eq!(http.bind_address, "0.0.0.0:49200");
        assert_eq!(
            http.public_url.as_deref(),
            Some("https://storage.vpn.example")
        );
    }

    #[test]
    fn local_http_rejects_invalid_listener_and_public_url() {
        let invalid_listener = WorkerConfig::from_yaml(
            "providers:\n  local:\n    data_dir: /tmp/storage\n    http:\n      bind_address: localhost:49200\nbuckets:\n  scratch:\n    provider: local\n",
        )
        .unwrap_err();
        assert!(invalid_listener.contains("bind_address"));

        let invalid_public = WorkerConfig::from_yaml(
            "providers:\n  local:\n    data_dir: /tmp/storage\n    http:\n      bind_address: 127.0.0.1:0\n      public_url: ftp://example.test\nbuckets:\n  scratch:\n    provider: local\n",
        )
        .unwrap_err();
        assert!(invalid_public.contains("http or https"));
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

    #[test]
    fn to_json_from_json_roundtrips() {
        let yaml =
            "buckets:\n  uploads:\n    provider: s3\n    bucket: my-app\n    region: us-east-1\n";
        let cfg = WorkerConfig::from_yaml(yaml).unwrap();
        let json = cfg.to_json();
        let back = WorkerConfig::from_json(&json).unwrap();
        match &back.buckets["uploads"] {
            BucketConfig::S3(s) => {
                assert_eq!(s.region, "us-east-1");
                assert_eq!(s.bucket.as_deref(), Some("my-app"));
            }
            other => panic!("expected S3, got {other:?}"),
        }
    }

    #[test]
    fn from_json_tolerates_zero_buckets() {
        let back = WorkerConfig::from_json(&serde_json::json!({ "buckets": {} })).unwrap();
        assert!(back.buckets.is_empty());
    }

    #[test]
    fn default_serializes_and_reparses_as_empty() {
        let json = WorkerConfig::default().to_json();
        let back = WorkerConfig::from_json(&json).unwrap();
        assert!(back.buckets.is_empty());
    }

    #[test]
    fn from_json_still_validates_bucket_names() {
        let err = WorkerConfig::from_json(&serde_json::json!({
            "buckets": { "Bad Name": { "provider": "local" } }
        }))
        .unwrap_err();
        assert!(err.contains("Bad Name"), "got: {err}");
    }

    #[test]
    fn from_yaml_still_requires_at_least_one_bucket() {
        let err = WorkerConfig::from_yaml("buckets: {}\n").unwrap_err();
        assert!(err.contains("at least one bucket"), "got: {err}");
    }

    #[test]
    fn json_schema_has_buckets_property() {
        let schema = WorkerConfig::json_schema();
        assert!(schema
            .get("properties")
            .and_then(|p| p.get("buckets"))
            .is_some());
    }
}
