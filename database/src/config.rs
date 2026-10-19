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

fn default_sqlite_url() -> String {
    DEFAULT_SQLITE_URL.to_string()
}

/// Top-level worker config registered with the `configuration` worker.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(example = "worker_config_example")]
pub struct WorkerConfig {
    #[serde(default)]
    #[schemars(schema_with = "databases_schema")]
    pub databases: HashMap<String, DatabaseConfig>,
    /// Maximum entries kept per database in the console query history
    /// (`database::history`, stored on the `state` worker). Oldest entries
    /// are dropped first. `0` disables history recording. Applied live.
    #[serde(default = "default_history_max_entries")]
    pub history_max_entries: usize,
    /// Maximum JSON-serialized size in bytes of one database's stored query
    /// history; oldest entries are dropped until the list fits. The hard
    /// guard that keeps history from growing into a value large enough to
    /// break the `state` worker's connection. `0` disables history
    /// recording. Applied live.
    #[serde(default = "default_history_max_bytes")]
    pub history_max_bytes: usize,
    /// Entries left out because they do not parse or validate, by name, with
    /// why. Only [`WorkerConfig::from_json_isolating`] fills it: the running
    /// worker reports them as unavailable connections instead of refusing
    /// the whole value (or failing to start) over one hand-edited entry.
    #[serde(skip)]
    #[schemars(skip)]
    pub invalid: std::collections::BTreeMap<String, String>,
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
                "url": default_sqlite_url(),
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

/// The value a client sends back in place of a stored sensitive literal it
/// was never shown (`database::getConnection` returns it, and
/// `saveConnection` / `testConnection` swap the stored value back in). Never
/// valid in the stored configuration itself.
pub const REDACTED: &str = "__redacted__";

/// Per-database connection settings: either a connection `url` (whose scheme
/// selects the driver) or the structured form (`driver`, `host`, `port`,
/// `user`, `password`, `database`) — never both. `pool` and `tls` are
/// optional and default when omitted.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
#[schemars(example = "structured_example")]
pub struct DatabaseConfig {
    /// Connection URL. Driver is inferred from the scheme: `sqlite:`,
    /// `postgres://` or `postgresql://`, or `mysql://`. Mutually exclusive
    /// with the structured form (`driver` and its fields).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// Structured form: the driver. Mutually exclusive with `url`.
    #[serde(default, rename = "driver", skip_serializing_if = "Option::is_none")]
    #[schemars(rename = "driver")]
    pub declared_driver: Option<DriverKind>,
    /// Structured form (postgres/mysql): server host name or IP. Through an
    /// `ssh` tunnel, the host as the SSH server resolves it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
    /// Structured form (postgres/mysql): server port. Default 5432 / 3306.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    /// Structured form (postgres/mysql): user name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    /// Structured form (postgres/mysql): password — a literal,
    /// `secret://NAME` (the secrets worker's vault) or `env://NAME` (an
    /// environment variable the secrets worker reads). Resolved through
    /// `secrets::resolve`; never echoed back.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Structured form: database name (postgres/mysql), or the database file
    /// path on the engine host (sqlite; `:memory:` for an in-memory one).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default)]
    pub pool: PoolConfig,
    #[serde(default)]
    pub tls: TlsConfig,
    /// Reach the server through an SSH tunnel (postgres/mysql).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh: Option<SshConfig>,
    /// How clients draw this connection (icon, color). Metadata only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayConfig>,
    /// Absolute path of a folder on the engine host that holds this
    /// connection's `.sql` query files. Metadata only: the worker never
    /// reads it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// How `database::row-changed` events are captured for this database.
    /// `statements` (default) classifies the SQL this worker executes;
    /// `native` makes writes from ANY client — psql, other processes — fire
    /// too. Postgres: triggers + LISTEN/NOTIFY. File-backed sqlite: a
    /// trigger-fed changelog drained on filesystem wake-up. MySQL: the
    /// binlog replication stream (needs REPLICATION SLAVE + CLIENT).
    #[serde(default, skip_serializing_if = "CaptureMode::is_statements")]
    pub capture: CaptureMode,
    /// Populated by [`WorkerConfig::finalize`] from the URL scheme or the
    /// structured `driver`. Do not construct `DatabaseConfig` directly
    /// without calling `finalize` — the default `Sqlite` value will silently
    /// mismatch the URL.
    #[serde(skip)]
    #[schemars(skip)]
    pub driver: DriverKind,
}

fn structured_example() -> Value {
    json!({
        "driver": "postgres",
        "host": "db.example.com",
        "port": 5432,
        "user": "app",
        "password": "secret://DATABASE_SHOP_PASSWORD",
        "database": "shop",
        "tls": { "mode": "verify-full" },
        "display": { "icon": "database", "color": "#22C55E" }
    })
}

impl DatabaseConfig {
    /// A minimal url-form entry (driver detected by [`WorkerConfig::finalize`]).
    pub fn from_url(url: impl Into<String>) -> Self {
        Self {
            url: Some(url.into()),
            declared_driver: None,
            host: None,
            port: None,
            user: None,
            password: None,
            database: None,
            pool: PoolConfig::default(),
            tls: TlsConfig::default(),
            ssh: None,
            display: None,
            folder: None,
            capture: CaptureMode::default(),
            driver: DriverKind::default(),
        }
    }

