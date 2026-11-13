#![cfg(unix)]
//! The real OpenTunnel provider with a closed API port and a temporary
//! storage: nothing leaves loopback. The crate retries API calls for up to
//! ~16 s before giving up, so these tests are slow by design.
use std::{
    os::unix::fs::PermissionsExt,
    path::Path,
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};

use chrono::Utc;
use opentunnel::{ClientOptions, Identity, Storage};
use quick_tunnel::{
    api::*,
    config::{Config, ProviderKind},
    manager::Manager,
    provider::{opentunnel::OpenTunnel, Provider, Signal, Sink},
};
use tokio::sync::{broadcast, mpsc};

const HOST: &str = "abc234.opentunnel.xyz";

/// HOME and the XDG directories point at one empty directory for this whole
/// test binary: the provider may write only under the state path. Proxy and
/// CA variables are cleared so nothing leaves loopback.
fn isolated_home() -> &'static Path {
    static HOME: OnceLock<tempfile::TempDir> = OnceLock::new();
    HOME.get_or_init(|| {
        let home = tempfile::tempdir().unwrap();
        for key in [
            "HOME",
            "XDG_DATA_HOME",
            "XDG_CONFIG_HOME",
            "XDG_STATE_HOME",
            "XDG_RUNTIME_DIR",
        ] {
            std::env::set_var(key, home.path());
        }
        for key in [
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
            "ALL_PROXY",
            "all_proxy",
            "OPENTUNNEL_CA_CERT",
        ] {
            std::env::remove_var(key);
        }
        home
    })
    .path()
}

fn assert_untouched(home: &Path) {
    let written: Vec<_> = std::fs::read_dir(home).unwrap().collect();
    assert!(written.is_empty(), "{written:?}");
}

fn closed_api() -> url::Url {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    drop(listener);
    url::Url::parse(&url).unwrap()
}

fn provider(storage: &Path) -> OpenTunnel {
    OpenTunnel::new(ClientOptions {
        api: Some(closed_api()),
        storage: Some(Storage::at(storage)),
        ca_cert: None,
    })
}

fn config(state: &Path) -> Config {
    Config {
        provider: ProviderKind::Opentunnel,
        state_path: state.join("leases.json"),
        max_retries: 0,
        ..Config::default()
    }
}

/// A provisioned identity with a self-signed certificate valid until 2099.
fn seed(storage: &Path) {
    let key = rcgen::generate_simple_self_signed(vec![HOST.to_owned()]).unwrap();
    Storage::at(storage)
        .save(
            "webhooks",
            &Identity {
                id: "abc234".into(),
                hostname: HOST.into(),
                token: "token".into(),
                private_key: key.signing_key.serialize_pem(),
                certificate: key.cert.pem(),
                chain: String::new(),
                certificate_expiry: "2099-01-01T00:00:00.000Z".into(),
            },
        )
        .unwrap();
}

fn request() -> AcquireRequest {
    AcquireRequest {
        consumer_id: "github".into(),
        tunnel_id: "webhooks".into(),
        expires_at: Utc::now() + chrono::Duration::seconds(120),
    }
}

