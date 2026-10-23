//! SSH tunnels for postgres/mysql connections behind a bastion.
//!
//! A tunnel listens on an ephemeral `127.0.0.1` port and forwards every
//! accepted connection through a `direct-tcpip` channel of ONE SSH session to
//! the database host/port as the SSH server resolves them. The pool (and the
//! native-capture listener) connect to the local port; TLS to the database is
//! negotiated end to end through the channel and still verifies the database
//! host name (postgres: `host` + `hostaddr`; mysql: a TLS hostname override).
//!
//! The session is opened eagerly by [`SshTunnel::ensure_connected`] (so a
//! wrong password or host key surfaces as the connection's error, not as an
//! opaque pool failure later) and reopened on demand when it drops: the next
//! accepted connection reconnects once before giving up.
//!
//! Host keys: with `ssh.host_key` set, a server presenting any other key is
//! refused and the error names both fingerprints; without it every key is
//! accepted (trust on first use) and the key seen is kept for
//! `database::testConnection` to report so a client can pin it.
//!
//! Nothing here logs or returns a password, passphrase or private key; errors
//! carry the SSH host, the user and russh's own (credential-free) reasons.

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKeyOrCertificate};
use schemars::JsonSchema;
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::secrets::SecretValue;

/// TCP connect + key exchange + authentication budget for one session.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Keepalive cadence; a session that misses [`KEEPALIVE_MAX`] in a row is
/// dropped and reopened on the next connection.
const KEEPALIVE_INTERVAL: Duration = Duration::from_secs(30);
const KEEPALIVE_MAX: usize = 3;

/// The server key a session saw, in OpenSSH notation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, JsonSchema)]
pub struct HostKeyInfo {
    /// `ssh-ed25519`, `ecdsa-sha2-nistp256`, `ssh-rsa`, …
    pub algorithm: String,
    /// `SHA256:…`, as `ssh-keygen -lf` prints it — the value `ssh.host_key`
    /// pins.
    pub fingerprint: String,
}

/// Resolved tunnel settings (secrets already looked up).
pub struct SshSettings {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub password: Option<SecretValue>,
    pub private_key: Option<Arc<PrivateKey>>,
    pub host_key: Option<String>,
    /// Where the SSH server forwards to: the database as it sees it.
    pub target_host: String,
    pub target_port: u16,
}

impl std::fmt::Debug for SshSettings {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshSettings")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("user", &self.user)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field(
                "private_key",
                &self.private_key.as_ref().map(|_| "<redacted>"),
            )
            .field("host_key", &self.host_key)
            .field("target_host", &self.target_host)
            .field("target_port", &self.target_port)
            .finish()
    }
}

/// What went wrong, by the step that failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelErrorKind {
    /// TCP to the SSH server, the SSH handshake, or a dropped session.
    Connect,
    /// The server's host key is not the pinned one.
    HostKey,
    /// The server refused every credential offered.
    Auth,
    /// The session is up but the server could not reach the database.
    Forward,
}

#[derive(Debug, Clone)]
pub struct TunnelError {
    pub kind: TunnelErrorKind,
    pub message: String,
}

impl std::fmt::Display for TunnelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl TunnelError {
    fn new(kind: TunnelErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

/// The host-key check russh calls during key exchange.
struct Checker {
    pin: Option<String>,
    seen: Arc<Mutex<Option<HostKeyInfo>>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &PublicKeyOrCertificate,
    ) -> Result<bool, Self::Error> {
        let key = match server_public_key {
            PublicKeyOrCertificate::PublicKey { key, .. } => key.clone(),
            PublicKeyOrCertificate::Certificate(cert) => cert.public_key().clone().into(),
        };
        let info = HostKeyInfo {
            algorithm: key.algorithm().to_string(),
            fingerprint: key.fingerprint(HashAlg::Sha256).to_string(),
        };
        let accepted = self
            .pin
            .as_deref()
            .is_none_or(|pin| pin == info.fingerprint);
        *self.seen.lock().unwrap_or_else(|e| e.into_inner()) = Some(info);
        Ok(accepted)
    }
}

struct Inner {
    settings: SshSettings,
    session: tokio::sync::Mutex<Option<Arc<Handle<Checker>>>>,
    seen: Arc<Mutex<Option<HostKeyInfo>>>,
    last_error: Mutex<Option<(Instant, TunnelError)>>,
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Close the session politely when the last user goes away; dropping
        // the handle alone also ends it.
        if let Some(handle) = self.session.get_mut().take() {
            if let Ok(rt) = tokio::runtime::Handle::try_current() {
                rt.spawn(async move {
                    let _ = handle
                        .disconnect(russh::Disconnect::ByApplication, "", "en")
                        .await;
                });
            }
        }
    }
}