    /// The sqlite url this entry opens (file-backed paths resolved against
    /// the project), for either form. `None` for postgres/mysql.
    pub(crate) fn sqlite_url(&self) -> Option<String> {
        if self.driver != DriverKind::Sqlite {
            return None;
        }
        Some(match (&self.url, &self.database) {
            (Some(url), _) => resolve_sqlite_url(url),
            (None, Some(path)) => resolve_sqlite_url(&format!("sqlite:{path}")),
            (None, None) => resolve_sqlite_url(DEFAULT_SQLITE_URL),
        })
    }

    /// The server's host and port (postgres/mysql), from either form. A url
    /// that names several hosts reports the first.
    pub fn server_endpoint(&self) -> Option<(String, u16)> {
        let default_port = match self.driver {
            DriverKind::Postgres => 5432,
            DriverKind::Mysql => 3306,
            DriverKind::Sqlite => return None,
        };
        match &self.url {
            Some(url) => {
                let parsed = url::Url::parse(url).ok()?;
                let host = parsed.host_str()?.trim_matches(['[', ']']).to_string();
                Some((host, parsed.port().unwrap_or(default_port)))
            }
            None => Some((self.host.clone()?, self.port.unwrap_or(default_port))),
        }
    }

    /// The url to show a reader: credentials removed. For the structured
    /// form, a url assembled from its fields (no password).
    pub fn display_url(&self) -> String {
        if let Some(url) = &self.url {
            return redact_url(url);
        }
        match self.driver {
            DriverKind::Sqlite => format!(
                "sqlite:{}",
                self.database.as_deref().unwrap_or(DEFAULT_SQLITE_URL)
            ),
            DriverKind::Postgres | DriverKind::Mysql => {
                let scheme = if self.driver == DriverKind::Postgres {
                    "postgres"
                } else {
                    "mysql"
                };
                let (host, port) = self.server_endpoint().unwrap_or_default();
                let host = if host.contains(':') {
                    format!("[{host}]")
                } else {
                    host
                };
                let user = if self.user.as_deref().is_some_and(|u| !u.is_empty()) {
                    "***@"
                } else {
                    ""
                };
                let db = self.database.as_deref().unwrap_or("");
                format!("{scheme}://{user}{host}:{port}/{db}")
            }
        }
    }

    /// The database (schema, on mysql) the entry names, from either form.
    pub fn database_name(&self) -> Option<String> {
        match &self.url {
            Some(url) if self.driver != DriverKind::Sqlite => url::Url::parse(url)
                .ok()
                .map(|u| u.path().trim_start_matches('/').to_string())
                .filter(|db| !db.is_empty()),
            _ => self.database.clone().filter(|db| !db.is_empty()),
        }
    }

    /// Every field whose value may be a `secret://` / `env://` reference,
    /// as `(field path, value)`.
    pub fn sensitive_fields(&self) -> Vec<(&'static str, &str)> {
        let ssh = self.ssh.as_ref();
        [
            ("password", self.password.as_deref()),
            ("tls.ca_cert", self.tls.ca_cert.as_deref()),
            ("tls.client_cert", self.tls.client_cert.as_deref()),
            ("tls.client_key", self.tls.client_key.as_deref()),
            ("ssh.password", ssh.and_then(|s| s.password.as_deref())),
            (
                "ssh.private_key",
                ssh.and_then(|s| s.private_key.as_deref()),
            ),
            ("ssh.passphrase", ssh.and_then(|s| s.passphrase.as_deref())),
        ]
        .into_iter()
        .filter_map(|(field, value)| value.map(|v| (field, v)))
        .collect()
    }

    /// Whether any value is a `secret://` / `env://` reference.
    pub fn uses_secrets(&self) -> bool {
        self.sensitive_fields()
            .iter()
            .any(|(_, v)| crate::secrets::parse_ref(v).is_some())
    }

