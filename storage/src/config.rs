//! Configuration parsing for the storage worker.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};

/// What `storage::getBucket` shows for a stored literal credential, and what
/// a client sends back to keep it (see `crate::redaction`).
pub const REDACTED: &str = "__redacted__";

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct WorkerConfig {
    #[serde(default)]
    pub providers: ProvidersConfig,
    #[serde(default)]
    pub buckets: HashMap<String, BucketConfig>,
    /// Stored bucket entries that did not parse or validate, by name (only
    /// [`WorkerConfig::from_json_isolating`] fills it). Each one is listed
    /// with its error and answers `BUCKET_UNAVAILABLE`; the other buckets
    /// keep working.
    #[serde(skip)]
    #[schemars(skip)]
    pub invalid: BTreeMap<String, InvalidBucket>,
}

/// A stored bucket entry that could not be used as it is.
#[derive(Debug, Clone)]
pub struct InvalidBucket {
    /// Why, naming fields but never a value.
    pub message: String,
    /// The entry as stored, for the listing's provider / bucket / display.
    pub raw: Value,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct ProvidersConfig {
    pub local: Option<LocalProviderConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct LocalProviderConfig {
    #[serde(default = "default_local_data_dir")]
    pub data_dir: String,
    /// Direct-transfer HTTP server used by signed upload/download endpoints.
    /// When omitted, the worker still starts one on a free loopback port
    /// (`127.0.0.1:0`) so browser transfers work out of the box; set it to
    /// choose the listener or publish a LAN/VPN/proxy `public_url`.
    pub http: Option<LocalHttpConfig>,
}

impl Default for LocalHttpConfig {
    fn default() -> Self {
        Self {
            bind_address: default_local_http_bind_address(),
            public_url: None,
        }
    }
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
    iii_worker_paths::default_path("data/storage")
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
    /// A literal, `secret://NAME` or `env://NAME` (resolved through the
    /// `secrets` worker). Absent with `secret_access_key` = the AWS default
    /// credential chain.
    pub access_key_id: Option<String>,
    /// Sensitive: `secret://NAME` / `env://NAME`, or a literal.
    pub secret_access_key: Option<String>,
    /// Sensitive: `secret://NAME` / `env://NAME`, or a literal.
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
    /// How clients draw the bucket (UI only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<BucketDisplay>,
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
            .field(
                "endpoint_url",
                &self.endpoint_url.as_deref().map(redact_url),
            )
            .field("force_path_style", &self.force_path_style)
            .field("notifications", &self.notifications)
            .field("display", &self.display)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct S3Notifications {
    pub sqs_queue_url: String,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct GcsBucketConfig {
    pub bucket: Option<String>,
    /// A service-account JSON file on the engine host.
    pub credentials_file: Option<String>,
    /// Sensitive: the service-account JSON itself, as `secret://NAME` /
    /// `env://NAME` (or a literal). Takes precedence over
    /// `credentials_file`; neither = Application Default Credentials.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials_json: Option<String>,
    /// When set, overrides the `https://storage.googleapis.com` default.
    /// Used by fake-gcs-server in the e2e harness; safe for any
    /// custom-endpoint GCS-compatible deployment.
    pub endpoint_url: Option<String>,
    pub notifications: Option<GcsNotifications>,
    /// How clients draw the bucket (UI only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<BucketDisplay>,
}

impl std::fmt::Debug for GcsBucketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcsBucketConfig")
            .field("bucket", &self.bucket)
            .field("credentials_file", &self.credentials_file)
            .field(
                "credentials_json",
                &self.credentials_json.as_deref().map(redact_secret),
            )
            .field(
                "endpoint_url",
                &self.endpoint_url.as_deref().map(redact_url),
            )
            .field("notifications", &self.notifications)
            .field("display", &self.display)
            .finish()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct GcsNotifications {
    pub pubsub_subscription: String,
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct R2BucketConfig {
    pub bucket: Option<String>,
    /// A literal, `secret://NAME` or `env://NAME`.
    pub account_id: String,
    /// A literal, `secret://NAME` or `env://NAME`.
    pub access_key_id: String,
    /// Sensitive: `secret://NAME` / `env://NAME`, or a literal.
    pub secret_access_key: String,
    /// When set, overrides the `<account_id>.r2.cloudflarestorage.com`
    /// default. For testing against MinIO or a private R2-compatible store.
    /// Emits a tracing::warn! at startup so accidental production use is
    /// visible in logs.
    pub endpoint_url: Option<String>,
    pub notifications: Option<R2Notifications>,
    /// How clients draw the bucket (UI only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<BucketDisplay>,
}

impl std::fmt::Debug for R2BucketConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("R2BucketConfig")
            .field("bucket", &self.bucket)
            .field("account_id", &self.account_id)
            .field("access_key_id", &self.access_key_id)
            .field("secret_access_key", &redact_secret(&self.secret_access_key))
            .field(
                "endpoint_url",
                &self.endpoint_url.as_deref().map(redact_url),
            )
            .field("notifications", &self.notifications)
            .field("display", &self.display)
            .finish()
    }
}

#[derive(Clone, Deserialize, Serialize, JsonSchema)]
pub struct R2Notifications {
    pub queue_id: String,
    /// Sensitive: `secret://NAME` / `env://NAME`, or a literal.
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
    /// How clients draw the bucket (UI only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<BucketDisplay>,
}

/// How clients draw a bucket. Never read by the worker itself.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
pub struct BucketDisplay {
    /// A Lucide icon name (`hard-drive`, `cloud`, …): `[a-z0-9-]`, at most 64.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

impl BucketDisplay {
    /// Refuse a malformed icon or color, naming the field.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(icon) = &self.icon {
            if !valid_icon(icon) {
                return Err(
                    "`display.icon` must be a Lucide icon name ([a-z0-9-], at most 64)".into(),
                );
            }
        }
        if let Some(color) = &self.color {
            if !valid_color(color) {
                return Err("`display.color` must be #RRGGBB".into());
            }
        }
        Ok(())
    }

    /// The well-formed parts only (a hand-edited entry may hold anything).
    pub fn sanitized(&self) -> Option<Self> {
        let icon = self.icon.clone().filter(|v| valid_icon(v));
        let color = self.color.clone().filter(|v| valid_color(v));
        (icon.is_some() || color.is_some()).then_some(Self { icon, color })
    }
}

/// A Cloudflare account id: 32 hex characters.
pub fn valid_cf_account_id(value: &str) -> bool {
    value.len() == 32 && value.chars().all(|c| c.is_ascii_hexdigit())
}

fn valid_icon(icon: &str) -> bool {
    !icon.is_empty()
        && icon.len() <= 64
        && icon
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

fn valid_color(color: &str) -> bool {
    color.len() == 7 && color.starts_with('#') && color[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Every key a bucket entry may hold, per provider (`provider` included).
/// Saves refuse anything else; a stored entry's extra keys are ignored.
pub fn entry_keys(provider: &str) -> Option<&'static [&'static str]> {
    Some(match provider {
        "s3" => &[
            "provider",
            "bucket",
            "region",
            "access_key_id",
            "secret_access_key",
            "session_token",
            "endpoint_url",
            "force_path_style",
            "notifications",
            "display",
        ],
        "r2" => &[
            "provider",
            "bucket",
            "account_id",
            "access_key_id",
            "secret_access_key",
            "endpoint_url",
            "notifications",
            "display",
        ],
        "gcs" => &[
            "provider",
            "bucket",
            "credentials_file",
            "credentials_json",
            "endpoint_url",
            "notifications",
            "display",
        ],
        "local" => &["provider", "bucket", "display"],
        _ => return None,
    })
}

fn notification_keys(provider: &str) -> &'static [&'static str] {
    match provider {
        "s3" => &["sqs_queue_url"],
        "r2" => &["queue_id", "api_token"],
        "gcs" => &["pubsub_subscription"],
        _ => &[],
    }
}

/// Keys of an entry that are not fields of its provider, as dotted paths
/// (`display.size`, `notifications.x`); every top-level key when the
/// provider is missing or unknown is NOT reported (the parse says that).
pub fn unknown_entry_keys(entry: &Value) -> Vec<String> {
    let Some(obj) = entry.as_object() else {
        return Vec::new();
    };
    let Some(keys) = obj
        .get("provider")
        .and_then(Value::as_str)
        .and_then(entry_keys)
    else {
        return Vec::new();
    };
    let provider = obj["provider"].as_str().unwrap_or_default();
    let mut out = Vec::new();
    for (key, value) in obj {
        if !keys.contains(&key.as_str()) {
            out.push(key.clone());
            continue;
        }
        let nested: &[&str] = match key.as_str() {
            "display" => &["icon", "color"],
            "notifications" => notification_keys(provider),
            _ => continue,
        };
        if let Some(block) = value.as_object() {
            out.extend(
                block
                    .keys()
                    .filter(|k| !nested.contains(&k.as_str()))
                    .map(|k| format!("{key}.{k}")),
            );
        }
    }
    out.sort();
    out
}

impl BucketConfig {
    /// `s3` | `r2` | `gcs` | `local`.
    pub fn provider(&self) -> &'static str {
        match self {
            Self::S3(_) => "s3",
            Self::Gcs(_) => "gcs",
            Self::R2(_) => "r2",
            Self::Local(_) => "local",
        }
    }

    /// The provider-side bucket: `bucket`, else the worker-facing name.
    pub fn underlying(&self, name: &str) -> String {
        let explicit = match self {
            Self::S3(v) => v.bucket.as_deref(),
            Self::Gcs(v) => v.bucket.as_deref(),
            Self::R2(v) => v.bucket.as_deref(),
            Self::Local(v) => v.bucket.as_deref(),
        };
        explicit
            .filter(|value| !value.is_empty())
            .unwrap_or(name)
            .to_string()
    }

    pub fn display(&self) -> Option<&BucketDisplay> {
        match self {
            Self::S3(v) => v.display.as_ref(),
            Self::Gcs(v) => v.display.as_ref(),
            Self::R2(v) => v.display.as_ref(),
            Self::Local(v) => v.display.as_ref(),
        }
    }

    pub fn endpoint_url(&self) -> Option<&str> {
        match self {
            Self::S3(v) => v.endpoint_url.as_deref(),
            Self::Gcs(v) => v.endpoint_url.as_deref(),
            Self::R2(v) => v.endpoint_url.as_deref(),
            Self::Local(_) => None,
        }
    }

    /// The fields that may hold a `secret://` / `env://` reference, with
    /// their values: `access_key_id`, `secret_access_key`, `session_token`,
    /// `account_id`, `credentials_json`, `notifications.api_token`.
    pub fn credential_fields(&self) -> Vec<(&'static str, &str)> {
        let fields: Vec<(&'static str, Option<&str>)> = match self {
            Self::S3(v) => vec![
                ("access_key_id", v.access_key_id.as_deref()),
                ("secret_access_key", v.secret_access_key.as_deref()),
                ("session_token", v.session_token.as_deref()),
            ],
            Self::R2(v) => vec![
                ("account_id", Some(v.account_id.as_str())),
                ("access_key_id", Some(v.access_key_id.as_str())),
                ("secret_access_key", Some(v.secret_access_key.as_str())),
                (
                    "notifications.api_token",
                    v.notifications.as_ref().map(|n| n.api_token.as_str()),
                ),
            ],
            Self::Gcs(v) => vec![("credentials_json", v.credentials_json.as_deref())],
            Self::Local(_) => Vec::new(),
        };
        fields
            .into_iter()
            .filter_map(|(field, value)| value.map(|v| (field, v)))
            .collect()
    }

    /// Whether any value is a `secret://` / `env://` reference.
    pub fn uses_secrets(&self) -> bool {
        self.credential_fields()
            .iter()
            .any(|(_, v)| crate::secrets::parse_ref(v).is_some())
    }

    /// Check one parsed entry. Every message names the bucket and the
    /// field, never a value: a misplaced value may be a credential.
    pub fn validate(&self, name: &str) -> Result<(), String> {
        validate_bucket_name(name).map_err(|e| format!("bucket `{name}`: {e}"))?;
        if let Self::Local(local) = self {
            if let Some(underlying) = local.bucket.as_deref() {
                validate_bucket_name(underlying)
                    .map_err(|e| format!("bucket `{name}` local bucket `{underlying}`: {e}"))?;
            }
        }
        for (field, value) in self.credential_fields() {
            if value.trim() == REDACTED {
                return Err(format!(
                    "bucket `{name}`: `{field}` holds the redaction placeholder \"{REDACTED}\"; \
                     store the value or a secret:// reference"
                ));
            }
        }
        if let Some(endpoint) = self.endpoint_url() {
            let parsed = url::Url::parse(endpoint)
                .map_err(|_| format!("bucket `{name}`: `endpoint_url` is not a valid url"))?;
            if !matches!(parsed.scheme(), "http" | "https") {
                return Err(format!(
                    "bucket `{name}`: `endpoint_url` must use http or https"
                ));
            }
        }
        Ok(())
    }
}

/// Parse and validate one stored bucket entry.
pub fn parse_bucket(name: &str, entry: &Value) -> Result<BucketConfig, String> {
    validate_bucket_name(name).map_err(|e| format!("bucket `{name}`: {e}"))?;
    if !entry.is_object() {
        return Err(format!("bucket `{name}`: the entry must be an object"));
    }
    let parsed: BucketConfig = serde_json::from_value(entry.clone())
        .map_err(|e| format!("bucket `{name}`: {}", sanitize_serde_error(&e.to_string())))?;
    parsed.validate(name)?;
    Ok(parsed)
}

/// A serde error text with every quoted value elided: `invalid type: string
/// "hunter2"` becomes `invalid type: string "…"`. Field names survive (serde
/// quotes them with backticks).
pub fn sanitize_serde_error(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_quote = false;
    for c in text.chars() {
        if c == '"' {
            if !in_quote {
                out.push_str("\"…");
            } else {
                out.push('"');
            }
            in_quote = !in_quote;
        } else if !in_quote {
            out.push(c);
        }
    }
    out
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
            bucket.validate(name)?;
        }
        self.validate_providers()
    }

