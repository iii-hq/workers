//! OpenTunnel, in-process. One identity (profile) per target id, kept across
//! leases and restarts, so the hostname is stable. Each target serves one
//! randomly named route, `<route>.<hostname>`, never the root route: the bare
//! hostname, which Certificate Transparency logs publish, answers nothing.
//! Visitor TLS ends here.
use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use opentunnel::{Client, ClientOptions, Error, Event, ProxyProtocol, Route, Routes, State};
use tokio::{
    sync::{broadcast::error::RecvError, oneshot, Mutex},
    task::JoinHandle,
};

use super::{Provider, Session, Signal, Sink};
use crate::{config::Config, parser, prerequisites::Prerequisites};

pub const DOMAIN: &str = ".opentunnel.xyz";
const STOP_TIMEOUT: Duration = Duration::from_secs(2);
const ENSURE_TIMEOUT: Duration = Duration::from_secs(600);
const STOPPED: &str = "opentunnel stopped";

/// `<state_dir>/opentunnel`, created `0700`. One readable by group or others
/// fails closed: it holds the certificate key and the tunnel token.
pub fn state_dir(parent: &Path) -> Result<PathBuf, String> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    let dir = parent.join("opentunnel");
    let created = std::fs::create_dir_all(parent).and_then(|()| {
        match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(()),
            other => other,
        }
    });
    let metadata = created
        .and_then(|()| std::fs::metadata(&dir))
        .map_err(|e| format!("opentunnel state directory {}: {e}", dir.display()))?;
    if !metadata.is_dir() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(format!(
            "opentunnel state directory {} must be a directory without group or other permissions (chmod 700)",
            dir.display()
        ));
    }
    Ok(dir)
}

pub struct OpenTunnel {
    client: Client,
    /// One ensure at a time: create is not idempotent, so a second one in
    /// flight for a profile would make a second server-side tunnel.
    ensuring: Arc<Mutex<()>>,
    proxied: BTreeSet<String>,
}

impl OpenTunnel {
    /// Tests pass a closed `api` port and a temporary `storage`.
    pub fn new(options: ClientOptions) -> Self {
        Self {
            client: Client::new(options),
            ensuring: Arc::default(),
            proxied: BTreeSet::new(),
        }
    }

    /// The provider a validated `opentunnel` config asks for.
    pub fn from_config(config: &Config) -> Result<Self, String> {
        let dir = state_dir(crate::manager::state_parent(&config.state_path))?;
        Ok(Self {
            proxied: config.proxy_protocol.clone(),
            ..Self::new(ClientOptions {
                api: None,
                storage: Some(opentunnel::Storage::at(dir)),
                ca_cert: None,
            })
        })
    }

    /// Whether `tunnel_id`'s route sends a PROXY protocol v2 header to the target.
    fn proxied(&self, tunnel_id: &str) -> bool {
        self.proxied.contains(tunnel_id)
    }
}

#[async_trait::async_trait]
impl Provider for OpenTunnel {
    fn name(&self) -> &'static str {
        "opentunnel"
    }

    fn start(
        &self,
        tunnel_id: &str,
        origin: &str,
        _state_dir: &Path,
        sink: Sink,
    ) -> Result<Box<dyn Session>, String> {
        // From the validated allowlist origin, never from a caller.
        let target = route_target(origin).ok_or("opentunnel provisioning failed")?;
        let proxy = self.proxied(tunnel_id);
        let (shutdown, stop) = oneshot::channel();
        let task = tokio::spawn(run(
            self.client.clone(),
            self.ensuring.clone(),
            tunnel_id.to_owned(),
            target,
            proxy,
            sink,
            stop,
        ));
        Ok(Box::new(Running {
            task,
            shutdown: Some(shutdown),
        }))
    }

    async fn prerequisites(&self) -> Option<Prerequisites> {
        None
    }
}

struct Running {
    task: JoinHandle<()>,
    shutdown: Option<oneshot::Sender<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Final safety net if the owning task is cancelled; dropping a Tunnel stops it.
        self.task.abort();
    }
}

#[async_trait::async_trait]
impl Session for Running {
    fn exited(&mut self) -> Option<String> {
        // The task holds after a Fatal until stopped, so this only sees a crash.
        self.task
            .is_finished()
            .then(|| "opentunnel task ended".into())
    }

    async fn stop(mut self: Box<Self>) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        if tokio::time::timeout(STOP_TIMEOUT, &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
            let _ = (&mut self.task).await;
        }
    }
}

