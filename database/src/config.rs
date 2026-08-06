//! Configuration parsing for the database worker.
//!
//! Runtime config is stored in the `configuration` worker under id `database`.
//! When no stored value and no `--config` seed exist, [`WorkerConfig::default`]
//! supplies a local SQLite pool. An optional YAML seed file (`--config`) may
//! override `initial_value` on first register. Each database entry has a `url`
//! (whose scheme picks the driver) and an optional `pool` block. The seed path
//! expands `${NAME}` against the process environment; values read from
//! `configuration::get` are already expanded by the configuration worker
//! (`${VAR:default}` syntax).

use schemars::schema::Schema;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

pub const DEFAULT_DB_NAME: &str = "primary";
pub const DEFAULT_SQLITE_URL: &str = "sqlite:./data/iii.db";

/// Top-level worker config registered with the `configuration` worker.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(example = "worker_config_example")]
pub struct WorkerConfig {
    #[serde(default)]
    #[schemars(schema_with = "databases_schema")]
    pub databases: HashMap<String, DatabaseConfig>,
}

fn worker_config_example() -> WorkerConfig {
    WorkerConfig::default()
}

fn databases_schema(gen: &mut schemars::gen::SchemaGenerator) -> Schema {
    let mut schema = gen.subschema_for::<HashMap<String, DatabaseConfig>>();
    if let Schema::Object(obj) = &mut schema {
        obj.metadata().description = Some(
            "Named connection pools. Keys are logical database names referenced \
             by RPC handlers (for example `primary`). At least one entry is required."
                .into(),
        );
        obj.metadata().examples = vec![json!({
            "primary": {
                "url": DEFAULT_SQLITE_URL,
                "pool": {
                    "max": 10,
                    "idle_timeout_ms": 30000,
                    "acquire_timeout_ms": 5000
                }
            }
        })];
        if let Some(validation) = obj.object.as_mut() {
            validation.min_properties = Some(1);
        }
    }
    schema
}

/// Per-database connection settings. The URL scheme selects the driver;
/// `pool` and `tls` are optional and default when omitted.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct DatabaseConfig {
    /// Connection URL. Driver is inferred from the scheme: `sqlite:`,
    /// `postgres://` or `postgresql://`, or `mysql://`.
    pub url: String,
    #[serde(default)]
    pub pool: PoolConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    /// How `database::row-changed` events are captured for this database.
    /// `statements` (default) classifies the SQL this worker executes;
    /// `native` makes writes from ANY client — psql, other processes — fire
    /// too. Postgres: triggers + LISTEN/NOTIFY. File-backed sqlite: a
    /// trigger-fed changelog drained on filesystem wake-up. MySQL: the
    /// binlog replication stream (needs REPLICATION SLAVE + CLIENT).
    #[serde(default, skip_serializing_if = "CaptureMode::is_statements")]
    pub capture: CaptureMode,
    /// Populated by [`WorkerConfig::finalize`] from the URL scheme.
    /// Do not construct `DatabaseConfig` directly without calling
    /// `finalize` — the default `Sqlite` value will silently mismatch
    /// the URL.
    #[serde(skip)]
    #[schemars(skip)]
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
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TlsConfig {
    /// TLS mode: `disable` (plaintext), `require` (default), or `verify-full`.
    #[serde(default)]
    pub mode: TlsMode,
    /// Optional path to a PEM file containing one or more CA certificates.
    /// Additive by default — these certs **extend** the system trust store
    /// rather than replace it. Set `trust_native: false` for strict-isolation
    /// deployments that must only trust the operator-supplied bundle.
    #[serde(default)]
    pub ca_cert: Option<String>,
    /// When true (default), the system/native trust store is loaded in
    /// addition to any `ca_cert` bundle. Set to `false` to trust only the
    /// `ca_cert` certificates — useful when an operator wants to pin trust
    /// to a private CA and explicitly *not* accept the public web PKI.
    ///
    /// Effective for postgres. MySQL is forced-additive: `mysql_async`'s
    /// rustls path always loads the Mozilla `webpki_roots` bundle and
    /// extends it with `ca_cert` — there is no upstream knob to suppress
    /// the bundled roots, so `trust_native: false` only affects postgres.
    ///
    /// Note: with both `trust_native: false` *and* `ca_cert: None` on
    /// postgres, no trust roots are available; pool construction fails
    /// with `CONFIG_ERROR`.
    #[serde(default = "default_trust_native")]
    pub trust_native: bool,
}

impl Default for TlsConfig {
    fn default() -> Self {
        Self {
            mode: TlsMode::default(),
            ca_cert: None,
            trust_native: default_trust_native(),
        }
    }
}