    /// Check one entry and detect its driver. Every message names the entry
    /// and the field, never a value: a misplaced value may be a password.
    pub fn validate(&mut self, name: &str) -> Result<(), String> {
        self.pool.validate(name)?;
        let structured_fields = [
            ("host", self.host.is_some()),
            ("port", self.port.is_some()),
            ("user", self.user.is_some()),
            ("password", self.password.is_some()),
            ("database", self.database.is_some()),
        ];
        match (&self.url, self.declared_driver) {
            (Some(_), Some(_)) => {
                return Err(format!(
                    "db `{name}`: `url` and `driver` are mutually exclusive — use either the \
                     url or the structured form (driver, host, port, user, password, database)"
                ))
            }
            (None, None) => {
                return Err(format!(
                    "db `{name}`: set either `url` or `driver` (structured form: driver, host, \
                     port, user, password, database)"
                ))
            }
            (Some(url), None) => {
                if let Some((field, _)) = structured_fields.iter().find(|(_, set)| *set) {
                    return Err(format!(
                        "db `{name}`: `{field}` belongs to the structured form and cannot be \
                         combined with `url` — put it in the url, or replace the url with \
                         `driver` and its fields"
                    ));
                }
                self.driver = detect_driver(url).ok_or_else(|| {
                    format!("unknown url scheme for db `{name}`: {}", redact_url(url))
                })?;
                if url.contains(REDACTED) {
                    return Err(format!(
                        "db `{name}`: `url` holds the redaction placeholder `{REDACTED}`; send \
                         the real password or keep the stored url"
                    ));
                }
            }
            (None, Some(driver)) => {
                self.driver = driver;
                match driver {
                    DriverKind::Postgres | DriverKind::Mysql => {
                        if self.host.as_deref().is_none_or(|h| h.trim().is_empty()) {
                            return Err(format!("db `{name}`: `host` is required for {driver}"));
                        }
                        if self.user.as_deref().is_none_or(|u| u.trim().is_empty()) {
                            return Err(format!("db `{name}`: `user` is required for {driver}"));
                        }
                        if self.port == Some(0) {
                            return Err(format!("db `{name}`: `port` must be 1-65535"));
                        }
                    }
                    DriverKind::Sqlite => {
                        if self.database.as_deref().is_none_or(|d| d.trim().is_empty()) {
                            return Err(format!(
                                "db `{name}`: `database` (the file path) is required for sqlite"
                            ));
                        }
                        if self.database.as_deref().is_some_and(has_parent_segment) {
                            return Err(format!(
                                "db `{name}`: `database` must not climb with `..` segments; \
                                 write the path it means"
                            ));
                        }
                        for (field, set) in structured_fields.iter().take(4) {
                            if *set {
                                return Err(format!(
                                    "db `{name}`: `{field}` does not apply to sqlite (a local \
                                     file); only `database` (its path) does"
                                ));
                            }
                        }
                    }
                }
            }
        }

        for (field, value) in self.sensitive_fields() {
            if value == REDACTED {
                return Err(format!(
                    "db `{name}`: `{field}` holds the redaction placeholder `{REDACTED}`; send \
                     the real value or keep the stored one"
                ));
            }
            if let Some(Err(e)) = crate::secrets::parse_ref(value) {
                return Err(format!("db `{name}`: `{field}` {}", e.describe_malformed()));
            }
        }
        let tls = &self.tls;
        if tls.client_cert.is_some() != tls.client_key.is_some() {
            return Err(format!(
                "db `{name}`: mutual TLS needs both `tls.client_cert` and `tls.client_key`"
            ));
        }

        if let Some(ssh) = &self.ssh {
            if self.driver == DriverKind::Sqlite {
                return Err(format!(
                    "db `{name}`: `ssh` does not apply to sqlite (a local file)"
                ));
            }
            ssh.validate(name)?;
            if let Some(url) = &self.url {
                // One tunnel forwards to one host; a multi-host url has no
                // single target.
                if self.driver == DriverKind::Postgres
                    && url.split('@').nth(1).is_some_and(|rest| {
                        rest.split(['/', '?'])
                            .next()
                            .is_some_and(|hosts| hosts.contains(','))
                    })
                {
                    return Err(format!(
                        "db `{name}`: `ssh` needs a single host; the url names several"
                    ));
                }
                if self.server_endpoint().is_none() {
                    return Err(format!(
                        "db `{name}`: `ssh` needs the url to name the server host"
                    ));
                }
            }
        }

        if let Some(display) = &self.display {
            if let Some(color) = &display.color {
                let hex = color.strip_prefix('#').unwrap_or("");
                if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                    return Err(format!(
                        "db `{name}`: `display.color` must be #RRGGBB (got {color:?})"
                    ));
                }
            }
            if let Some(icon) = &display.icon {
                if icon.is_empty()
                    || icon.len() > 64
                    || !icon
                        .chars()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                {
                    return Err(format!(
                        "db `{name}`: `display.icon` must be a Lucide icon name (lowercase \
                         letters, digits and `-`)"
                    ));
                }
            }
        }
        if let Some(folder) = &self.folder {
            let windows_abs = folder.len() > 2
                && folder.as_bytes()[0].is_ascii_alphabetic()
                && &folder[1..2] == ":"
                && matches!(&folder[2..3], "\\" | "/");
            if !(folder.starts_with('/') || windows_abs) || folder.contains('\0') {
                return Err(format!(
                    "db `{name}`: `folder` must be an absolute path on the engine host"
                ));
            }
            if has_parent_segment(folder) {
                return Err(format!(
                    "db `{name}`: `folder` must not climb with `..` segments; write the path it means"
                ));
            }
        }