/// A running tunnel. Dropping it stops accepting; connections already
/// forwarded keep their channel until they close.
pub struct SshTunnel {
    inner: Arc<Inner>,
    local: SocketAddr,
    accept: JoinHandle<()>,
}

impl Drop for SshTunnel {
    fn drop(&mut self) {
        self.accept.abort();
    }
}

impl std::fmt::Debug for SshTunnel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshTunnel")
            .field(
                "ssh",
                &format_args!(
                    "{}@{}:{}",
                    self.inner.settings.user, self.inner.settings.host, self.inner.settings.port
                ),
            )
            .field("local", &self.local)
            .finish()
    }
}

impl SshTunnel {
    /// Bind the local port and start accepting. Must run inside a tokio
    /// runtime. No SSH traffic happens until a connection arrives or
    /// [`SshTunnel::ensure_connected`] is called.
    pub fn start(settings: SshSettings) -> std::io::Result<Self> {
        let std_listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
        std_listener.set_nonblocking(true)?;
        let local = std_listener.local_addr()?;
        let listener = tokio::net::TcpListener::from_std(std_listener)?;
        let inner = Arc::new(Inner {
            settings,
            session: tokio::sync::Mutex::new(None),
            seen: Arc::new(Mutex::new(None)),
            last_error: Mutex::new(None),
        });
        // The accept loop holds a Weak: a pool dropping its tunnel must free
        // the session even while the loop is parked in accept().
        let weak = Arc::downgrade(&inner);
        let accept = tokio::spawn(async move {
            loop {
                let Ok((socket, peer)) = listener.accept().await else {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                    continue;
                };
                let Some(inner) = weak.upgrade() else { return };
                tokio::spawn(forward(inner, socket, peer));
            }
        });
        Ok(Self {
            inner,
            local,
            accept,
        })
    }

    /// The local address database clients connect to.
    pub fn local_addr(&self) -> SocketAddr {
        self.local
    }

    /// Open the session now (or confirm it is up).
    pub async fn ensure_connected(&self) -> Result<(), TunnelError> {
        session(&self.inner).await.map(|_| ())
    }