fn route_target(origin: &str) -> Option<String> {
    let url = url::Url::parse(origin).ok()?;
    Some(format!(
        "{}:{}",
        url.host_str()?,
        url.port_or_known_default()?
    ))
}

async fn run(
    client: Client,
    lock: Arc<Mutex<()>>,
    profile: String,
    target: String,
    proxy: bool,
    sink: Sink,
    mut shutdown: oneshot::Receiver<()>,
) {
    let Some(reason) = serve(&client, lock, &profile, target, proxy, &sink, &mut shutdown).await
    else {
        return;
    };
    // Hold until stopped, so the manager reads this reason, not a bare exit.
    if sink.send(Signal::Fatal(reason)).await {
        let _ = shutdown.await;
    }
}

/// Some(fatal reason), or None once shut down.
async fn serve(
    client: &Client,
    lock: Arc<Mutex<()>>,
    profile: &str,
    target: String,
    proxy: bool,
    sink: &Sink,
    shutdown: &mut oneshot::Receiver<()>,
) -> Option<String> {
    // Detached, so a stop cannot drop it between the relay's create and the
    // local save, which would orphan that tunnel.
    // shortcut: a create hung past ENSURE_TIMEOUT is still dropped, bound create alone if relays get that slow.
    let ensuring = tokio::spawn({
        let (client, profile) = (client.clone(), profile.to_owned());
        async move {
            let _one = lock.lock_owned().await;
            tokio::time::timeout(ENSURE_TIMEOUT, client.ensure(&profile)).await
        }
    });
    let ensured = tokio::select! {
        _ = &mut *shutdown => return None,
        ensured = ensuring => ensured,
    };
    let identity = match ensured {
        Ok(Ok(Ok(identity))) => identity,
        Ok(Ok(Err(error))) => return Some(failure(&error).into()),
        _ => {
            tracing::warn!("opentunnel provisioning timed out or panicked");
            return Some("opentunnel provisioning failed".into());
        }
    };
    if parser::public_url(&format!("https://{}", identity.hostname), DOMAIN).is_none() {
        tracing::warn!(hostname = %identity.hostname, "opentunnel returned an invalid hostname");
        return Some("opentunnel returned an invalid hostname".into());
    }
    let route = match route_name(&client.storage().root().join(profile)) {
        Ok(route) => route,
        Err(error) => {
            tracing::warn!(%error, "opentunnel route name unreadable");
            return Some("opentunnel state unreadable".into());
        }
    };
    sink.send(Signal::Url(format!(
        "https://{route}.{}",
        identity.hostname
    )))
    .await;
    let tunnel = match client.connect(profile, routes(&route, target, proxy)) {
        Ok(tunnel) => tunnel,
        Err(error) => return Some(failure(&error).into()),
    };
    let mut events = tunnel.subscribe();
    let reason = loop {
        let next = tokio::select! {
            _ = &mut *shutdown => break None,
            event = events.recv() => match event {
                Ok(event) => signal(&event, &route),
                Err(RecvError::Lagged(_)) => Some(if tunnel.status().state == State::Connected {
                    Signal::Connected
                } else {
                    Signal::Disconnected
                }),
                Err(RecvError::Closed) => Some(Signal::Fatal(STOPPED.into())),
            },
        };
        match next {
            Some(Signal::Fatal(reason)) => break Some(reason),
            Some(next) => {
                sink.send(next).await;
            }
            None => {}
        }
    };
    if let Err(error) = tunnel.close().await {
        tracing::warn!(%error, "opentunnel tunnel stopped with an error");
    }
    reason
}

fn routes(route: &str, target: String, proxy: bool) -> Routes {
    Routes::from([(
        route.to_owned(),
        Route::new(target).with_proxy_protocol(proxy.then_some(ProxyProtocol::V2)),
    )])
}

/// The target's route name: read from `<profile dir>/route`, or generated once and
/// stored there (0600). It is in no certificate, unlike the tunnel hostname.
fn route_name(dir: &Path) -> std::io::Result<String> {
    let path = dir.join("route");
    let name = match std::fs::read_to_string(&path) {
        Ok(name) => name.trim().to_owned(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // 16 hex chars, the tail of a v4 UUID: 62 random bits.
            let name = uuid::Uuid::new_v4().simple().to_string()[16..].to_owned();
            let mut file = tempfile::NamedTempFile::new_in(dir)?; // 0600
            std::io::Write::write_all(&mut file, name.as_bytes())?;
            file.as_file().sync_all()?;
            file.persist_noclobber(&path).map_err(|e| e.error)?;
            name
        }
        Err(e) => return Err(e),
    };
    // Never replaced when invalid: a new name would silently change the public URL.
    if !opentunnel::protocol::names::is_valid_profile(&name) {
        return Err(std::io::Error::other("invalid route name"));
    }
    Ok(name)
}