        if self.capture == CaptureMode::Native {
            match self.driver {
                // Postgres captures via LISTEN/NOTIFY; server-side
                // prerequisites are checked at binding registration,
                // where failures are actionable.
                DriverKind::Postgres => {}
                DriverKind::Mysql => {
                    // Binlog events are filtered to the entry's schema; one
                    // without a schema would capture every database on the
                    // server — table names and change volumes from schemas
                    // this handle was never meant to see.
                    if self.database_name().is_none() {
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
                    let target = self.url.as_deref().or(self.database.as_deref());
                    if target.is_some_and(|t| t.contains(":memory:")) {
                        return Err(format!(
                            "db `{name}`: `capture: native` requires a file-backed \
                             sqlite database; `:memory:` is per-connection and \
                             cannot be observed"
                        ));
                    }
                }
            }
        }
        Ok(())
    }
}

/// A path with a `..` component (either separator).
fn has_parent_segment(path: &str) -> bool {
    path.split(['/', '\\']).any(|segment| segment == "..")
}

/// serde's messages quote offending values (`invalid type: string "…"`),
/// and a misplaced value may be a password: drop every quoted span.
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

/// Connection names are passed in every call (`db`), stored as keys and
/// written into mention tokens: letters, digits, `_`, `-` and `.`, starting
/// with a letter, digit or `_`, at most 64 characters.
pub fn validate_connection_name(name: &str) -> Result<(), String> {
    let mut chars = name.chars();
    let ok = name.len() <= 64
        && chars
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'));
    if ok {
        Ok(())
    } else {
        Err(format!(
            "connection name {name:?} must be 1-64 letters, digits, `_`, `-` or `.`, starting \
             with a letter, digit or `_`"
        ))
    }
}

/// SSH tunnel to the database server (postgres/mysql): the worker connects
/// to `host:port` as `user` and forwards each database connection through a
/// `direct-tcpip` channel to the entry's host/port as the SSH server sees
/// them. TLS to the database still verifies the database host name.
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct SshConfig {
    /// SSH server host name or IP.
    pub host: String,
    /// SSH server port. Default 22.
    #[serde(default = "default_ssh_port")]
    pub port: u16,
    /// SSH user name.
    pub user: String,
    /// Password authentication — a literal, `secret://NAME` or `env://NAME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
    /// Private key authentication: the key itself (OpenSSH or PEM; RSA,
    /// ECDSA or Ed25519), a path to it on the engine host, `secret://NAME`
    /// or `env://NAME`. Tried before the password when both are set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private_key: Option<String>,
    /// Passphrase of an encrypted `private_key` — a literal, `secret://NAME`
    /// or `env://NAME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passphrase: Option<String>,
    /// Pin the server's host key by its OpenSSH SHA-256 fingerprint
    /// (`SHA256:…`, as `ssh-keygen -lf` prints it). A different key is
    /// refused. When absent, any key is accepted and `testConnection`
    /// reports the one it saw so a client can pin it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host_key: Option<String>,
}

fn default_ssh_port() -> u16 {
    22
}

impl SshConfig {
    fn validate(&self, name: &str) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err(format!("db `{name}`: `ssh.host` is required"));
        }
        if self.user.trim().is_empty() {
            return Err(format!("db `{name}`: `ssh.user` is required"));
        }
        if self.port == 0 {
            return Err(format!("db `{name}`: `ssh.port` must be 1-65535"));
        }
        if self.password.is_none() && self.private_key.is_none() {
            return Err(format!(
                "db `{name}`: `ssh` needs `password` or `private_key` (or both)"
            ));
        }
        if self.passphrase.is_some() && self.private_key.is_none() {
            return Err(format!(
                "db `{name}`: `ssh.passphrase` is set without `ssh.private_key`"
            ));
        }
        if let Some(pin) = &self.host_key {
            if !valid_host_key_pin(pin) {
                return Err(format!(
                    "db `{name}`: `ssh.host_key` must be an OpenSSH SHA-256 fingerprint \
                     (`SHA256:` followed by base64, as `ssh-keygen -lf` prints it)"
                ));
            }
        }
        Ok(())
    }
}

/// `SHA256:` + unpadded base64 of a 32-byte digest (43 characters).
pub fn valid_host_key_pin(pin: &str) -> bool {
    pin.strip_prefix("SHA256:").is_some_and(|b64| {
        let b64 = b64.trim_end_matches('=');
        b64.len() == 43
            && b64
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '+' || c == '/')
    })
}

