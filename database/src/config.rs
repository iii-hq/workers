//! Configuration parsing for the database worker.
//!
//! The worker accepts a YAML file with a `databases:` map keyed by name.
//! Each entry has a `url` (whose scheme picks the driver) and an optional
//! `pool` block. Environment variables in the form `${NAME}` are expanded
//! against the process environment.

use serde::Deserialize;
use std::collections::HashMap;

/// Top-level worker config (the contents of `config.yaml`, or the `config`
/// block of `iii-config.yaml` when running embedded).
#[derive(Debug, Clone, Deserialize)]
pub struct WorkerConfig {
    #[serde(default)]
    pub databases: HashMap<String, DatabaseConfig>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DatabaseConfig {
    pub url: String,
    #[serde(default)]
    pub pool: PoolConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Populated by [`WorkerConfig::from_yaml`] from the URL scheme.
    /// Do not construct `DatabaseConfig` directly without calling
    /// `detect_driver` — the default `Sqlite` value will silently mismatch
    /// the URL.
    #[serde(skip)]
    pub driver: DriverKind,
}

/// TLS settings for a single database. Applies to postgres and mysql.
/// Sqlite is local-file and ignores this block.
///
/// Default is `mode: require` — TLS handshake required, certificate chain
/// validated against the system trust store, hostname verification skipped
/// (matching libpq's `sslmode=require` semantics). Use `mode: verify-full`
/// to additionally verify the certificate hostname matches the URL host,
/// and `mode: disable` to opt out of TLS entirely (local-dev only).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct TlsConfig {
    #[serde(default)]
    pub mode: TlsMode,
    /// Optional path to a PEM file containing one or more CA certificates.
    /// When set, the system trust store is **replaced** by these certs
    /// (not extended). Use this for self-hosted databases with a private CA.
    #[serde(default)]
    pub ca_cert: Option<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TlsMode {
    /// No TLS. Plaintext connection. Local-dev only.
    Disable,
    /// TLS handshake required; certificate chain validated; hostname NOT
    /// verified. Matches libpq's `sslmode=require`. The default.
    #[default]
    Require,
    /// TLS handshake required; certificate chain validated; certificate
    /// hostname must match the URL host. Matches libpq's `sslmode=verify-full`.
    #[serde(rename = "verify-full")]
    VerifyFull,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum DriverKind {
    Postgres,
    Mysql,
    #[default]
    Sqlite,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PoolConfig {
    #[serde(default = "default_pool_max")]
    pub max: u32,
    #[serde(default = "default_idle_timeout_ms")]
    pub idle_timeout_ms: u64,
    #[serde(default = "default_acquire_timeout_ms")]
    pub acquire_timeout_ms: u64,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max: default_pool_max(),
            idle_timeout_ms: default_idle_timeout_ms(),
            acquire_timeout_ms: default_acquire_timeout_ms(),
        }
    }
}

fn default_pool_max() -> u32 {
    10
}
fn default_idle_timeout_ms() -> u64 {
    30_000
}
fn default_acquire_timeout_ms() -> u64 {
    5_000
}

impl WorkerConfig {
    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let mut cfg: WorkerConfig =
            serde_yml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        if cfg.databases.is_empty() {
            return Err("config must declare at least one database".into());
        }
        for (name, db) in cfg.databases.iter_mut() {
            db.driver = detect_driver(&db.url).ok_or_else(|| {
                format!(
                    "unknown url scheme for db `{name}`: {}",
                    redact_url(&db.url)
                )
            })?;
        }
        Ok(cfg)
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
    }
}

/// Strip the userinfo from a URL-like string for safe logging.
///
/// Best-effort: malformed or non-URL forms (e.g. `sqlite::memory:`) are
/// returned unchanged because the `url` crate cannot parse them and they
/// cannot carry credentials anyway. Successfully parsed URLs have their
/// password removed and any non-empty username replaced with `***`.
pub fn redact_url(input: &str) -> String {
    use url::Url;
    if let Ok(parsed) = Url::parse(input) {
        let mut redacted = parsed;
        if redacted.password().is_some() {
            let _ = redacted.set_password(None);
        }
        if !redacted.username().is_empty() {
            let _ = redacted.set_username("***");
        }
        return redacted.into();
    }
    input.to_string()
}