/// The next snapshot with `status`; none before it may be `ready`.
async fn event(
    events: &mut broadcast::Receiver<Snapshot>,
    status: Status,
    within: Duration,
) -> Snapshot {
    tokio::time::timeout(within, async {
        loop {
            let next = events.recv().await.unwrap();
            assert_ne!(next.status, Status::Ready, "{next:?}");
            if next.status == status {
                break next;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn state_directory_is_private_and_a_group_readable_one_is_refused() {
    let home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let config = config(&dir.path().join("state"));
    let manager = Manager::open(config.clone()).unwrap();
    let state = dir.path().join("state/opentunnel");
    let mode = std::fs::metadata(&state).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o700);
    let status = manager
        .status(StatusRequest {
            tunnel_id: "webhooks".into(),
        })
        .await
        .unwrap();
    assert!(status.prerequisites.is_none());
    manager.shutdown().await;
    std::fs::set_permissions(&state, std::fs::Permissions::from_mode(0o750)).unwrap();
    let Err(error) = Manager::open(config) else {
        panic!("a group-readable state directory was accepted");
    };
    assert!(error.to_string().contains("chmod 700"), "{error}");
    assert_untouched(home);
}

#[tokio::test]
async fn empty_storage_and_a_closed_api_port_give_relay_unreachable() {
    let home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let mut config = config(dir.path());
    config.startup_timeout_ms = 60_000;
    let provider = Arc::new(provider(&dir.path().join("opentunnel")));
    let manager = Manager::open_with(config, provider).unwrap();
    let mut events = manager.subscribe();
    manager.acquire(request()).await.unwrap();
    let failed = event(&mut events, Status::Failed, Duration::from_secs(45)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("opentunnel relay unreachable")
    );
    manager.shutdown().await;
    assert_untouched(home);
}

#[tokio::test]
async fn seeded_identity_emits_the_url_never_connects_and_stops_within_the_bound() {
    let home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("opentunnel");
    seed(&storage);
    let (tx, mut rx) = mpsc::channel(64);
    let session = provider(&storage)
        .start(
            "webhooks",
            "http://127.0.0.1:3112",
            dir.path(),
            Sink::new("webhooks", "g1", tx),
        )
        .unwrap();
    let first = tokio::time::timeout(Duration::from_secs(45), rx.recv())
        .await
        .unwrap()
        .unwrap();
    // The served name is the target's random route under the hostname, never the bare hostname.
    let route_file = storage.join("webhooks/route");
    let route = std::fs::read_to_string(&route_file).unwrap();
    assert!(
        route.len() == 16
            && route
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')),
        "{route}"
    );
    let mode = std::fs::metadata(&route_file).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(first.signal, Signal::Url(format!("https://{route}.{HOST}")));
    assert_eq!(
        (first.tunnel_id.as_str(), first.generation.as_str()),
        ("webhooks", "g1")
    );
    // The bridge cannot attach to a closed port: only disconnections follow.
    let settle = tokio::time::sleep(Duration::from_secs(1));
    tokio::pin!(settle);
    loop {
        tokio::select! {
            _ = &mut settle => break,
            Some(next) = rx.recv() => assert_eq!(next.signal, Signal::Disconnected),
        }
    }
    let started = Instant::now();
    session.stop().await;
    assert!(
        started.elapsed() <= Duration::from_millis(2_500),
        "{:?}",
        started.elapsed()
    );
    assert_untouched(home);
}

#[tokio::test]
async fn seeded_identity_with_a_closed_api_port_ends_in_the_startup_timeout() {
    let home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let storage = dir.path().join("opentunnel");
    seed(&storage);
    // The default 30 s outlasts ensure's API retries, so the URL came first.
    let manager = Manager::open_with(config(dir.path()), Arc::new(provider(&storage))).unwrap();
    let mut events = manager.subscribe();
    manager.acquire(request()).await.unwrap();
    let failed = event(&mut events, Status::Failed, Duration::from_secs(45)).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("opentunnel startup/reconnect timeout")
    );
    manager.shutdown().await;
    assert_untouched(home);
}

/// A relay that accepts and never answers: create hangs after reaching it.
/// A stop then must not drop the create, nor let a retry create a second tunnel.
#[tokio::test]
async fn a_stop_during_first_create_never_creates_a_second_tunnel() {
    let home = isolated_home();
    let dir = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let api = url::Url::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
    let (accepted, mut connections) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Ok((socket, _)) = listener.accept().await {
            let _ = accepted.send(socket); // kept open, never answered
        }
    });
    let provider = OpenTunnel::new(ClientOptions {
        api: Some(api),
        storage: Some(Storage::at(dir.path().join("opentunnel"))),
        ca_cert: None,
    });
    let start = || {
        let (tx, _rx) = mpsc::channel(64);
        provider
            .start(
                "webhooks",
                "http://127.0.0.1:3112",
                dir.path(),
                Sink::new("webhooks", "g", tx),
            )
            .unwrap()
    };
    let first = start();
    let _held = tokio::time::timeout(Duration::from_secs(10), connections.recv())
        .await
        .unwrap()
        .unwrap();
    first.stop().await;
    let second = start();
    let again = tokio::time::timeout(Duration::from_secs(1), connections.recv()).await;
    assert!(again.is_err(), "a second create reached the relay");
    second.stop().await;
    assert_untouched(home);
}
