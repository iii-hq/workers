#![cfg(unix)]

use chrono::Utc;
use quick_tunnel::{api::*, config::Config, manager::Manager};
use std::{os::unix::fs::PermissionsExt, path::Path, time::Duration};
use tokio::sync::broadcast;

fn fixture(mode: &str) -> (tempfile::TempDir, Config) {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join(".test-data");
    std::fs::create_dir_all(&root).unwrap();
    let dir = tempfile::tempdir_in(root).unwrap();
    let executable = dir.path().join("fake-cloudflared");
    std::fs::write(&executable, include_str!("fixtures/fake-cloudflared.py")).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::write(dir.path().join("mode"), mode).unwrap();
    let config = Config {
        cloudflared: executable.to_str().unwrap().into(),
        state_path: dir.path().join("leases.json"),
        startup_timeout_ms: 500,
        max_retries: 0,
        retry_initial_ms: 20,
        retry_max_ms: 40,
        ..Config::default()
    };
    (dir, config)
}

fn request(consumer: &str, ms: i64) -> AcquireRequest {
    AcquireRequest {
        consumer_id: consumer.into(),
        tunnel_id: "webhooks".into(),
        expires_at: Utc::now() + chrono::Duration::milliseconds(ms),
    }
}
async fn snapshot(manager: &Manager) -> StatusResponse {
    manager
        .status(StatusRequest {
            tunnel_id: "webhooks".into(),
        })
        .await
        .unwrap()
}
async fn event(events: &mut broadcast::Receiver<Snapshot>, status: Status) -> Snapshot {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = events.recv().await.unwrap();
            if next.status == status {
                break next;
            }
        }
    })
    .await
    .unwrap()
}
fn pids(dir: &Path) -> Vec<String> {
    std::fs::read_to_string(dir.join("pids"))
        .unwrap_or_default()
        .lines()
        .map(str::to_owned)
        .collect()
}
fn reaped(dir: &Path) {
    #[cfg(target_os = "linux")]
    for pid in pids(dir) {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists(),
            "child {pid} not reaped"
        );
    }
}

#[tokio::test]
async fn concurrent_acquire_is_idempotent_and_independent_leases_keep_child_alive() {
    let (dir, config) = fixture("ready");
    let manager = Manager::open(config).unwrap();
    let mut events = manager.subscribe();
    let calls = (0..20).map(|_| manager.acquire(request("github", 5000)));
    let responses = futures_util::future::join_all(calls).await;
    let first = responses[0].as_ref().unwrap();
    assert!(responses
        .iter()
        .all(|r| r.as_ref().unwrap().lease_id == first.lease_id));
    let other = manager.acquire(request("preview", 5000)).await.unwrap();
    event(&mut events, Status::Ready).await;
    assert_eq!(pids(dir.path()).len(), 1);
    manager
        .release(ReleaseRequest {
            lease_id: first.lease_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(snapshot(&manager).await.snapshot.status, Status::Ready);
    manager
        .release(ReleaseRequest {
            lease_id: other.lease_id.clone(),
        })
        .await
        .unwrap();
    assert_eq!(snapshot(&manager).await.snapshot.status, Status::Stopped);
    assert!(
        !manager
            .release(ReleaseRequest {
                lease_id: other.lease_id
            })
            .await
            .unwrap()
            .released
    );
    reaped(dir.path());
    manager.shutdown().await;
}

#[tokio::test]
async fn lease_expiry_stops_child_without_an_agent_or_request() {
    let (dir, config) = fixture("ready");
    let manager = Manager::open(config.clone()).unwrap();
    let mut events = manager.subscribe();
    manager.acquire(request("github", 300)).await.unwrap();
    event(&mut events, Status::Ready).await;
    event(&mut events, Status::Stopped).await;
    assert!(snapshot(&manager).await.leases.is_empty());
    assert_eq!(std::fs::read_to_string(config.state_path).unwrap(), "[]");
    reaped(dir.path());
    manager.shutdown().await;
}

#[tokio::test]
async fn printed_url_without_connection_times_out_and_bounded_retries_fail() {
    let (dir, mut config) = fixture("url-only");
    config.startup_timeout_ms = 100;
    config.max_retries = 2;
    let manager = Manager::open(config).unwrap();
    let mut events = manager.subscribe();
    let first = manager.acquire(request("github", 5000)).await.unwrap();
    assert!(first.snapshot.public_url.is_none());
    let mut generations = std::collections::BTreeSet::new();
    let failed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let next = events.recv().await.unwrap();
            assert_ne!(next.status, Status::Ready);
            generations.insert(next.generation.clone());
            if next.status == Status::Failed {
                break next;
            }
        }
    })
    .await
    .unwrap();
    assert!(failed.error.unwrap().contains("timeout"));
    assert_eq!(generations.len(), 3);
    assert_eq!(pids(dir.path()).len(), 3);
    reaped(dir.path());
    manager.shutdown().await;
}

