//! From a configured entry to something that opens connections.
//!
//! A [`Connector`] is one connection entry with every reference resolved,
//! every certificate and key loaded, and (when `ssh` is set) its tunnel
//! running. Pools, the native-capture listeners and `testConnection` all
//! connect through one — so the url form, the structured form, secrets, TLS
//! modes and tunnels mean the same thing on every path.
//!
//! Building one is synchronous (the tunnel binds its local port and connects
//! lazily); [`Connector::check_tunnel`] opens the SSH session up front so a
//! bad password or host key becomes the connection's error instead of an
//! opaque pool failure later.
//!
//! [`Material`] is the entry's sensitive values with references resolved. It
//! lives only as long as a build, and every error message names the field
//! and (for references) the reference — never a value.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Serialize;
use tokio_postgres::config::SslMode;

use crate::config::{DatabaseConfig, DriverKind, TlsMode};
use crate::pool::tls::{make_mysql_ssl_opts, make_pg_connector, ResolvedTls};
use crate::secrets::{parse_ref, SecretError, SecretValue};
use crate::ssh::{SshSettings, SshTunnel, TunnelErrorKind};

/// Where an attempt to reach a database failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Stage {
    /// The entry itself is unusable (unparseable url, unreadable file).
    Config,
    /// A `secret://` / `env://` reference did not resolve.
    Secrets,
    /// The SSH tunnel: server unreachable, host key, authentication.
    Ssh,
    /// The database server (or the tunnel's far end) is unreachable.
    Tcp,
    /// TLS negotiation or certificate validation.
    Tls,
    /// The database refused the credentials.
    Auth,
    /// Connected and authenticated, but the database is missing or unusable.
    Database,
}

/// A connection failure that is safe to show: it names fields, references,
/// hosts and users, never a password, key or passphrase.
#[derive(Debug, Clone, Serialize, JsonSchema)]
pub struct ConnectError {
    pub stage: Stage,
    pub message: String,
}