fn signal(event: &Event, route: &str) -> Option<Signal> {
    match event {
        Event::Connected { routes, .. } if routes.iter().any(|r| r == route) => {
            Some(Signal::Connected)
        }
        Event::Connected { .. } => Some(Signal::Fatal("opentunnel route not attached".into())),
        Event::Disconnected { .. } | Event::Reconnecting { .. } => Some(Signal::Disconnected),
        Event::Stopped { .. } => Some(Signal::Fatal(STOPPED.into())),
        Event::Connecting { .. }
        | Event::ConnectionOpened { .. }
        | Event::ConnectionClosed { .. }
        | Event::CertificateRenewed { .. } => None,
    }
}

/// Fixed text for the snapshot; the raw error only reaches the log.
fn failure(error: &Error) -> &'static str {
    tracing::warn!(%error, "opentunnel provisioning failed");
    match error {
        Error::Api { status: 429, .. } => "opentunnel rate limited",
        error if error.is_transient() => "opentunnel relay unreachable",
        Error::CertificateFailed(_) => "opentunnel certificate failed",
        Error::Io { .. } | Error::Corrupt { .. } => "opentunnel state unreadable",
        _ => "opentunnel provisioning failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROUTE: &str = "r0ute234abcd5678";

    #[test]
    fn every_event_maps_to_its_signal() {
        let signal = |event: &Event| signal(event, ROUTE);
        let connected = |routes: &[&str]| Event::Connected {
            session: "s".into(),
            routes: routes.iter().map(|r| r.to_string()).collect(),
        };
        assert_eq!(signal(&connected(&[ROUTE])), Some(Signal::Connected));
        assert_eq!(signal(&connected(&["api", ROUTE])), Some(Signal::Connected));
        for other in [&["@"][..], &["api"]] {
            assert_eq!(
                signal(&connected(other)),
                Some(Signal::Fatal("opentunnel route not attached".into())),
                "{other:?}"
            );
        }
        assert_eq!(
            signal(&Event::Disconnected {
                reason: "secret".into()
            }),
            Some(Signal::Disconnected)
        );
        assert_eq!(
            signal(&Event::Reconnecting {
                attempt: 1,
                delay_ms: 10
            }),
            Some(Signal::Disconnected)
        );
        assert_eq!(
            signal(&Event::Stopped {
                error: Some("secret".into())
            }),
            Some(Signal::Fatal(STOPPED.into()))
        );
        assert_eq!(
            signal(&Event::Stopped { error: None }),
            Some(Signal::Fatal(STOPPED.into()))
        );
        for ignored in [
            Event::Connecting { attempt: 0 },
            Event::ConnectionOpened {
                conn: 1,
                route: "@".into(),
                peer: "203.0.113.7:4000".into(),
            },
            Event::ConnectionClosed {
                conn: 1,
                route: "@".into(),
                error: None,
            },
            Event::CertificateRenewed {
                expiry: "2099-01-01T00:00:00Z".into(),
            },
        ] {
            assert_eq!(signal(&ignored), None, "{ignored:?}");
        }
    }

    #[test]
    fn every_error_class_maps_to_fixed_text() {
        let secret = "secret-detail";
        let api = |status| Error::Api {
            status,
            tag: secret.into(),
            message: secret.into(),
        };
        let corrupt = serde_json::from_str::<()>(secret).unwrap_err();
        for (error, text) in [
            (api(429), "opentunnel rate limited"),
            (api(503), "opentunnel relay unreachable"),
            (api(401), "opentunnel provisioning failed"),
            (
                Error::CertificateFailed(secret.into()),
                "opentunnel certificate failed",
            ),
            (
                Error::Io {
                    context: secret.into(),
                    source: std::io::Error::other(secret),
                },
                "opentunnel state unreadable",
            ),
            (
                Error::Corrupt {
                    path: secret.into(),
                    source: corrupt,
                },
                "opentunnel state unreadable",
            ),
            (
                Error::Attach(secret.into()),
                "opentunnel provisioning failed",
            ),
            (
                Error::Bridge(secret.into()),
                "opentunnel provisioning failed",
            ),
            (
                Error::Invalid(secret.into()),
                "opentunnel provisioning failed",
            ),
            (
                Error::Crypto(secret.into()),
                "opentunnel provisioning failed",
            ),
        ] {
            assert_eq!(failure(&error), text, "{error}");
        }
    }

    #[test]
    fn hostname_validator_accepts_only_one_label_under_the_relay_domain() {
        assert_eq!(
            parser::public_url("https://abc234.opentunnel.xyz", DOMAIN).as_deref(),
            Some("https://abc234.opentunnel.xyz")
        );
        for url in [
            "https://abc234.trycloudflare.com",
            "https://abc234.opentunnel.xyz.evil.test",
            "https://abc234.opentunnel.xyz:443",
            "https://abc234.opentunnel.xyz/path",
            "https://a.b.opentunnel.xyz",
            "https://opentunnel.xyz",
            "http://abc234.opentunnel.xyz",
        ] {
            assert_eq!(parser::public_url(url, DOMAIN), None, "{url}");
        }
    }

    #[test]
    fn route_target_comes_from_the_origin() {
        assert_eq!(
            route_target("http://127.0.0.1:3112").as_deref(),
            Some("127.0.0.1:3112")
        );
        assert_eq!(
            route_target("http://[::1]:49180").as_deref(),
            Some("[::1]:49180")
        );
        assert_eq!(
            route_target("http://127.0.0.1").as_deref(),
            Some("127.0.0.1:80")
        );
    }

    #[test]
    fn route_name_is_created_once_private_and_stable() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let name = route_name(dir.path()).unwrap();
        assert_eq!(name.len(), 16, "{name}");
        assert!(
            name.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
            "{name}"
        );
        let path = dir.path().join("route");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(route_name(dir.path()).unwrap(), name);
        let files: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(files, ["route"]);
    }

    #[test]
    fn an_invalid_route_file_is_an_error_not_a_new_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("route");
        for content in ["", "@", "Bad", "a.b"] {
            std::fs::write(&path, content).unwrap();
            assert!(route_name(dir.path()).is_err(), "{content:?}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), content);
        }
        std::fs::write(&path, "custom-name\n").unwrap();
        assert_eq!(route_name(dir.path()).unwrap(), "custom-name");
    }

    #[test]
    fn one_named_route_carries_the_header_only_when_listed() {
        for (proxy, expected) in [(true, Some(ProxyProtocol::V2)), (false, None)] {
            let routes = routes("r", "127.0.0.1:1".into(), proxy);
            assert_eq!(routes.keys().collect::<Vec<_>>(), ["r"]);
            assert_eq!(routes["r"].proxy_protocol, expected);
            assert!(!routes.contains_key("@"));
        }
    }

    #[test]
    fn the_config_that_marks_a_lease_v2_turns_the_header_on() {
        let dir = tempfile::tempdir().unwrap();
        let config = Config {
            provider: crate::config::ProviderKind::Opentunnel,
            targets: [
                ("webhooks", "http://127.0.0.1:3112"),
                ("mesh", "http://127.0.0.1:49180"),
            ]
            .into_iter()
            .map(|(id, origin)| (id.into(), origin.into()))
            .collect(),
            proxy_protocol: BTreeSet::from(["mesh".into()]),
            state_path: dir.path().join("leases.json"),
            ..Config::default()
        };
        let provider = OpenTunnel::from_config(&config).unwrap();
        assert!(provider.proxied("mesh"));
        assert!(!provider.proxied("webhooks"));
        for id in config.targets.keys() {
            assert_eq!(
                provider.proxied(id),
                config.proxy_protocol_of(id).is_some(),
                "{id}"
            );
        }
    }

    #[test]
    fn proxy_header_matches_the_vector_the_mesh_hub_parses() {
        // The three vectors of mesh/internal/hub/proxyproto_test.go.
        for (peer, len, vector) in [
            ("203.0.113.7:4000", 69, "0d0a0d0a000d0a515549540a21110035cb007107000000000fa001bb020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a"),
            ("[2001:db8::1]:443", 93, "0d0a0d0a000d0a515549540a2121004d20010db80000000000000000000000010000000000000000000000000000000001bb01bb020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a"),
            // An unparseable peer string gives LOCAL: no addresses.
            ("unknown", 57, "0d0a0d0a000d0a515549540a20000029020026723075746532333461626364353637382e6162633233342e6f70656e74756e6e656c2e78797a"),
        ] {
            let expected: Vec<u8> = (0..vector.len())
                .step_by(2)
                .map(|i| u8::from_str_radix(&vector[i..i + 2], 16).unwrap())
                .collect();
            assert_eq!(expected.len(), len, "{peer}");
            let header = opentunnel::protocol::proxy::header(
                ProxyProtocol::V2,
                peer,
                "r0ute234abcd5678.abc234.opentunnel.xyz",
            );
            assert_eq!(header, expected, "{peer}");
        }
    }
}