#[tokio::test]
async fn restart_restores_leases_but_not_generation_or_public_url() {
    let (dir, config) = fixture("ready");
    let manager = Manager::open(config.clone()).unwrap();
    let mut events = manager.subscribe();
    let lease = manager.acquire(request("github", 5000)).await.unwrap();
    event(&mut events, Status::Ready).await;
    let old = snapshot(&manager).await;
    manager.shutdown().await;
    reaped(dir.path());
    let restarted = Manager::open(config).unwrap();
    let mut events = restarted.subscribe();
    let new = event(&mut events, Status::Ready).await;
    assert_ne!(new.generation, old.snapshot.generation);
    assert_ne!(new.public_url, old.snapshot.public_url);
    assert_eq!(
        snapshot(&restarted).await.leases[0].lease_id,
        lease.lease_id
    );
    restarted.shutdown().await;
    reaped(dir.path());
}

#[tokio::test]
async fn missing_executable_is_observable_but_does_not_prevent_registration() {
    let (_dir, mut config) = fixture("ready");
    config.cloudflared = "/nonexistent/quick-tunnel-cloudflared".into();
    let manager = Manager::open(config).unwrap();
    assert_eq!(snapshot(&manager).await.snapshot.status, Status::Stopped);
    let reply = manager.acquire(request("github", 5000)).await.unwrap();
    assert_eq!(reply.snapshot.status, Status::Failed);
    assert!(reply.snapshot.error.unwrap().contains("prerequisite"));
    manager.shutdown().await;
}

#[tokio::test]
async fn disconnection_clears_url_and_reconnect_requires_connection_again() {
    let (dir, config) = fixture("reconnect");
    let manager = Manager::open(config).unwrap();
    let mut events = manager.subscribe();
    manager.acquire(request("github", 5000)).await.unwrap();
    let ready = event(&mut events, Status::Ready).await;
    let reconnecting = event(&mut events, Status::Reconnecting).await;
    assert!(reconnecting.public_url.is_none());
    assert!(!reconnecting.error.unwrap().contains("fixture secret"));
    let next = event(&mut events, Status::Ready).await;
    assert_eq!(next.generation, ready.generation);
    manager.shutdown().await;
    reaped(dir.path());
}

#[tokio::test]
async fn crash_and_oversized_output_both_fail_and_reap() {
    for mode in ["crash", "oversized", "silent", "disconnect"] {
        let (dir, config) = fixture(mode);
        let manager = Manager::open(config).unwrap();
        let mut events = manager.subscribe();
        manager.acquire(request("github", 5000)).await.unwrap();
        let failed = event(&mut events, Status::Failed).await;
        assert!(failed.public_url.is_none(), "{mode}");
        reaped(dir.path());
        manager.shutdown().await;
    }
}

#[tokio::test]
async fn validation_renewal_and_exclusive_persistence_are_enforced() {
    let (_dir, config) = fixture("ready");
    let manager = Manager::open(config.clone()).unwrap();
    assert!(Manager::open(config).is_err());
    assert!(manager.acquire(request("github", -1)).await.is_err());
    let mut invalid = request("github", 5000);
    invalid.tunnel_id = "arbitrary-port".into();
    assert!(manager.acquire(invalid).await.is_err());
    let first = manager.acquire(request("github", 5000)).await.unwrap();
    let expiry = snapshot(&manager).await.leases[0].expires_at;
    let renewal = manager.acquire(request("github", 1000)).await.unwrap();
    assert_eq!(renewal.lease_id, first.lease_id);
    assert_eq!(snapshot(&manager).await.leases[0].expires_at, expiry);
    manager.shutdown().await;
}

#[tokio::test]
async fn corrupt_persistence_fails_closed_and_expired_leases_do_not_restart() {
    let (dir, config) = fixture("ready");
    std::fs::write(&config.state_path, "not json").unwrap();
    assert!(Manager::open(config.clone()).is_err());
    let expired = Lease {
        lease_id: uuid::Uuid::new_v4().to_string(),
        consumer_id: "github".into(),
        tunnel_id: "webhooks".into(),
        expires_at: Utc::now() - chrono::Duration::seconds(1),
    };
    std::fs::write(
        &config.state_path,
        serde_json::to_vec(&vec![expired]).unwrap(),
    )
    .unwrap();
    let manager = Manager::open(config).unwrap();
    assert!(snapshot(&manager).await.leases.is_empty());
    assert!(pids(dir.path()).is_empty());
    manager.shutdown().await;
}

#[tokio::test]
async fn worker_sigterm_and_ctrl_c_reap_even_an_uncooperative_child() {
    for signal in ["-TERM", "-INT"] {
        let (dir, config) = fixture("ignore-term");
        let lease = Lease {
            lease_id: uuid::Uuid::new_v4().to_string(),
            consumer_id: "github".into(),
            tunnel_id: "webhooks".into(),
            expires_at: Utc::now() + chrono::Duration::seconds(10),
        };
        std::fs::write(
            &config.state_path,
            serde_json::to_vec(&vec![lease]).unwrap(),
        )
        .unwrap();
        let yaml = dir.path().join("worker.yaml");
        std::fs::write(&yaml, serde_yaml::to_string(&config).unwrap()).unwrap();
        let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_quick-tunnel"))
            .args(["--local-config", "--url", "ws://127.0.0.1:1", "--config"])
            .arg(yaml)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            while pids(dir.path()).is_empty() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let status = tokio::process::Command::new("/bin/kill")
            .args([signal, &child.id().unwrap().to_string()])
            .status()
            .await
            .unwrap();
        assert!(status.success());
        assert!(tokio::time::timeout(Duration::from_secs(5), child.wait())
            .await
            .unwrap()
            .unwrap()
            .success());
        reaped(dir.path());
    }
}