    /// The server key the last handshake saw.
    pub fn host_key(&self) -> Option<HostKeyInfo> {
        self.inner
            .seen
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The most recent forwarding failure at or after `since`.
    pub fn error_since(&self, since: Instant) -> Option<TunnelError> {
        self.inner
            .last_error
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .filter(|(at, _)| *at >= since)
            .map(|(_, e)| e.clone())
    }

    /// `user@host:port` of the SSH server, for messages.
    pub fn describe(&self) -> String {
        let s = &self.inner.settings;
        format!("{}@{}:{}", s.user, s.host, s.port)
    }
}

/// The live session, opening one when there is none or the last one closed.
async fn session(inner: &Arc<Inner>) -> Result<Arc<Handle<Checker>>, TunnelError> {
    let mut guard = inner.session.lock().await;
    if let Some(handle) = guard.as_ref() {
        if !handle.is_closed() {
            return Ok(handle.clone());
        }
    }
    *guard = None;
    let handle = match tokio::time::timeout(CONNECT_TIMEOUT, connect(inner)).await {
        Ok(result) => result,
        Err(_) => Err(TunnelError::new(
            TunnelErrorKind::Connect,
            format!(
                "timed out after {}s connecting to the SSH server {}:{}",
                CONNECT_TIMEOUT.as_secs(),
                inner.settings.host,
                inner.settings.port
            ),
        )),
    };
    match handle {
        Ok(handle) => {
            let handle = Arc::new(handle);
            *guard = Some(handle.clone());
            Ok(handle)
        }
        Err(e) => {
            record(inner, &e);
            Err(e)
        }
    }
}

async fn connect(inner: &Arc<Inner>) -> Result<Handle<Checker>, TunnelError> {
    let s = &inner.settings;
    let config = Arc::new(client::Config {
        inactivity_timeout: None,
        keepalive_interval: Some(KEEPALIVE_INTERVAL),
        keepalive_max: KEEPALIVE_MAX,
        nodelay: true,
        ..Default::default()
    });
    *inner.seen.lock().unwrap_or_else(|e| e.into_inner()) = None;
    let checker = Checker {
        pin: s.host_key.clone(),
        seen: inner.seen.clone(),
    };
    let mut handle = match client::connect(config, (s.host.as_str(), s.port), checker).await {
        Ok(handle) => handle,
        Err(russh::Error::UnknownKey) => {
            let seen = inner.seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
            let presented = seen.map_or_else(
                || "an unreadable key".to_string(),
                |k| format!("{} ({})", k.fingerprint, k.algorithm),
            );
            return Err(TunnelError::new(
                TunnelErrorKind::HostKey,
                format!(
                    "SSH host key mismatch for {}:{}: ssh.host_key pins {} but the server \
                     presented {presented}; if the server's key was rotated on purpose, pin the \
                     new fingerprint",
                    s.host,
                    s.port,
                    s.host_key.as_deref().unwrap_or("(none)")
                ),
            ));
        }
        Err(e) => {
            return Err(TunnelError::new(
                TunnelErrorKind::Connect,
                format!("cannot reach the SSH server {}:{}: {e}", s.host, s.port),
            ))
        }
    };

    let mut tried: Vec<&str> = Vec::new();
    let mut authenticated = false;
    if let Some(key) = &s.private_key {
        tried.push("publickey");
        let hash_alg = if key.algorithm().is_rsa() {
            // Servers that announce server-sig-algs say which RSA hash they
            // take; the rest get SHA-256 (OpenSSH has refused SHA-1 since 8.8).
            match handle.best_supported_rsa_hash().await {
                Ok(Some(alg)) => alg,
                _ => Some(HashAlg::Sha256),
            }
        } else {
            None
        };
        let result = handle
            .authenticate_publickey(
                s.user.clone(),
                PrivateKeyWithHashAlg::new(key.clone(), hash_alg),
            )
            .await
            .map_err(|e| auth_transport_error(s, e))?;
        authenticated = result.success();
    }
    if !authenticated {
        if let Some(password) = &s.password {
            tried.push("password");
            let result = handle
                .authenticate_password(s.user.clone(), password.expose())
                .await
                .map_err(|e| auth_transport_error(s, e))?;
            authenticated = result.success();
        }
    }
    if !authenticated {
        return Err(TunnelError::new(
            TunnelErrorKind::Auth,
            format!(
                "SSH authentication failed for {}@{}:{} (tried {})",
                s.user,
                s.host,
                s.port,
                tried.join(", ")
            ),
        ));
    }
    tracing::info!(ssh = %format_args!("{}@{}:{}", s.user, s.host, s.port), "ssh tunnel session open");
    Ok(handle)
}

fn auth_transport_error(s: &SshSettings, e: russh::Error) -> TunnelError {
    TunnelError::new(
        TunnelErrorKind::Connect,
        format!(
            "SSH session to {}:{} failed during authentication: {e}",
            s.host, s.port
        ),
    )
}

fn record(inner: &Inner, error: &TunnelError) {
    tracing::warn!(
        ssh = %format_args!("{}@{}:{}", inner.settings.user, inner.settings.host, inner.settings.port),
        error = %error.message,
        "ssh tunnel error"
    );
    *inner.last_error.lock().unwrap_or_else(|e| e.into_inner()) =
        Some((Instant::now(), error.clone()));
}

/// Pump one accepted local connection through a fresh channel.
async fn forward(inner: Arc<Inner>, mut socket: tokio::net::TcpStream, peer: SocketAddr) {
    let _ = socket.set_nodelay(true);
    let channel = match open_channel(&inner, peer).await {
        Ok(channel) => channel,
        Err(e) => {
            record(&inner, &e);
            return;
        }
    };
    let mut stream = channel.into_stream();
    let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
}

async fn open_channel(
    inner: &Arc<Inner>,
    peer: SocketAddr,
) -> Result<russh::Channel<client::Msg>, TunnelError> {
    let s = &inner.settings;
    for attempt in 0..2 {
        let handle = session(inner).await?;
        match handle
            .channel_open_direct_tcpip(
                s.target_host.clone(),
                u32::from(s.target_port),
                peer.ip().to_string(),
                u32::from(peer.port()),
            )
            .await
        {
            Ok(channel) => return Ok(channel),
            Err(russh::Error::ChannelOpenFailure(reason)) => {
                return Err(TunnelError::new(
                    TunnelErrorKind::Forward,
                    format!(
                        "the SSH server {}:{} could not open a connection to {}:{} ({})",
                        s.host,
                        s.port,
                        s.target_host,
                        s.target_port,
                        channel_failure_reason(&reason)
                    ),
                ))
            }
            Err(e) if attempt == 0 => {
                // A dead session that has not noticed yet: drop it and
                // reconnect once.
                tracing::debug!(error = %e, "ssh channel open failed; reconnecting the session");
                let mut guard = inner.session.lock().await;
                if guard.as_ref().is_some_and(|h| Arc::ptr_eq(h, &handle)) {
                    *guard = None;
                }
            }
            Err(e) => {
                return Err(TunnelError::new(
                    TunnelErrorKind::Connect,
                    format!("SSH session to {}:{} failed: {e}", s.host, s.port),
                ))
            }
        }
    }
    unreachable!("the loop returns on its second attempt")
}

fn channel_failure_reason(reason: &russh::ChannelOpenFailure) -> String {
    match reason {
        russh::ChannelOpenFailure::AdministrativelyProhibited => {
            "forwarding is not allowed for this user (AllowTcpForwarding?)".into()
        }
        russh::ChannelOpenFailure::ConnectFailed => "connect failed".into(),
        russh::ChannelOpenFailure::UnknownChannelType => "direct-tcpip not supported".into(),
        russh::ChannelOpenFailure::ResourceShortage => "resource shortage".into(),
        russh::ChannelOpenFailure::Other { code, reason } => format!("{reason} (code {code})"),
    }
}

/// Decode an OpenSSH or PEM private key. The error never quotes the key.
pub fn decode_private_key(pem: &str, passphrase: Option<&str>) -> Result<PrivateKey, String> {
    russh::keys::decode_secret_key(pem, passphrase).map_err(|e| {
        let encrypted_pem = pem.contains("ENCRYPTED");
        match (&e, passphrase.is_some()) {
            (russh::keys::Error::KeyIsEncrypted, false) => {
                "ssh.private_key is encrypted; set ssh.passphrase".to_string()
            }
            (_, false) if encrypted_pem => {
                "ssh.private_key is encrypted; set ssh.passphrase".to_string()
            }
            (_, true) => format!(
                "ssh.private_key could not be decoded with the given ssh.passphrase (wrong \
                 passphrase, or not an OpenSSH/PEM RSA, ECDSA or Ed25519 key): {}",
                key_error_reason(&e)
            ),
            _ => format!(
                "ssh.private_key could not be read (expected an OpenSSH or PEM RSA, ECDSA or \
                 Ed25519 key): {}",
                key_error_reason(&e)
            ),
        }
    })
}

/// russh's key errors name formats and algorithms, never key material; keep
/// the message short anyway.
fn key_error_reason(e: &russh::keys::Error) -> String {
    let text = e.to_string();
    text.chars().take(160).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A throwaway private key from the ssh-keygen CLI (`None` without it).
    pub(crate) fn keygen(kind: &str, passphrase: &str) -> Option<String> {
        let dir = tempfile::tempdir().ok()?;
        let path = dir.path().join("k");
        let ok = std::process::Command::new("ssh-keygen")
            .args(["-q", "-t", kind, "-N", passphrase, "-C", "test", "-f"])
            .arg(&path)
            .output()
            .ok()?
            .status
            .success();
        ok.then(|| std::fs::read_to_string(&path).ok()).flatten()
    }

    #[test]
    fn decodes_openssh_keys_and_explains_passphrase_problems_without_echoing_keys() {
        let Some(ed25519) = keygen("ed25519", "") else {
            eprintln!("skipping: ssh-keygen not available");
            return;
        };
        let key = decode_private_key(&ed25519, None).expect("ed25519 decodes");
        assert_eq!(key.algorithm().to_string(), "ssh-ed25519");
        for kind in ["rsa", "ecdsa"] {
            let pem = keygen(kind, "").unwrap();
            decode_private_key(&pem, None).unwrap_or_else(|e| panic!("{kind}: {e}"));
        }

        let encrypted = keygen("ed25519", "correct horse").unwrap();
        assert!(decode_private_key(&encrypted, Some("correct horse")).is_ok());
        let err = decode_private_key(&encrypted, Some("wrong")).unwrap_err();
        assert!(
            !err.contains("wrong") || err.contains("wrong passphrase"),
            "{err}"
        );
        let err = decode_private_key(&encrypted, None).unwrap_err();
        assert!(err.contains("passphrase"), "{err}");

        let body: String = ed25519.lines().nth(2).unwrap().to_string();
        let mangled = ed25519.replace(&body, &"A".repeat(body.len()));
        let err = decode_private_key(&mangled, None).unwrap_err();
        assert!(!err.contains(&body), "{err}");
    }

    #[test]
    fn settings_debug_redacts_credentials() {
        let settings = SshSettings {
            host: "bastion".into(),
            port: 22,
            user: "ops".into(),
            password: Some(SecretValue::new("hunter2".into())),
            private_key: None,
            host_key: None,
            target_host: "db".into(),
            target_port: 5432,
        };
        let dbg = format!("{settings:?}");
        assert!(!dbg.contains("hunter2"), "{dbg}");
    }

    #[tokio::test]
    async fn a_refused_ssh_server_is_a_connect_error_naming_the_server() {
        let tunnel = SshTunnel::start(SshSettings {
            host: "127.0.0.1".into(),
            port: 1,
            user: "ops".into(),
            password: Some(SecretValue::new("hunter2".into())),
            private_key: None,
            host_key: None,
            target_host: "db".into(),
            target_port: 5432,
        })
        .unwrap();
        let err = tunnel.ensure_connected().await.unwrap_err();
        assert_eq!(err.kind, TunnelErrorKind::Connect);
        assert!(err.message.contains("127.0.0.1:1"), "{}", err.message);
        assert!(!err.message.contains("hunter2"));
    }
}