impl ConnectError {
    pub fn new(stage: Stage, message: impl Into<String>) -> Self {
        Self {
            stage,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl From<ConnectError> for crate::error::DbError {
    fn from(e: ConnectError) -> Self {
        crate::error::DbError::ConfigError { message: e.message }
    }
}

/// The canonical `secret://` / `env://` references an entry uses.
pub fn references(db: &DatabaseConfig) -> Vec<String> {
    db.sensitive_fields()
        .into_iter()
        .filter_map(|(_, v)| parse_ref(v).and_then(Result::ok).map(|r| r.key()))
        .collect()
}

/// One sensitive field's usable value.
struct Resolved {
    value: SecretValue,
    /// The reference it came from, for messages (`None` for a literal).
    reference: Option<String>,
}

/// An entry's sensitive values with references resolved.
pub struct Material {
    values: HashMap<&'static str, Resolved>,
}

impl std::fmt::Debug for Material {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut fields: Vec<_> = self.values.keys().collect();
        fields.sort();
        f.debug_struct("Material").field("fields", &fields).finish()
    }
}

impl Material {
    /// Resolve through `lookup` (a [`crate::secrets::SecretCache`] in the
    /// worker). A reference the cache has not seen yet counts as
    /// unavailable.
    pub fn resolve(
        db: &DatabaseConfig,
        lookup: impl Fn(&str) -> Option<Result<SecretValue, SecretError>>,
    ) -> Result<Self, ConnectError> {
        let mut values = HashMap::new();
        for (field, raw) in db.sensitive_fields() {
            let resolved = match parse_ref(raw) {
                None => Resolved {
                    value: SecretValue::new(raw.to_string()),
                    reference: None,
                },
                Some(Err(e)) => {
                    return Err(ConnectError::new(
                        Stage::Secrets,
                        format!("`{field}` {}", e.describe_malformed()),
                    ))
                }
                Some(Ok(reference)) => {
                    let key = reference.key();
                    match lookup(&key).unwrap_or(Err(SecretError::Unavailable)) {
                        Ok(value) => Resolved {
                            value,
                            reference: Some(key),
                        },
                        Err(e) => {
                            return Err(ConnectError::new(
                                Stage::Secrets,
                                format!("`{field}`: {}", e.describe(&key)),
                            ))
                        }
                    }
                }
            };
            values.insert(field, resolved);
        }
        Ok(Self { values })
    }

    /// Literal values only; any reference fails (tests, offline tools).
    pub fn literal(db: &DatabaseConfig) -> Result<Self, ConnectError> {
        Self::resolve(db, |_| None)
    }

    fn get(&self, field: &str) -> Option<&Resolved> {
        self.values.get(field)
    }

    fn value(&self, field: &str) -> Option<&str> {
        self.get(field).map(|r| r.value.expose())
    }

    /// PEM material from inline text, a resolved reference, or a file path.
    fn pem(&self, field: &str) -> Result<Option<Vec<u8>>, ConnectError> {
        let Some(resolved) = self.get(field) else {
            return Ok(None);
        };
        let text = resolved.value.expose();
        if text.contains("-----BEGIN") {
            return Ok(Some(text.as_bytes().to_vec()));
        }
        if let Some(reference) = &resolved.reference {
            // A resolved value is never read as a path: an error would
            // echo it.
            return Err(ConnectError::new(
                Stage::Config,
                format!("`{field}`: {reference} does not hold PEM text"),
            ));
        }
        let path = iii_worker_paths::resolve_path(text.trim());
        // Never echo the value: what looked like a path may be key material.
        std::fs::read(&path).map(Some).map_err(|e| {
            ConnectError::new(
                Stage::Config,
                format!("`{field}`: cannot read the file ({e})"),
            )
        })
    }

    /// The literal credentials in play (passwords, passphrase), for
    /// scrubbing driver error texts.
    pub fn credentials(&self) -> Vec<String> {
        ["password", "ssh.password", "ssh.passphrase"]
            .iter()
            .filter_map(|f| self.value(f))
            .filter(|v| !v.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// Hash of every value, for change detection. In-process only.
    fn digest(&self, hasher: &mut impl Hasher) {
        let mut fields: Vec<_> = self.values.iter().collect();
        fields.sort_by_key(|(k, _)| **k);
        for (field, resolved) in fields {
            field.hash(hasher);
            resolved.value.expose().hash(hasher);
        }
    }
}

/// What a connector connects with, per driver.
pub(crate) enum Kind {
    Sqlite {
        url: String,
    },
    Postgres {
        config: Box<tokio_postgres::Config>,
        tls: Option<tokio_postgres_rustls::MakeRustlsConnect>,
    },
    Mysql {
        /// With TLS (`None` for `disable`).
        tls: Option<Box<mysql_async::OptsBuilder>>,
        /// Plaintext (`disable`, and `prefer`'s fallback).
        plain: Option<Box<mysql_async::OptsBuilder>>,
    },
}

/// One connection entry, ready to connect.
pub struct Connector {
    pub name: String,
    pub driver: DriverKind,
    pub tls_mode: TlsMode,
    /// Changes whenever the entry or any resolved value changes.
    pub fingerprint: u64,
    pub(crate) kind: Kind,
    tunnel: Option<SshTunnel>,
}

impl std::fmt::Debug for Connector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Connector")
            .field("name", &self.name)
            .field("driver", &self.driver)
            .field("tls_mode", &self.tls_mode)
            .field("tunnel", &self.tunnel)
            .finish()
    }
}

/// The change-detection hash of an entry and its resolved values.
pub fn fingerprint(db: &DatabaseConfig, material: &Material) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(db)
        .unwrap_or_default()
        .hash(&mut hasher);
    material.digest(&mut hasher);
    hasher.finish()
}

impl Connector {
    /// Build from a validated entry (its `driver` detected) and its
    /// resolved values. Must run inside a tokio runtime when `ssh` is set.
    pub fn build(
        name: &str,
        db: &DatabaseConfig,
        material: &Material,
    ) -> Result<Self, ConnectError> {
        let fingerprint = fingerprint(db, material);
        let tls = resolve_tls(db, material)?;
        let tls_mode = tls.mode;
        let (kind, tunnel) = match db.driver {
            DriverKind::Sqlite => (
                Kind::Sqlite {
                    url: db.sqlite_url().unwrap_or_default(),
                },
                None,
            ),
            DriverKind::Postgres => {
                let (mut config, explicit_sslmode) = pg_config(db, material)?;
                let tunnel = start_tunnel(db, material)?;
                if let Some(t) = &tunnel {
                    let (host, _) = db.server_endpoint().unwrap_or_default();
                    config = retarget_pg(&config, &host, t.local_addr().port());
                }
                // The url's own `sslmode` is the operator's explicit choice
                // and stays; otherwise the tls block decides, and `require`
                // really requires (tokio-postgres alone would default to
                // `prefer` and silently accept a server that refuses TLS).
                if !explicit_sslmode {
                    config.ssl_mode(match tls.mode {
                        TlsMode::Disable => SslMode::Disable,
                        TlsMode::Prefer => SslMode::Prefer,
                        TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull => {
                            SslMode::Require
                        }
                    });
                }
                let connector = make_pg_connector(&tls)
                    .map_err(|e| ConnectError::new(Stage::Tls, db_error_message(&e)))?;
                (
                    Kind::Postgres {
                        config: Box::new(config),
                        tls: connector,
                    },
                    tunnel,
                )
            }
            DriverKind::Mysql => {
                let mut base = mysql_opts(db, material)?;
                let tunnel = start_tunnel(db, material)?;
                let mut tls_host = None;
                if let Some(t) = &tunnel {
                    base = base
                        .ip_or_hostname("127.0.0.1")
                        .tcp_port(t.local_addr().port())
                        .prefer_socket(false);
                    tls_host = db.server_endpoint().map(|(h, _)| h);
                }
                let ssl = make_mysql_ssl_opts(&tls, tls_host.as_deref())
                    .map_err(|e| ConnectError::new(Stage::Tls, db_error_message(&e)))?;
                let with_tls = ssl.map(|ssl| Box::new(base.clone().ssl_opts(ssl)));
                let plain =
                    matches!(tls.mode, TlsMode::Disable | TlsMode::Prefer).then(|| Box::new(base));
                (
                    Kind::Mysql {
                        tls: with_tls,
                        plain,
                    },
                    tunnel,
                )
            }
        };
        Ok(Self {
            name: name.to_string(),
            driver: db.driver,
            tls_mode,
            fingerprint,
            kind,
            tunnel,
        })
    }

