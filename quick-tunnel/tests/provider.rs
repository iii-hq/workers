//! Manager behaviour behind the provider trait, driven by a scripted fake.
mod support;

use std::{sync::Arc, time::Duration};

use chrono::Utc;
use quick_tunnel::{
    api::*,
    config::{Config, ProviderKind},
    manager::Manager,
    provider::Signal,
};
use support::FakeProvider;
use tokio::sync::broadcast;

const URL: &str = "https://abc234.opentunnel.xyz";

fn open(max_retries: u32) -> (tempfile::TempDir, Manager, Arc<FakeProvider>) {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        state_path: dir.path().join("leases.json"),
        startup_timeout_ms: 1_000,
        max_retries,
        retry_initial_ms: 20,
        retry_max_ms: 40,
        ..Config::default()
    };
    let fake = Arc::new(FakeProvider::default());
    let manager = Manager::open_with(config, fake.clone()).unwrap();
    (dir, manager, fake)
}

fn request(consumer: &str) -> AcquireRequest {
    AcquireRequest {
        consumer_id: consumer.into(),
        tunnel_id: "webhooks".into(),
        expires_at: Utc::now() + chrono::Duration::seconds(30),
    }
}

async fn status(manager: &Manager) -> StatusResponse {
    manager
        .status(StatusRequest {
            tunnel_id: "webhooks".into(),
        })
        .await
        .unwrap()
}

/// The next snapshot with `status`; none before it may be `ready` unless asked.
async fn event(events: &mut broadcast::Receiver<Snapshot>, status: Status) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = events.recv().await.unwrap();
            if next.status == status {
                break next;
            }
            assert_ne!(next.status, Status::Ready, "{next:?}");
        }
    })
    .await
    .unwrap()
}

async fn ready(
    manager: &Manager,
    fake: &FakeProvider,
    events: &mut broadcast::Receiver<Snapshot>,
) -> Snapshot {
    manager.acquire(request("github")).await.unwrap();
    fake.started(1).await;
    fake.send(Signal::Url(URL.into())).await;
    fake.send(Signal::Connected).await;
    event(events, Status::Ready).await
}

#[tokio::test]
async fn url_then_connected_is_ready() {
    let (_dir, manager, fake) = open(0);
    let mut events = manager.subscribe();
    let ready = ready(&manager, &fake, &mut events).await;
    assert_eq!(ready.public_url.as_deref(), Some(URL));
    assert_eq!(ready.error, None);
    manager.shutdown().await;
}

#[tokio::test]
async fn connected_without_url_is_not_ready_and_times_out_with_the_provider_name() {
    let (_dir, manager, fake) = open(0);
    let mut events = manager.subscribe();
    manager.acquire(request("github")).await.unwrap();
    fake.started(1).await;
    fake.send(Signal::Connected).await;
    let failed = event(&mut events, Status::Failed).await;
    assert_eq!(
        failed.error.as_deref(),
        Some("fake startup/reconnect timeout")
    );
    assert_eq!(failed.public_url, None);
    manager.shutdown().await;
}

#[tokio::test]
async fn disconnected_clears_the_url_and_keeps_the_generation() {
    let (_dir, manager, fake) = open(0);
    let mut events = manager.subscribe();
    let ready = ready(&manager, &fake, &mut events).await;
    fake.send(Signal::Disconnected).await;
    let reconnecting = event(&mut events, Status::Reconnecting).await;
    assert_eq!(reconnecting.public_url, None);
    assert_eq!(reconnecting.generation, ready.generation);
    assert_eq!(
        reconnecting.error.as_deref(),
        Some("fake edge connection interrupted")
    );
    fake.send(Signal::Connected).await;
    let again = event(&mut events, Status::Ready).await;
    assert_eq!(again.generation, ready.generation);
    assert_eq!(again.public_url.as_deref(), Some(URL));
    assert_eq!(fake.starts(), 1);
    manager.shutdown().await;
}

#[tokio::test]
async fn fatal_retries_in_a_new_generation_that_may_reuse_the_url_then_fails() {
    let (_dir, manager, fake) = open(1);
    let mut events = manager.subscribe();
    let first = ready(&manager, &fake, &mut events).await;
    fake.send(Signal::Fatal("fake broke".into())).await;
    let retrying = event(&mut events, Status::Reconnecting).await;
    assert_eq!(retrying.error.as_deref(), Some("fake broke"));
    fake.started(2).await;
    // A stable hostname (OpenTunnel) repeats the URL; the generation still changes.
    fake.send(Signal::Url(URL.into())).await;
    fake.send(Signal::Connected).await;
    let second = event(&mut events, Status::Ready).await;
    assert_ne!(second.generation, first.generation);
    assert_eq!(second.public_url.as_deref(), Some(URL));
    fake.send(Signal::Fatal("fake broke".into())).await;
    let failed = event(&mut events, Status::Failed).await;
    assert_eq!(failed.error.as_deref(), Some("fake broke"));
    assert_eq!(fake.stops(), 2);
    manager.shutdown().await;
}