/// How a client draws a connection. The worker only stores it.
#[derive(Debug, Clone, Default, Deserialize, Serialize, JsonSchema)]
pub struct DisplayConfig {
    /// A Lucide icon name (`database`, `server`, `cloud`, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// `#RRGGBB`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// TLS settings for a single database. Applies to postgres and mysql.
/// Sqlite is local-file and ignores this block.
///
/// Default is `mode: require` — TLS handshake required, certificate chain
/// validated against the system trust store, hostname verification skipped
/// (matching libpq's `sslmode=require` semantics). Use `mode: verify-full`
/// to additionally verify the certificate hostname matches the server host,
/// and `mode: disable` to opt out of TLS entirely (local-dev only).
#[derive(Debug, Clone, Deserialize, Serialize, JsonSchema)]
pub struct TlsConfig {
    /// TLS mode: `disable` (plaintext), `prefer` (TLS when the server offers
    /// it), `require` (default), `verify-ca`, or `verify-full`.
    #[serde(default)]
    pub mode: TlsMode,
    /// Optional CA certificate(s): a path to a PEM file, the PEM itself,
    /// `secret://NAME` or `env://NAME`. Additive by default — these certs
    /// **extend** the system trust store rather than replace it. Set
    /// `trust_native: false` for strict-isolation deployments that must only
    /// trust the operator-supplied bundle.
    #[serde(default)]
    pub ca_cert: Option<String>,
    /// Client certificate (chain) for mutual TLS: a path, the PEM itself,
    /// `secret://NAME` or `env://NAME`. Requires `client_key`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_cert: Option<String>,
    /// Private key of `client_cert` (PEM: PKCS#8, PKCS#1 or SEC1): a path,
    /// the PEM itself, `secret://NAME` or `env://NAME`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_key: Option<String>,
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
            client_cert: None,
            client_key: None,
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
    /// TLS when the server offers it (chain validated like `require`);
    /// plaintext only when the server does not offer TLS at all. A failed
    /// certificate check is an error, never a fallback.
    Prefer,
    /// TLS handshake required; certificate chain validated; hostname NOT
    /// verified. Matches libpq's `sslmode=require`. The default.
    #[default]
    Require,
    /// Same checks as `require` (chain validated against the trust roots,
    /// hostname not checked), named after libpq's `sslmode=verify-ca`.
    #[serde(rename = "verify-ca")]
    VerifyCa,
    /// TLS handshake required; certificate chain validated; certificate
    /// hostname must match the server host. Matches libpq's `sslmode=verify-full`.
    #[serde(rename = "verify-full")]
    VerifyFull,
}

impl TlsMode {
    pub fn as_str(self) -> &'static str {
        match self {
            TlsMode::Disable => "disable",
            TlsMode::Prefer => "prefer",
            TlsMode::Require => "require",
            TlsMode::VerifyCa => "verify-ca",
            TlsMode::VerifyFull => "verify-full",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DriverKind {
    Postgres,
    Mysql,
    #[default]
    Sqlite,
}

impl std::fmt::Display for DriverKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            DriverKind::Postgres => "postgres",
            DriverKind::Mysql => "mysql",
            DriverKind::Sqlite => "sqlite",
        })
    }
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
    /// Maximum number of open connections in the pool (1-500).
    #[serde(default = "default_pool_max")]
    #[schemars(range(min = 1, max = 500))]
    pub max: u32,
    /// Close idle connections after this many milliseconds (at most 24 h).
    #[serde(default = "default_idle_timeout_ms")]
    #[schemars(range(min = 0, max = 86_400_000))]
    pub idle_timeout_ms: u64,
    /// Fail pool acquisition when no connection is available within this many
    /// milliseconds (1 ms to 10 min).
    #[serde(default = "default_acquire_timeout_ms")]
    #[schemars(range(min = 1, max = 600_000))]
    pub acquire_timeout_ms: u64,
}

/// Bounds on pool settings. A pool sized from an unchecked number can ask
/// the allocator for gigabytes up front (a `max` of 1e9 aborted the
/// process), and a zero `max` panics inside the pool crates.
pub const POOL_MAX_LIMIT: u32 = 500;
pub const IDLE_TIMEOUT_LIMIT_MS: u64 = 86_400_000;
pub const ACQUIRE_TIMEOUT_LIMIT_MS: u64 = 600_000;