/// Validate a SQL identifier component (table name, column name, schema, etc.).
/// Allows ASCII letters, digits, underscore. Must start with letter or underscore.
/// Max 63 chars (Postgres NAMEDATALEN - 1).
///
/// This is the chokepoint for any operator-supplied identifier that gets
/// interpolated into a SQL string via `format!()` (replication slots,
/// publication names, schema/table names, cursor table). Validation is
/// strict ASCII because the alternative — quoting and escaping per-driver —
/// is fragile and the v1.0 surface does not need unicode identifiers.
pub fn validate_sql_identifier(s: &str) -> Result<(), String> {
    if s.is_empty() {
        return Err("identifier is empty".into());
    }
    if s.len() > 63 {
        return Err(format!("identifier `{s}` exceeds 63 characters"));
    }
    let mut chars = s.chars();
    let first = chars.next().unwrap();
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(format!(
            "identifier `{s}` must start with letter or underscore"
        ));
    }
    for c in chars {
        if !(c.is_ascii_alphanumeric() || c == '_') {
            return Err(format!(
                "identifier `{s}` contains invalid character `{c}` (only [a-zA-Z0-9_] allowed)"
            ));
        }
    }
    Ok(())
}

fn detect_driver(url: &str) -> Option<DriverKind> {
    let lower = url.to_ascii_lowercase();
    if lower.starts_with("postgres://") || lower.starts_with("postgresql://") {
        Some(DriverKind::Postgres)
    } else if lower.starts_with("mysql://") {
        Some(DriverKind::Mysql)
    } else if lower.starts_with("sqlite:") {
        Some(DriverKind::Sqlite)
    } else {
        None
    }
}