    /// The `providers` block alone: the local listener and its public url.
    fn validate_providers(&self) -> Result<(), String> {
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

    /// Parse the live configuration, isolating bucket entries: one that does
    /// not parse or validate lands in [`WorkerConfig::invalid`] with its
    /// error instead of failing the whole value. `providers` stays strict:
    /// the local listener is applied all-or-nothing.
    pub fn from_json_isolating(value: &Value) -> Result<Self, String> {
        let mut rest = value.clone();
        let Some(obj) = rest.as_object_mut() else {
            return Err("json parse: the configuration value must be an object".into());
        };
        let entries = match obj.remove("buckets") {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(map)) => map,
            Some(_) => return Err("json parse: `buckets` must be an object".into()),
        };
        let mut cfg: WorkerConfig = serde_json::from_value(rest)
            .map_err(|e| format!("json parse: {}", sanitize_serde_error(&e.to_string())))?;
        cfg.validate_providers()?;
        for (name, entry) in entries {
            let unknown = unknown_entry_keys(&entry);
            if !unknown.is_empty() {
                // Stored by hand or by another client: ignored, but said.
                tracing::warn!(bucket = %name, keys = ?unknown, "bucket entry has keys that are not fields; ignored");
            }
            match parse_bucket(&name, &entry) {
                Ok(bucket) => {
                    cfg.buckets.insert(name, bucket);
                }
                Err(message) => {
                    cfg.invalid.insert(
                        name,
                        InvalidBucket {
                            message,
                            raw: entry,
                        },
                    );
                }
            }
        }
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
    fn isolating_parse_keeps_good_buckets_and_lists_bad_ones() {
        let cfg = WorkerConfig::from_json_isolating(&serde_json::json!({
            "providers": { "local": { "data_dir": "/tmp/x" } },
            "buckets": {
                "good": { "provider": "local" },
                "Bad Name": { "provider": "local" },
                "noregion": { "provider": "s3" },
                "typo": { "provider": "s3", "region": "r", "force_path_style": "yes" },
                "placeholder": { "provider": "s3", "region": "r", "secret_access_key": "__redacted__" },
                "badurl": { "provider": "s3", "region": "r", "endpoint_url": "ftp://x" },
                "extra": { "provider": "local", "colour": "red" }
            }
        }))
        .unwrap();
        assert!(cfg.buckets.contains_key("good"));
        assert!(
            cfg.buckets.contains_key("extra"),
            "unknown keys are ignored when loading"
        );
        for bad in ["Bad Name", "noregion", "typo", "placeholder", "badurl"] {
            assert!(cfg.invalid.contains_key(bad), "{bad} should be invalid");
        }
        assert!(
            !cfg.invalid["typo"].message.contains("yes"),
            "values are never echoed"
        );
        assert!(cfg.invalid["placeholder"]
            .message
            .contains("redaction placeholder"));
        // The providers block stays strict.
        assert!(WorkerConfig::from_json_isolating(&serde_json::json!({
            "providers": { "local": { "http": { "bind_address": "nope" } } }
        }))
        .is_err());
        assert!(WorkerConfig::from_json_isolating(&serde_json::json!({ "buckets": [] })).is_err());
        assert!(WorkerConfig::from_json_isolating(&serde_json::json!({}))
            .unwrap()
            .buckets
            .is_empty());
    }

    #[test]
    fn credential_fields_and_references_per_provider() {
        let r2: BucketConfig = serde_json::from_value(serde_json::json!({
            "provider": "r2", "account_id": "env://CF_ACCOUNT", "access_key_id": "k",
            "secret_access_key": "secret://R2_SK",
            "notifications": { "queue_id": "q", "api_token": "secret://CF_TOKEN" },
            "display": { "icon": "cloud", "color": "#F38020" }
        }))
        .unwrap();
        let fields: Vec<_> = r2.credential_fields().into_iter().map(|(f, _)| f).collect();
        assert_eq!(
            fields,
            vec![
                "account_id",
                "access_key_id",
                "secret_access_key",
                "notifications.api_token"
            ]
        );
        assert!(r2.uses_secrets());
        assert_eq!(r2.underlying("docs"), "docs");
        assert_eq!(r2.display().unwrap().color.as_deref(), Some("#F38020"));
        let gcs: BucketConfig = serde_json::from_value(serde_json::json!({
            "provider": "gcs", "bucket": "b", "credentials_json": "{\"private_key\":\"LEAK\"}"
        }))
        .unwrap();
        assert!(!gcs.uses_secrets());
        assert!(
            !format!("{gcs:?}").contains("LEAK"),
            "Debug redacts credentials_json"
        );
        let local: BucketConfig =
            serde_json::from_value(serde_json::json!({ "provider": "local" })).unwrap();
        assert!(local.credential_fields().is_empty());
        // Absent display/credentials_json are not written back as nulls.
        let json = serde_json::to_value(&local).unwrap();
        assert!(json.get("display").is_none(), "{json}");
    }

    #[test]
    fn display_validation_and_unknown_keys() {
        let ok = BucketDisplay {
            icon: Some("hard-drive".into()),
            color: Some("#3b82F6".into()),
        };
        assert!(ok.validate().is_ok());
        assert_eq!(ok.sanitized(), Some(ok.clone()));
        let bad = BucketDisplay {
            icon: Some("Hard Drive".into()),
            color: Some("#12345".into()),
        };
        assert!(bad.validate().unwrap_err().contains("display.icon"));
        assert_eq!(bad.sanitized(), None);
        assert_eq!(
            unknown_entry_keys(&serde_json::json!({
                "provider": "s3", "region": "r", "Region": "x",
                "display": { "icon": "x", "size": 2 },
                "notifications": { "sqs_queue_url": "u", "queue_id": "q" }
            })),
            vec!["Region", "display.size", "notifications.queue_id"]
        );
        assert_eq!(
            unknown_entry_keys(&serde_json::json!({ "provider": "local", "region": "r" })),
            vec!["region"]
        );
        assert!(unknown_entry_keys(&serde_json::json!({ "provider": "ftp", "x": 1 })).is_empty());
        assert_eq!(
            sanitize_serde_error("invalid type: string \"hunter2\", expected a boolean"),
            "invalid type: string \"…\", expected a boolean"
        );
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