impl PoolConfig {
    fn validate(&self, name: &str) -> Result<(), String> {
        if !(1..=POOL_MAX_LIMIT).contains(&self.max) {
            return Err(format!(
                "db `{name}`: `pool.max` must be 1-{POOL_MAX_LIMIT}"
            ));
        }
        if self.idle_timeout_ms > IDLE_TIMEOUT_LIMIT_MS {
            return Err(format!(
                "db `{name}`: `pool.idle_timeout_ms` must be at most {IDLE_TIMEOUT_LIMIT_MS} (24 h)"
            ));
        }
        if !(1..=ACQUIRE_TIMEOUT_LIMIT_MS).contains(&self.acquire_timeout_ms) {
            return Err(format!(
                "db `{name}`: `pool.acquire_timeout_ms` must be 1-{ACQUIRE_TIMEOUT_LIMIT_MS} (10 min)"
            ));
        }
        Ok(())
    }
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
fn default_history_max_entries() -> usize {
    200
}
fn default_history_max_bytes() -> usize {
    262_144
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
                DatabaseConfig::from_url(default_sqlite_url()),
            )]),
            history_max_entries: default_history_max_entries(),
            history_max_bytes: default_history_max_bytes(),
            invalid: Default::default(),
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

    /// Parse the stored value the way the running worker applies it: an
    /// entry that does not parse or validate is set aside in
    /// [`WorkerConfig::invalid`] (its connection reports the reason) while
    /// every other entry loads. Only a value that is not an object, or that
    /// has no entries at all, is an error.
    pub fn from_json_isolating(value: &Value) -> Result<Self, String> {
        let mut rest = value.clone();
        let Some(obj) = rest.as_object_mut() else {
            return Err("json parse: the configuration value must be an object".into());
        };
        let entries = match obj.remove("databases") {
            None | Some(Value::Null) => serde_json::Map::new(),
            Some(Value::Object(map)) => map,
            Some(_) => return Err("json parse: `databases` must be an object".into()),
        };
        let mut cfg: WorkerConfig = serde_json::from_value(rest)
            .map_err(|e| format!("json parse: {}", sanitize_serde_error(&e.to_string())))?;
        for (name, entry) in entries {
            let parsed = serde_json::from_value::<DatabaseConfig>(entry)
                .map_err(|e| format!("db `{name}`: {}", sanitize_serde_error(&e.to_string())))
                .and_then(|mut db| db.validate(&name).map(|()| db));
            match parsed {
                Ok(db) => {
                    cfg.databases.insert(name, db);
                }
                Err(e) => {
                    cfg.invalid.insert(name, e);
                }
            }
        }
        if cfg.databases.is_empty() && cfg.invalid.is_empty() {
            return Err("config must declare at least one database".into());
        }
        Ok(cfg)
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
                            "url": default_sqlite_url(),
                            "pool": {
                                "max": default_pool_max(),
                                "idle_timeout_ms": default_idle_timeout_ms(),
                                "acquire_timeout_ms": default_acquire_timeout_ms(),
                            }
                        }
                    },
                    "history_max_entries": default_history_max_entries(),
                    "history_max_bytes": default_history_max_bytes(),
                }),
            );
        }
        schema
    }

    fn finalize(mut cfg: WorkerConfig) -> Result<Self, String> {
        if cfg.databases.is_empty() {
            return Err("config must declare at least one database".into());
        }
        // Sorted so the first error reported is deterministic.
        let mut names: Vec<String> = cfg.databases.keys().cloned().collect();
        names.sort();
        for name in names {
            if let Some(db) = cfg.databases.get_mut(&name) {
                db.validate(&name)?;
            }
        }
        Ok(cfg)
    }
}