#[tokio::test]
async fn last_release_stops_the_session_exactly_once() {
    let (_dir, manager, fake) = open(0);
    let mut events = manager.subscribe();
    let first = manager.acquire(request("github")).await.unwrap();
    ready(&manager, &fake, &mut events).await;
    let other = manager.acquire(request("preview")).await.unwrap();
    manager
        .release(ReleaseRequest {
            lease_id: first.lease_id,
        })
        .await
        .unwrap();
    assert_eq!(status(&manager).await.snapshot.status, Status::Ready);
    assert_eq!(fake.stops(), 0);
    manager
        .release(ReleaseRequest {
            lease_id: other.lease_id,
        })
        .await
        .unwrap();
    assert_eq!(status(&manager).await.snapshot.status, Status::Stopped);
    manager.shutdown().await;
    assert_eq!(fake.stops(), 1);
    assert_eq!(fake.starts(), 1);
}

#[tokio::test]
async fn start_error_is_observable_in_the_snapshot() {
    let (_dir, manager, fake) = open(0);
    *fake.start_error.lock().unwrap() = Some("fake refused".into());
    let reply = manager.acquire(request("github")).await.unwrap();
    assert_eq!(reply.snapshot.status, Status::Failed);
    assert_eq!(reply.snapshot.error.as_deref(), Some("fake refused"));
    manager.shutdown().await;
}

#[tokio::test]
async fn a_lease_records_proxy_protocol_and_a_mode_change_drops_it() {
    let dir = tempfile::tempdir().unwrap();
    let open = |proxied: &[&str]| {
        let config = Config {
            provider: ProviderKind::Opentunnel,
            proxy_protocol: proxied.iter().map(|id| id.to_string()).collect(),
            state_path: dir.path().join("leases.json"),
            startup_timeout_ms: 1_000,
            max_retries: 0,
            retry_initial_ms: 20,
            retry_max_ms: 40,
            ..Config::default()
        };
        let fake = Arc::new(FakeProvider::default());
        (Manager::open_with(config, fake.clone()).unwrap(), fake)
    };
    let json = |status: &StatusResponse| serde_json::to_value(status).unwrap();

    let (manager, _fake) = open(&["webhooks"]);
    let acquired = serde_json::to_value(manager.acquire(request("mesh")).await.unwrap()).unwrap();
    assert!(acquired.get("provider").is_none(), "{acquired}");
    assert!(acquired.get("proxy_protocol").is_none(), "{acquired}");
    let listed = json(&status(&manager).await);
    assert_eq!(listed["provider"], "opentunnel", "{listed}");
    assert_eq!(listed["leases"][0]["proxy_protocol"], "v2", "{listed}");
    manager.shutdown().await;
    drop(manager);

    // Same mode: the "v2" lease survives the restart.
    let (manager, _fake) = open(&["webhooks"]);
    let kept = json(&status(&manager).await);
    assert_eq!(kept["leases"], listed["leases"], "{kept}");
    manager.shutdown().await;
    drop(manager);

    let (manager, fake) = open(&[]);
    assert!(status(&manager).await.leases.is_empty());
    assert_eq!(fake.starts(), 0);
    manager.acquire(request("mesh")).await.unwrap();
    let plain = json(&status(&manager).await);
    assert_eq!(plain["leases"][0]["consumer_id"], "mesh", "{plain}");
    assert!(
        plain["leases"][0].get("proxy_protocol").is_none(),
        "{plain}"
    );
    manager.shutdown().await;
    drop(manager);

    let (manager, _fake) = open(&["webhooks"]);
    assert!(status(&manager).await.leases.is_empty());
    manager.shutdown().await;
}

#[tokio::test]
async fn cloudflared_json_has_no_new_keys() {
    let (dir, manager, _fake) = open(0);
    manager.acquire(request("github")).await.unwrap();
    let json = serde_json::to_value(status(&manager).await).unwrap();
    assert!(json.get("provider").is_none(), "{json}");
    assert_eq!(json["leases"][0]["consumer_id"], "github", "{json}");
    assert!(json["leases"][0].get("proxy_protocol").is_none(), "{json}");
    let stored = std::fs::read_to_string(dir.path().join("leases.json")).unwrap();
    assert!(!stored.contains("proxy_protocol"), "{stored}");
    manager.shutdown().await;
}

#[tokio::test]
async fn prerequisites_are_omitted_when_the_provider_has_none() {
    let (_dir, manager, _fake) = open(0);
    let status = status(&manager).await;
    assert!(status.prerequisites.is_none());
    let json = serde_json::to_value(&status).unwrap();
    assert!(json.get("prerequisites").is_none(), "{json}");
    manager.shutdown().await;
}