/// Expand `${NAME}` occurrences against the process environment.
/// Unknown variables expand to the empty string and emit a tracing warning.
/// Non-ASCII content outside `${...}` markers is preserved verbatim.
fn expand_env(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find("${") {
        // Push the prefix verbatim (UTF-8-safe slice — start is a char boundary
        // because it points at an ASCII `$`).
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) => out.push_str(&v),
                    Err(_) => {
                        tracing::warn!(var = %name, "config references undefined env var");
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                // Unterminated `${`; treat as literal.
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(yaml: &str) -> WorkerConfig {
        WorkerConfig::from_yaml(yaml).unwrap()
    }

    #[test]
    fn parses_single_sqlite_database() {
        let yaml = r#"
databases:
  primary:
    url: sqlite:./data/iii.db
"#;
        let c = cfg(yaml);
        assert_eq!(c.databases.len(), 1);
        let db = &c.databases["primary"];
        assert!(matches!(db.driver, DriverKind::Sqlite));
        assert_eq!(db.url, "sqlite:./data/iii.db");
        assert_eq!(db.pool.max, 10);
        assert_eq!(db.pool.idle_timeout_ms, 30_000);
        assert_eq!(db.pool.acquire_timeout_ms, 5_000);
    }

    #[test]
    fn parses_postgres_url() {
        let c = cfg("databases:\n  p:\n    url: postgres://u@h/db\n");
        assert!(matches!(c.databases["p"].driver, DriverKind::Postgres));
    }

    #[test]
    fn parses_postgresql_alias() {
        let c = cfg("databases:\n  p:\n    url: postgresql://u@h/db\n");
        assert!(matches!(c.databases["p"].driver, DriverKind::Postgres));
    }

    #[test]
    fn parses_mysql_url() {
        let c = cfg("databases:\n  m:\n    url: mysql://u@h/db\n");
        assert!(matches!(c.databases["m"].driver, DriverKind::Mysql));
    }

    #[test]
    fn unknown_url_scheme_errors() {
        let err =
            WorkerConfig::from_yaml("databases:\n  x:\n    url: oracle://h/db\n").unwrap_err();
        assert!(err.contains("unknown url scheme"), "got: {err}");
    }

    #[test]
    fn pool_overrides_take_effect() {
        // URL is quoted because `sqlite::memory:` contains a trailing colon
        // that YAML would otherwise interpret as a nested mapping key.
        let yaml = r#"
databases:
  primary:
    url: "sqlite::memory:"
    pool:
      max: 25
      idle_timeout_ms: 1000
      acquire_timeout_ms: 250
"#;
        let c = cfg(yaml);
        let p = &c.databases["primary"].pool;
        assert_eq!(p.max, 25);
        assert_eq!(p.idle_timeout_ms, 1000);
        assert_eq!(p.acquire_timeout_ms, 250);
    }

    #[test]
    fn env_var_expansion_in_url() {
        std::env::set_var("DATABASE_WORKER_TEST_URL", "sqlite::memory:");
        // Quote the interpolation site so the expanded value (which ends in
        // a colon) is unambiguously a YAML scalar.
        let yaml = "databases:\n  p:\n    url: \"${DATABASE_WORKER_TEST_URL}\"\n";
        let c = cfg(yaml);
        assert_eq!(c.databases["p"].url, "sqlite::memory:");
        std::env::remove_var("DATABASE_WORKER_TEST_URL");
    }

    #[test]
    fn empty_databases_block_errors() {
        let err = WorkerConfig::from_yaml("databases: {}\n").unwrap_err();
        assert!(err.contains("at least one database"), "got: {err}");
    }

    #[test]
    fn env_var_expansion_multiple_in_one_url() {
        std::env::set_var("DBW_TEST_USER", "alice");
        std::env::set_var("DBW_TEST_HOST", "host.example");
        std::env::set_var("DBW_TEST_DB", "shop");
        let yaml = "databases:\n  p:\n    url: \"postgres://${DBW_TEST_USER}@${DBW_TEST_HOST}/${DBW_TEST_DB}\"\n";
        let c = cfg(yaml);
        assert_eq!(c.databases["p"].url, "postgres://alice@host.example/shop");
        std::env::remove_var("DBW_TEST_USER");
        std::env::remove_var("DBW_TEST_HOST");
        std::env::remove_var("DBW_TEST_DB");
    }

    #[test]
    fn validate_sql_identifier_accepts_normal_names() {
        assert!(validate_sql_identifier("orders").is_ok());
        assert!(validate_sql_identifier("_iii_cursors").is_ok());
        assert!(validate_sql_identifier("users_2024").is_ok());
        assert!(validate_sql_identifier("A").is_ok());
        assert!(validate_sql_identifier("_").is_ok());
    }

    #[test]
    fn validate_sql_identifier_rejects_empty() {
        let err = validate_sql_identifier("").unwrap_err();
        assert!(err.contains("empty"), "got: {err}");
    }

    #[test]
    fn validate_sql_identifier_rejects_digit_first() {
        let err = validate_sql_identifier("1users").unwrap_err();
        assert!(err.contains("start with"), "got: {err}");
    }

    #[test]
    fn validate_sql_identifier_rejects_injection_chars() {
        assert!(validate_sql_identifier("orders; DROP").is_err());
        assert!(validate_sql_identifier("orders'--").is_err());
        assert!(validate_sql_identifier("orders\"").is_err());
        assert!(validate_sql_identifier("a b").is_err());
        assert!(validate_sql_identifier("a.b").is_err());
    }

    #[test]
    fn validate_sql_identifier_rejects_too_long() {
        let s: String = "a".repeat(64);
        let err = validate_sql_identifier(&s).unwrap_err();
        assert!(err.contains("exceeds 63"), "got: {err}");
        // Boundary: 63 is OK.
        let ok: String = "a".repeat(63);
        assert!(validate_sql_identifier(&ok).is_ok());
    }

    #[test]
    fn redact_url_strips_password() {
        assert_eq!(
            redact_url("postgres://user:pass@host/db"),
            "postgres://***@host/db"
        );
        assert_eq!(
            redact_url("mysql://admin:s3cret@127.0.0.1:3306/test"),
            "mysql://***@127.0.0.1:3306/test"
        );
    }

    #[test]
    fn redact_url_handles_no_password() {
        assert_eq!(
            redact_url("postgres://user@host/db"),
            "postgres://***@host/db"
        );
    }

    #[test]
    fn redact_url_handles_no_userinfo() {
        let result = redact_url("postgres://host/db");
        assert!(!result.contains('@'), "no userinfo should remain: {result}");
    }

    #[test]
    fn redact_url_passthrough_sqlite() {
        // The `url` crate does not parse `sqlite:` URIs (no authority); the
        // helper falls back to returning the input unchanged. Either way
        // these forms cannot carry credentials.
        assert_eq!(redact_url("sqlite::memory:"), "sqlite::memory:");
        let result = redact_url("sqlite:./data/iii.db");
        assert!(
            !result.contains("user:"),
            "no credentials present: {result}"
        );
    }

    #[test]
    fn redact_url_unknown_scheme_passthrough() {
        // Malformed/unknown schemes round-trip unchanged. The caller is
        // responsible for not leaking them in error messages, but redact_url
        // is best-effort.
        assert_eq!(redact_url("not-a-url"), "not-a-url");
    }

    #[test]
    fn expand_env_preserves_unicode_outside_markers() {
        // Direct unit test of the expand_env helper to guard against the
        // "byte-iteration mojibake" regression. The helper is private; we
        // exercise it via a YAML containing a non-ASCII comment.
        let yaml = "# café 日本語\ndatabases:\n  p:\n    url: \"sqlite::memory:\"\n";
        // Note: serde_yml strips comments, but expand_env runs on the raw
        // text *before* parsing. If the helper corrupted UTF-8, the parse
        // would fail because the multibyte sequence would be mangled into
        // an invalid byte run inside the string we hand to serde_yml.
        let c = cfg(yaml);
        assert!(matches!(c.databases["p"].driver, DriverKind::Sqlite));
    }
}