fn resolve_sqlite_url(url: &str) -> String {
    let Some((scheme, path)) = url.split_once(':') else {
        return url.to_string();
    };
    if !scheme.eq_ignore_ascii_case("sqlite") {
        return url.to_string();
    }
    if path.contains(":memory:") {
        return url.to_string();
    }

    let (file_prefix, path) = path
        .get(..5)
        .filter(|prefix| prefix.eq_ignore_ascii_case("file:"))
        .map_or(("", path), |prefix| (prefix, &path[5..]));
    let (path, query) = path
        .split_once('?')
        .map_or((path, ""), |(path, query)| (path, query));
    let resolved = iii_worker_paths::resolve_path(path);
    if query.is_empty() {
        format!("{scheme}:{file_prefix}{}", resolved.display())
    } else {
        format!("{scheme}:{file_prefix}{}?{query}", resolved.display())
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
        assert_eq!(
            c.databases["primary"].url.as_deref(),
            Some(DEFAULT_SQLITE_URL)
        );
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
        assert_eq!(db.url.as_deref(), Some(default_sqlite_url().as_str()));
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
        assert_eq!(
            back.databases[DEFAULT_DB_NAME].url.as_deref(),
            Some(default_sqlite_url().as_str())
        );
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
        assert_eq!(
            databases["examples"][0]["primary"]["url"],
            default_sqlite_url()
        );
        assert_eq!(
            schema["example"]["databases"][DEFAULT_DB_NAME]["url"],
            default_sqlite_url()
        );

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

        for field in ["history_max_entries", "history_max_bytes"] {
            assert!(
                schema["properties"][field].get("description").is_some(),
                "missing description for {field}"
            );
        }

        assert!(schema.get("example").is_some());
    }

    #[test]
    fn history_caps_default_and_override() {
        let d = cfg("databases:\n  p:\n    url: postgres://u@h/db\n");
        assert_eq!(d.history_max_entries, 200);
        assert_eq!(d.history_max_bytes, 262_144);

        let c = cfg("databases:\n  p:\n    url: postgres://u@h/db\n\
             history_max_entries: 25\nhistory_max_bytes: 4096\n");
        assert_eq!(c.history_max_entries, 25);
        assert_eq!(c.history_max_bytes, 4096);
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
        assert_eq!(db.url.as_deref(), Some(default_sqlite_url().as_str()));
        assert_eq!(db.pool.max, 10);
        assert_eq!(db.pool.idle_timeout_ms, 30_000);
        assert_eq!(db.pool.acquire_timeout_ms, 5_000);
    }

    #[test]
    fn uppercase_sqlite_scheme_resolves_project_path_at_use() {
        let c = cfg("databases:\n  primary:\n    url: SQLiTE:FiLe:./data/case.db?mode=rwc\n");
        let expected = format!(
            "SQLiTE:FiLe:{}?mode=rwc",
            iii_worker_paths::project_path("data/case.db").display()
        );

        assert_eq!(c.databases["primary"].sqlite_url().unwrap(), expected);
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
        assert_eq!(c.databases["p"].url.as_deref(), Some("sqlite::memory:"));
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
        assert_eq!(
            c.databases["p"].url.as_deref(),
            Some("postgres://alice@host.example/shop")
        );
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

    fn entry_err(yaml: &str) -> String {
        WorkerConfig::from_yaml(&format!("databases:\n  x:\n{yaml}")).unwrap_err()
    }

    fn entry_ok(yaml: &str) -> DatabaseConfig {
        WorkerConfig::from_yaml(&format!("databases:\n  x:\n{yaml}"))
            .unwrap()
            .databases
            .remove("x")
            .unwrap()
    }

    #[test]
    fn url_and_structured_form_are_mutually_exclusive() {
        let err = entry_err(
            "    url: postgres://u@h/db\n    driver: postgres\n    host: h\n    user: u\n",
        );
        assert!(err.contains("mutually exclusive"), "{err}");
        let err = entry_err("    url: postgres://u@h/db\n    password: hunter2\n");
        assert!(
            err.contains("`password` belongs to the structured form"),
            "{err}"
        );
        assert!(!err.contains("hunter2"), "{err}");
        let err = entry_err("    pool: { max: 2 }\n");
        assert!(err.contains("set either `url` or `driver`"), "{err}");
    }

    #[test]
    fn the_structured_form_parses_with_defaults() {
        let db = entry_ok(
            "    driver: mysql\n    host: db.example\n    user: app\n    password: secret://P\n    database: shop\n    display: { icon: server, color: \"#F29111\" }\n    folder: /srv/queries\n",
        );
        assert_eq!(db.driver, DriverKind::Mysql);
        assert_eq!(db.server_endpoint(), Some(("db.example".into(), 3306)));
        assert_eq!(db.database_name().as_deref(), Some("shop"));
        assert_eq!(db.display_url(), "mysql://***@db.example:3306/shop");
        assert!(db.uses_secrets());
        assert_eq!(db.tls.mode, TlsMode::Require);
        // The serialized form keeps `driver` and drops what is unset.
        let json = serde_json::to_value(&db).unwrap();
        assert_eq!(json["driver"], "mysql");
        assert!(
            json.get("url").is_none() && json.get("port").is_none() && json.get("ssh").is_none()
        );

        let lite = entry_ok("    driver: sqlite\n    database: /tmp/x.db\n");
        assert_eq!(lite.sqlite_url().as_deref(), Some("sqlite:/tmp/x.db"));
    }

    #[test]
    fn structured_entries_name_the_missing_or_misplaced_field() {
        assert!(entry_err("    driver: postgres\n    user: u\n").contains("`host` is required"));
        assert!(entry_err("    driver: postgres\n    host: h\n").contains("`user` is required"));
        assert!(
            entry_err("    driver: postgres\n    host: h\n    user: u\n    port: 0\n")
                .contains("`port`")
        );
        assert!(entry_err("    driver: sqlite\n").contains("`database` (the file path)"));
        assert!(
            entry_err("    driver: sqlite\n    database: a.db\n    host: h\n")
                .contains("does not apply to sqlite")
        );
        assert!(entry_err("    driver: oracle\n    host: h\n").contains("unknown variant"));
    }

    #[test]
    fn sensitive_fields_refuse_the_placeholder_and_malformed_references() {
        let err = entry_err(
            "    driver: postgres\n    host: h\n    user: u\n    password: __redacted__\n",
        );
        assert!(
            err.contains("`password` holds the redaction placeholder"),
            "{err}"
        );
        let err = entry_err("    url: \"postgres://u:__redacted__@h/db\"\n");
        assert!(err.contains("redaction placeholder"), "{err}");
        let err = entry_err(
            "    driver: postgres\n    host: h\n    user: u\n    password: \"secret://bad name\"\n",
        );
        assert!(
            err.contains("`password` holds a malformed reference"),
            "{err}"
        );
        assert!(!err.contains("bad name"), "{err}");
        let err = entry_err(
            "    driver: postgres\n    host: h\n    user: u\n    tls: { client_cert: /c.pem }\n",
        );
        assert!(
            err.contains("needs both `tls.client_cert` and `tls.client_key`"),
            "{err}"
        );
    }

    #[test]
    fn ssh_blocks_are_validated() {
        let base = "    driver: postgres\n    host: pg-private\n    user: u\n";
        assert!(
            entry_ok(&format!(
                "{base}    ssh: {{ host: b, user: t, password: p }}\n"
            ))
            .ssh
            .unwrap()
            .port
                == 22
        );
        assert!(
            entry_err(&format!("{base}    ssh: {{ host: b, user: t }}\n"))
                .contains("needs `password` or `private_key`")
        );
        assert!(entry_err(&format!(
            "{base}    ssh: {{ host: b, user: t, password: p, passphrase: x }}\n"
        ))
        .contains("`ssh.passphrase` is set without `ssh.private_key`"));
        assert!(entry_err(&format!(
            "{base}    ssh: {{ host: b, user: t, password: p, host_key: \"MD5:aa\" }}\n"
        ))
        .contains("`ssh.host_key` must be an OpenSSH SHA-256 fingerprint"));
        assert!(entry_err(
            "    driver: sqlite\n    database: a.db\n    ssh: { host: b, user: t, password: p }\n"
        )
        .contains("`ssh` does not apply to sqlite"));
        assert!(valid_host_key_pin(
            "SHA256:0rGFgr3lbFUjBJmay4CvRxX16hnJLF7mvototEUXoW0"
        ));
        assert!(!valid_host_key_pin("SHA256:short"));
    }

    #[test]
    fn display_and_folder_are_checked() {
        let base = "    driver: sqlite\n    database: a.db\n";
        assert!(entry_err(&format!("{base}    display: {{ color: red }}\n")).contains("#RRGGBB"));
        assert!(
            entry_err(&format!("{base}    display: {{ icon: \"Not An Icon\" }}\n"))
                .contains("Lucide")
        );
        assert!(entry_err(&format!("{base}    folder: relative/path\n")).contains("absolute path"));
        entry_ok(&format!("{base}    folder: /abs/path\n"));
        entry_ok(&format!("{base}    folder: \"C:\\\\queries\"\n"));
    }

    #[test]
    fn connection_names_follow_the_documented_rule() {
        for ok in ["shop", "shop_pg", "Shop-2.prod", "_x", "9lives"] {
            assert!(validate_connection_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "-x", ".x", "has space", "a/b", "ü", &"a".repeat(65)] {
            assert!(validate_connection_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn json_schema_exposes_the_structured_form() {
        let schema = WorkerConfig::json_schema();
        let props = &schema["definitions"]["DatabaseConfig"]["properties"];
        for field in [
            "url", "driver", "host", "port", "user", "password", "database", "ssh", "display",
            "folder",
        ] {
            assert!(props.get(field).is_some(), "schema lacks {field}");
        }
        let modes = schema["definitions"]["TlsMode"].to_string();
        for mode in ["disable", "prefer", "require", "verify-ca", "verify-full"] {
            assert!(modes.contains(mode), "TlsMode lacks {mode}");
        }
        assert!(schema["definitions"]["TlsConfig"]["properties"]
            .get("client_key")
            .is_some());
    }

    #[test]
    fn pool_numbers_are_bounded() {
        let base = "    driver: sqlite\n    database: a.db\n";
        assert!(entry_err(&format!("{base}    pool: {{ max: 0 }}\n"))
            .contains("`pool.max` must be 1-500"));
        assert!(
            entry_err(&format!("{base}    pool: {{ max: 1000000000 }}\n")).contains("`pool.max`")
        );
        assert!(
            entry_err(&format!("{base}    pool: {{ acquire_timeout_ms: 0 }}\n"))
                .contains("acquire_timeout_ms")
        );
        assert!(entry_err(&format!(
            "{base}    pool: {{ idle_timeout_ms: 999999999999 }}\n"
        ))
        .contains("idle_timeout_ms"));
        entry_ok(&format!(
            "{base}    pool: {{ max: 500, idle_timeout_ms: 0, acquire_timeout_ms: 600000 }}\n"
        ));
        let schema = WorkerConfig::json_schema();
        assert_eq!(
            schema["definitions"]["PoolConfig"]["properties"]["max"]["maximum"],
            500.0
        );
    }

    #[test]
    fn parent_segments_are_refused_in_folder_and_sqlite_paths() {
        assert!(entry_err("    driver: sqlite\n    database: ../../etc/x.db\n").contains("`..`"));
        assert!(
            entry_err("    driver: sqlite\n    database: a.db\n    folder: /srv/../etc\n")
                .contains("`..`")
        );
        entry_ok("    driver: sqlite\n    database: data/x..y.db\n    folder: /srv/q..s\n");
    }

    #[test]
    fn the_running_worker_sets_a_bad_entry_aside_instead_of_refusing_the_value() {
        let value = serde_json::json!({
            "databases": {
                "good": { "url": "sqlite::memory:" },
                "huge": { "url": "sqlite::memory:", "pool": { "max": 1000000000 } },
                "typo": { "driver": "postgres", "host": "h", "user": "u", "port": "hunter2" },
            },
            "history_max_entries": 3
        });
        let cfg = WorkerConfig::from_json_isolating(&value).unwrap();
        assert_eq!(cfg.databases.len(), 1);
        assert!(cfg.invalid["huge"].contains("`pool.max`"));
        assert!(cfg.invalid["typo"].contains("db `typo`"));
        assert!(
            !cfg.invalid["typo"].contains("hunter2"),
            "{}",
            cfg.invalid["typo"]
        );
        assert_eq!(cfg.history_max_entries, 3);
        // The strict parser still refuses the whole value.
        assert!(WorkerConfig::from_json(&value).is_err());
        // Nothing at all is still an error.
        assert!(
            WorkerConfig::from_json_isolating(&serde_json::json!({ "databases": {} })).is_err()
        );
    }
}