    /// Build with literal values only (no secrets worker at hand).
    pub fn literal(name: &str, db: &DatabaseConfig) -> Result<Self, ConnectError> {
        Self::build(name, db, &Material::literal(db)?)
    }

    pub fn tunnel(&self) -> Option<&SshTunnel> {
        self.tunnel.as_ref()
    }

    /// Open the SSH session now, when there is a tunnel.
    pub async fn check_tunnel(&self) -> Result<(), ConnectError> {
        let Some(tunnel) = &self.tunnel else {
            return Ok(());
        };
        tunnel.ensure_connected().await.map_err(|e| {
            let stage = match e.kind {
                TunnelErrorKind::Forward => Stage::Tcp,
                _ => Stage::Ssh,
            };
            ConnectError::new(stage, e.message)
        })
    }

    /// Postgres connection settings and TLS connector.
    pub(crate) fn pg(
        &self,
    ) -> Option<(
        &tokio_postgres::Config,
        Option<&tokio_postgres_rustls::MakeRustlsConnect>,
    )> {
        match &self.kind {
            Kind::Postgres { config, tls } => Some((config, tls.as_ref())),
            _ => None,
        }
    }

    /// MySQL options: `(with TLS, plaintext)`.
    pub(crate) fn mysql(
        &self,
    ) -> Option<(
        Option<&mysql_async::OptsBuilder>,
        Option<&mysql_async::OptsBuilder>,
    )> {
        match &self.kind {
            Kind::Mysql { tls, plain } => Some((tls.as_deref(), plain.as_deref())),
            _ => None,
        }
    }