fn default_trust_native() -> bool {
    true
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
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

/// How row-change events are captured for one database.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMode {
    /// Classify the SQL this worker executes. Writes from other clients are
    /// invisible. Works on every driver. The default.
    #[default]
    Statements,
    /// Capture writes from any client, including other processes.
    /// Table-scoped bindings only. Postgres: triggers + LISTEN/NOTIFY on a
    /// dedicated connection (needs DDL rights). File-backed sqlite:
    /// triggers + changelog table + filesystem watch. MySQL: the binlog
    /// replication stream (needs replication grants, nothing installed).
    Native,
}

impl CaptureMode {
    pub fn is_statements(&self) -> bool {
        *self == CaptureMode::Statements
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct PoolConfig {
    /// Maximum number of open connections in the pool.
    #[serde(default = "default_pool_max")]
    pub max: u32,
    /// Close idle connections after this many milliseconds.
    #[serde(default = "default_idle_timeout_ms")]
    pub idle_timeout_ms: u64,
    /// Fail pool acquisition when no connection is available within this many milliseconds.
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

impl Default for WorkerConfig {
    fn default() -> Self {
        Self::default_unchecked()
    }
}

impl WorkerConfig {
    fn default_unchecked() -> Self {
        Self::finalize(WorkerConfig {
            databases: HashMap::from([(
                DEFAULT_DB_NAME.to_string(),
                DatabaseConfig {
                    url: DEFAULT_SQLITE_URL.to_string(),
                    pool: PoolConfig::default(),
                    tls: TlsConfig::default(),
                    capture: CaptureMode::default(),
                    driver: DriverKind::default(),
                },
            )]),
        })
        .expect("built-in default config is valid")
    }

    pub fn from_yaml(yaml: &str) -> Result<Self, String> {
        let expanded = expand_env(yaml);
        let cfg: WorkerConfig =
            serde_yml::from_str(&expanded).map_err(|e| format!("yaml parse: {e}"))?;
        Self::finalize(cfg)
    }

    pub fn from_json(value: &Value) -> Result<Self, String> {
        let cfg: WorkerConfig =
            serde_json::from_value(value.clone()).map_err(|e| format!("json parse: {e}"))?;
        Self::finalize(cfg)
    }

    pub fn from_file(path: &str) -> Result<Self, String> {
        let raw = std::fs::read_to_string(path).map_err(|e| format!("read {path}: {e}"))?;
        Self::from_yaml(&raw)
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
            obj.insert(
                "example".into(),
                json!({
                    "databases": {
                        DEFAULT_DB_NAME: {
                            "url": DEFAULT_SQLITE_URL,
                            "pool": {
                                "max": default_pool_max(),
                                "idle_timeout_ms": default_idle_timeout_ms(),
                                "acquire_timeout_ms": default_acquire_timeout_ms(),
                            }
                        }
                    }
                }),
            );
        }
        schema
    }

    fn finalize(mut cfg: WorkerConfig) -> Result<Self, String> {
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
            if db.capture == CaptureMode::Native {
                match db.driver {
                    // Postgres captures via LISTEN/NOTIFY; server-side
                    // prerequisites are checked at binding registration,
                    // where failures are actionable.
                    DriverKind::Postgres => {}
                    DriverKind::Mysql => {
                        // Binlog events are filtered to the url's schema; a
                        // url without one would capture every database on
                        // the server — table names and change volumes from
                        // schemas this handle was never meant to see.
                        let has_schema = url::Url::parse(&db.url)
                            .map(|u| !u.path().trim_start_matches('/').is_empty())
                            .unwrap_or(false);
                        if !has_schema {
                            return Err(format!(
                                "db `{name}`: `capture: native` on mysql requires the \
                                 url to name a database (mysql://host/dbname) — binlog \
                                 events are filtered to that schema"
                            ));
                        }
                    }
                    DriverKind::Sqlite => {
                        // A `:memory:` database exists per connection — a
                        // watcher connection would open a different database
                        // and hear nothing, ever.
                        if db.url.contains(":memory:") {
                            return Err(format!(
                                "db `{name}`: `capture: native` requires a file-backed \
                                 sqlite database; `:memory:` is per-connection and \
                                 cannot be observed"
                            ));
                        }
                    }
                }
            }
        }
        Ok(cfg)
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
/// interpolated into a SQL string via `format!()` (schema/table names,
/// cursor table). Validation is
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

pub(crate) fn detect_driver(url: &str) -> Option<DriverKind> {
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
    fn from_json_parses_sqlite_database() {
        let json = serde_json::json!({
            "databases": {
                "primary": { "url": "sqlite:./data/iii.db" }
            }
        });
        let c = WorkerConfig::from_json(&json).unwrap();
        assert!(matches!(c.databases["primary"].driver, DriverKind::Sqlite));
        assert_eq!(c.databases["primary"].url, "sqlite:./data/iii.db");
    }

    #[test]
    fn from_json_empty_databases_errors() {
        let err = WorkerConfig::from_json(&serde_json::json!({ "databases": {} })).unwrap_err();
        assert!(err.contains("at least one database"), "got: {err}");
    }

    #[test]
    fn to_json_roundtrip_omits_driver() {
        let yaml = "databases:\n  p:\n    url: \"sqlite::memory:\"\n";
        let cfg = cfg(yaml);
        let json = cfg.to_json();
        assert!(json["databases"]["p"].get("driver").is_none());
        let back = WorkerConfig::from_json(&json).unwrap();
        assert!(matches!(back.databases["p"].driver, DriverKind::Sqlite));
    }

    #[test]
    fn capture_native_allows_all_drivers_except_memory_sqlite() {
        for url in [
            "postgres://u@h/db",
            "sqlite:./data/iii.db",
            "mysql://u@h/db",
        ] {
            let c = cfg(&format!(
                "databases:\n  p:\n    url: {url}\n    capture: native\n"
            ));
            assert_eq!(c.databases["p"].capture, CaptureMode::Native, "{url}");
        }

        // A per-connection `:memory:` database cannot be observed.
        let err = WorkerConfig::from_yaml(
            "databases:\n  p:\n    url: \"sqlite::memory:\"\n    capture: native\n",
        )
        .unwrap_err();
        assert!(err.contains("file-backed"), "got: {err}");

        // A schema-less mysql url would capture every database on the server.
        let err = WorkerConfig::from_yaml(
            "databases:\n  p:\n    url: mysql://u@h\n    capture: native\n",
        )
        .unwrap_err();
        assert!(err.contains("name a database"), "got: {err}");
        let err = WorkerConfig::from_yaml(
            "databases:\n  p:\n    url: mysql://u@h/\n    capture: native\n",
        )
        .unwrap_err();
        assert!(err.contains("name a database"), "got: {err}");

        // Default stays statements and stays out of the serialized form —
        // existing configs round-trip byte-identical.
        let d = cfg("databases:\n  p:\n    url: postgres://u@h/db\n");
        assert_eq!(d.databases["p"].capture, CaptureMode::Statements);
        assert!(d.to_json()["databases"]["p"].get("capture").is_none());
    }

    #[test]
    fn json_schema_is_object_with_databases_property() {
        let schema = WorkerConfig::json_schema();
        assert!(schema
            .get("properties")
            .and_then(|p| p.get("databases"))
            .is_some());
    }

    #[test]
    fn default_matches_expected_primary_sqlite() {
        let cfg = WorkerConfig::default();
        assert_eq!(cfg.databases.len(), 1);
        let db = &cfg.databases[DEFAULT_DB_NAME];
        assert_eq!(db.url, DEFAULT_SQLITE_URL);
        assert!(matches!(db.driver, DriverKind::Sqlite));
        assert_eq!(db.pool.max, 10);
        assert_eq!(db.pool.idle_timeout_ms, 30_000);
        assert_eq!(db.pool.acquire_timeout_ms, 5_000);
    }

    #[test]
    fn default_json_roundtrips_and_omits_driver() {
        let cfg = WorkerConfig::default();
        let json = cfg.to_json();
        assert!(json["databases"][DEFAULT_DB_NAME].get("driver").is_none());
        let back = WorkerConfig::from_json(&json).unwrap();
        assert_eq!(back.databases[DEFAULT_DB_NAME].url, DEFAULT_SQLITE_URL);
        assert!(matches!(
            back.databases[DEFAULT_DB_NAME].driver,
            DriverKind::Sqlite
        ));
    }

    #[test]
    fn json_schema_describes_url_and_requires_databases() {
        let schema = WorkerConfig::json_schema();
        let databases = schema["properties"]["databases"].as_object().unwrap();
        assert!(databases.get("description").is_some());
        assert_eq!(databases["minProperties"], 1);

        let db_schema = schema["definitions"]["DatabaseConfig"].as_object().unwrap();
        let url = db_schema["properties"]["url"].as_object().unwrap();
        assert!(url.get("description").is_some());

        let pool_schema = schema["definitions"]["PoolConfig"].as_object().unwrap();
        for field in ["max", "idle_timeout_ms", "acquire_timeout_ms"] {
            assert!(
                pool_schema["properties"][field]
                    .get("description")
                    .is_some(),
                "missing description for pool.{field}"
            );
        }

        assert!(schema.get("example").is_some());
    }

    /// Regenerate the e2e harness schema fixture when `WorkerConfig` changes:
    /// `EXPORT_E2E_SCHEMA=1 cargo test -p database export_e2e_schema_fixture -- --ignored`
    #[test]
    #[ignore]
    fn export_e2e_schema_fixture() {
        if std::env::var("EXPORT_E2E_SCHEMA").is_err() {
            return;
        }
        let schema = WorkerConfig::json_schema();
        let pretty = serde_json::to_string_pretty(&schema).expect("schema serializes");
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/e2e/workers/harness/fixtures/database.schema.json"
        );
        std::fs::write(path, pretty + "\n").expect("write schema fixture");
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