    pub(crate) fn sqlite_url(&self) -> Option<&str> {
        match &self.kind {
            Kind::Sqlite { url } => Some(url),
            _ => None,
        }
    }

    /// The schema a mysql connector targets.
    pub(crate) fn mysql_db_name(&self) -> Option<String> {
        let (tls, plain) = self.mysql()?;
        let opts: mysql_async::Opts = tls.or(plain)?.clone().into();
        opts.db_name().map(str::to_string)
    }

    /// One dedicated mysql connection, honouring `prefer`: TLS first, and
    /// plaintext only when the server does not offer TLS.
    pub(crate) async fn mysql_connect(&self) -> Result<mysql_async::Conn, mysql_async::Error> {
        let Some((tls, plain)) = self.mysql() else {
            return Err(mysql_async::Error::Driver(
                mysql_async::DriverError::ConnectionClosed,
            ));
        };
        match (tls, plain) {
            (Some(tls), Some(plain)) => match mysql_async::Conn::new(tls.clone()).await {
                Err(mysql_async::Error::Driver(
                    mysql_async::DriverError::NoClientSslFlagFromServer,
                )) => mysql_async::Conn::new(plain.clone()).await,
                other => other,
            },
            (Some(opts), None) | (None, Some(opts)) => mysql_async::Conn::new(opts.clone()).await,
            (None, None) => Err(mysql_async::Error::Driver(
                mysql_async::DriverError::ConnectionClosed,
            )),
        }
    }
}

fn db_error_message(e: &crate::error::DbError) -> String {
    match e {
        crate::error::DbError::ConfigError { message } => message.clone(),
        other => other.to_string(),
    }
}

/// TLS settings with certificates and keys loaded.
fn resolve_tls(db: &DatabaseConfig, material: &Material) -> Result<ResolvedTls, ConnectError> {
    if db.driver == DriverKind::Sqlite || db.tls.mode == TlsMode::Disable {
        return Ok(ResolvedTls::mode(db.tls.mode));
    }
    let tls_err = |e: ConnectError| ConnectError::new(Stage::Tls, e.message);
    let client_key_pem = material
        .pem("tls.client_key")
        .map_err(tls_err)?
        .map(|bytes| SecretValue::new(String::from_utf8(bytes).unwrap_or_default()));
    Ok(ResolvedTls {
        mode: db.tls.mode,
        trust_native: db.tls.trust_native,
        ca_pem: material.pem("tls.ca_cert").map_err(tls_err)?,
        client_cert_pem: material.pem("tls.client_cert").map_err(tls_err)?,
        client_key_pem,
    })
}

/// The tokio-postgres settings of an entry, and whether its url carries an
/// explicit `sslmode`.
fn pg_config(
    db: &DatabaseConfig,
    material: &Material,
) -> Result<(tokio_postgres::Config, bool), ConnectError> {
    if let Some(url) = &db.url {
        // Never echo the parse error: it quotes the url, password included.
        let config: tokio_postgres::Config = url.parse().map_err(|_| {
            ConnectError::new(
                Stage::Config,
                "postgres url could not be parsed; check the configured url",
            )
        })?;
        let explicit = url::Url::parse(url)
            .map(|u| {
                u.query_pairs()
                    .any(|(k, _)| k.eq_ignore_ascii_case("sslmode"))
            })
            .unwrap_or(false);
        return Ok((config, explicit));
    }
    let mut config = tokio_postgres::Config::new();
    let (host, port) = db.server_endpoint().unwrap_or_default();
    config.host(&host).port(port);
    if let Some(user) = &db.user {
        config.user(user);
    }
    if let Some(password) = material.value("password") {
        config.password(password);
    }
    if let Some(database) = db.database.as_deref().filter(|d| !d.is_empty()) {
        config.dbname(database);
    }
    config.application_name("iii-database");
    Ok((config, false))
}

/// The same settings, connecting to the tunnel's local port while TLS keeps
/// verifying the database host name.
fn retarget_pg(src: &tokio_postgres::Config, host: &str, port: u16) -> tokio_postgres::Config {
    let mut c = tokio_postgres::Config::new();
    c.host(host)
        .hostaddr(IpAddr::V4(Ipv4Addr::LOCALHOST))
        .port(port);
    if let Some(user) = src.get_user() {
        c.user(user);
    }
    if let Some(password) = src.get_password() {
        c.password(password);
    }
    if let Some(dbname) = src.get_dbname() {
        c.dbname(dbname);
    }
    if let Some(options) = src.get_options() {
        c.options(options);
    }
    if let Some(name) = src.get_application_name() {
        c.application_name(name);
    }
    if let Some(timeout) = src.get_connect_timeout() {
        c.connect_timeout(*timeout);
    }
    c.ssl_mode(src.get_ssl_mode())
        .keepalives(src.get_keepalives())
        .keepalives_idle(src.get_keepalives_idle())
        .target_session_attrs(src.get_target_session_attrs())
        .channel_binding(src.get_channel_binding());
    c
}

/// The mysql_async options of an entry (pool options added by the pool).
fn mysql_opts(
    db: &DatabaseConfig,
    material: &Material,
) -> Result<mysql_async::OptsBuilder, ConnectError> {
    if let Some(url) = &db.url {
        // Don't echo the parse error: it includes the url verbatim.
        let opts = mysql_async::Opts::from_url(url).map_err(|_| {
            ConnectError::new(
                Stage::Config,
                "mysql url could not be parsed; check the configured url",
            )
        })?;
        // Never hop from TCP to a local socket the server names: it may be
        // a different server, and a socket connection skips TLS.
        return Ok(mysql_async::OptsBuilder::from_opts(opts).prefer_socket(false));
    }
    let (host, port) = db.server_endpoint().unwrap_or_default();
    Ok(mysql_async::OptsBuilder::default()
        .ip_or_hostname(host)
        .tcp_port(port)
        .user(db.user.clone())
        .pass(material.value("password").map(str::to_string))
        .db_name(db.database.clone().filter(|d| !d.is_empty()))
        // An explicit host means TCP: never hop to a local socket that may
        // belong to a different server (and would skip TLS).
        .prefer_socket(false))
}

fn start_tunnel(
    db: &DatabaseConfig,
    material: &Material,
) -> Result<Option<SshTunnel>, ConnectError> {
    let Some(ssh) = &db.ssh else {
        return Ok(None);
    };
    let (target_host, target_port) = db.server_endpoint().ok_or_else(|| {
        ConnectError::new(Stage::Config, "`ssh` needs the database host and port")
    })?;
    let private_key = match material.get("ssh.private_key") {
        None => None,
        Some(resolved) => {
            let text = resolved.value.expose();
            let pem = if text.contains("-----BEGIN") || text.starts_with("PuTTY-User-Key-File-") {
                SecretValue::new(text.to_string())
            } else if let Some(reference) = &resolved.reference {
                return Err(ConnectError::new(
                    Stage::Ssh,
                    format!("`ssh.private_key`: {reference} does not hold a private key"),
                ));
            } else {
                let path = iii_worker_paths::resolve_path(text.trim());
                let bytes = std::fs::read(&path).map_err(|e| {
                    ConnectError::new(
                        Stage::Ssh,
                        format!("`ssh.private_key`: cannot read the key file ({e})"),
                    )
                })?;
                SecretValue::new(String::from_utf8(bytes).map_err(|_| {
                    ConnectError::new(Stage::Ssh, "`ssh.private_key`: the file is not text (PEM)")
                })?)
            };
            let key =
                crate::ssh::decode_private_key(pem.expose(), material.value("ssh.passphrase"))
                    .map_err(|e| ConnectError::new(Stage::Ssh, e))?;
            Some(Arc::new(key))
        }
    };
    let settings = SshSettings {
        host: ssh.host.clone(),
        port: ssh.port,
        user: ssh.user.clone(),
        password: material
            .value("ssh.password")
            .map(|p| SecretValue::new(p.to_string())),
        private_key,
        host_key: ssh.host_key.clone(),
        target_host,
        target_port,
    };
    SshTunnel::start(settings).map(Some).map_err(|e| {
        ConnectError::new(
            Stage::Ssh,
            format!("cannot open the tunnel's local port: {e}"),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::secrets::testing::FakeSecrets;

    fn entry(yaml: &str) -> DatabaseConfig {
        let cfg =
            crate::config::WorkerConfig::from_yaml(&format!("databases:\n  x:\n{yaml}")).unwrap();
        cfg.databases["x"].clone()
    }

    #[tokio::test]
    async fn references_resolve_through_the_cache_and_failures_name_the_reference() {
        let db = entry(
            "    driver: postgres\n    host: db\n    user: app\n    password: secret://DB_PASS\n    tls: { mode: disable }\n",
        );
        assert_eq!(references(&db), vec!["secret://DB_PASS".to_string()]);

        let fake = FakeSecrets::default();
        let cache = fake.cache();
        cache.ensure(references(&db)).await;
        let err = Material::resolve(&db, |k| cache.lookup(k)).unwrap_err();
        assert_eq!(err.stage, Stage::Secrets);
        assert!(err.message.contains("secret://DB_PASS"), "{}", err.message);

        fake.set("secret://DB_PASS", Ok("s3cr3t-value"));
        cache
            .refresh(references(&db), crate::secrets::Refetch::Invalidate)
            .await;
        let material = Material::resolve(&db, |k| cache.lookup(k)).unwrap();
        let connector = Connector::build("x", &db, &material).unwrap();
        let (config, tls) = connector.pg().unwrap();
        assert_eq!(config.get_password(), Some(&b"s3cr3t-value"[..]));
        assert!(tls.is_none());
        assert_eq!(config.get_ssl_mode(), SslMode::Disable);
        // Neither the connector nor its config prints the value.
        assert!(!format!("{connector:?}").contains("s3cr3t-value"));
    }

    #[test]
    fn a_rotated_secret_changes_the_fingerprint() {
        let db = entry("    driver: mysql\n    host: db\n    user: app\n    password: env://P\n");
        let a = Material::resolve(&db, |_| Some(Ok(SecretValue::new("one".into())))).unwrap();
        let b = Material::resolve(&db, |_| Some(Ok(SecretValue::new("two".into())))).unwrap();
        assert_ne!(fingerprint(&db, &a), fingerprint(&db, &b));
        let a2 = Material::resolve(&db, |_| Some(Ok(SecretValue::new("one".into())))).unwrap();
        assert_eq!(fingerprint(&db, &a), fingerprint(&db, &a2));
    }

    #[test]
    fn require_really_requires_tls_unless_the_url_says_otherwise() {
        let db = entry("    url: postgres://u@h/db\n");
        let c = Connector::literal("x", &db).unwrap();
        assert_eq!(c.pg().unwrap().0.get_ssl_mode(), SslMode::Require);

        let db = entry("    url: postgres://u@h/db\n    tls: { mode: prefer }\n");
        let c = Connector::literal("x", &db).unwrap();
        assert_eq!(c.pg().unwrap().0.get_ssl_mode(), SslMode::Prefer);

        // An explicit sslmode in the url is the operator's choice.
        let db = entry("    url: postgres://u@h/db?sslmode=disable\n");
        let c = Connector::literal("x", &db).unwrap();
        assert_eq!(c.pg().unwrap().0.get_ssl_mode(), SslMode::Disable);
    }

    #[test]
    fn structured_postgres_and_mysql_build_their_driver_settings() {
        let db = entry(
            "    driver: postgres\n    host: db.example\n    port: 6543\n    user: app\n    password: pw\n    database: shop\n",
        );
        let c = Connector::literal("x", &db).unwrap();
        let (config, tls) = c.pg().unwrap();
        assert_eq!(config.get_ports(), &[6543]);
        assert_eq!(config.get_dbname(), Some("shop"));
        assert_eq!(config.get_user(), Some("app"));
        assert!(tls.is_some(), "require builds a TLS connector");

        let db = entry(
            "    driver: mysql\n    host: my.example\n    user: app\n    password: pw\n    database: shop\n    tls: { mode: prefer }\n",
        );
        let c = Connector::literal("x", &db).unwrap();
        let (tls, plain) = c.mysql().unwrap();
        assert!(tls.is_some() && plain.is_some(), "prefer keeps both");
        let opts: mysql_async::Opts = plain.unwrap().clone().into();
        assert_eq!(opts.ip_or_hostname(), "my.example");
        assert_eq!(opts.tcp_port(), 3306);
        assert_eq!(opts.db_name(), Some("shop"));
        assert!(!opts.prefer_socket());
        assert_eq!(c.mysql_db_name().as_deref(), Some("shop"));
    }

    #[test]
    fn literal_material_refuses_references() {
        let db =
            entry("    driver: postgres\n    host: h\n    user: u\n    password: secret://P\n");
        let err = Connector::literal("x", &db).unwrap_err();
        assert_eq!(err.stage, Stage::Secrets);
    }

    #[test]
    fn pem_values_come_inline_from_a_path_never_from_a_referenced_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ca.pem");
        std::fs::write(&path, "not a cert").unwrap();
        let db = entry(&format!(
            "    driver: postgres\n    host: h\n    user: u\n    tls:\n      mode: require\n      ca_cert: {}\n",
            path.display()
        ));
        let err = Connector::literal("x", &db).unwrap_err();
        assert_eq!(err.stage, Stage::Tls);
        assert!(
            err.message.contains("no PEM CERTIFICATE"),
            "{}",
            err.message
        );

        let db = entry(
            "    driver: postgres\n    host: h\n    user: u\n    tls:\n      mode: require\n      ca_cert: /no/such/ca.pem\n",
        );
        let err = Connector::literal("x", &db).unwrap_err();
        assert!(
            err.message.contains("cannot read the file"),
            "{}",
            err.message
        );
        assert!(
            !err.message.contains("/no/such/ca.pem"),
            "never echo what may be key material"
        );

        // A referenced value that is not PEM is never treated as a path (the
        // error would print it).
        let db = entry(
            "    driver: postgres\n    host: h\n    user: u\n    tls:\n      mode: require\n      ca_cert: secret://CA\n",
        );
        let material =
            Material::resolve(&db, |_| Some(Ok(SecretValue::new("/etc/hunter2".into())))).unwrap();
        let err = Connector::build("x", &db, &material).unwrap_err();
        assert!(!err.message.contains("hunter2"), "{}", err.message);
        assert!(err.message.contains("secret://CA"), "{}", err.message);
    }

    #[tokio::test]
    async fn a_tunnel_retargets_postgres_to_localhost_but_keeps_the_tls_host() {
        let db = entry(
            "    driver: postgres\n    host: pg-private\n    user: app\n    ssh:\n      host: 127.0.0.1\n      port: 1\n      user: ops\n      password: pw\n",
        );
        let c = Connector::literal("x", &db).unwrap();
        let (config, _) = c.pg().unwrap();
        let tunnel = c.tunnel().unwrap();
        assert_eq!(config.get_ports(), &[tunnel.local_addr().port()]);
        assert_eq!(config.get_hostaddrs(), &[IpAddr::V4(Ipv4Addr::LOCALHOST)]);
        assert!(matches!(
            &config.get_hosts()[0],
            tokio_postgres::config::Host::Tcp(h) if h == "pg-private"
        ));
        let err = c.check_tunnel().await.unwrap_err();
        assert_eq!(err.stage, Stage::Ssh);
    }
}
