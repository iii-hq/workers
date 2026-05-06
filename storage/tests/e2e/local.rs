//! Local rustfs-backed round-trip. Skips cleanly when no rustfs binary is
//! discoverable (in which case the test reports "skip" but does not fail).
//! Otherwise spawns rustfs on a temp data dir, runs put/get/delete.

use std::collections::HashMap;
use std::time::Duration;
use storage::backend::factory::{self, LocalBackendCtx};
use storage::backend::{GetReq, PutReq};
use storage::config::{BucketConfig, LocalBucketConfig, LocalProviderConfig, ProvidersConfig};
use storage::rustfs::{health, spawn};

#[tokio::test]
async fn local_round_trip_when_rustfs_available() {
    if spawn::discover_binary().is_err() {
        eprintln!("e2e:local — skip: no rustfs binary on PATH (set RUSTFS_BIN to enable)");
        return;
    }
    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().to_string_lossy().to_string();
    let port = spawn::allocate_port().expect("port");
    let (access_key, secret_key) = spawn::ephemeral_credentials();
    let handle = spawn::spawn(spawn::RustfsSpawnOpts {
        data_dir: data_dir.clone(),
        access_key: access_key.clone(),
        secret_key: secret_key.clone(),
        port,
        notify_webhook_url: None,
    })
    .await
    .expect("rustfs spawn");
    health::wait_for_healthy(port, Duration::from_secs(30))
        .await
        .expect("rustfs healthy");
    storage::backend::local::ensure_bucket(port, &access_key, &secret_key, "scratch")
        .await
        .expect("bucket bootstrap");

    let local_ctx = LocalBackendCtx {
        port,
        access_key_id: access_key,
        secret_access_key: secret_key,
    };
    let providers = ProvidersConfig {
        local: Some(LocalProviderConfig { data_dir }),
    };
    let cfg = BucketConfig::Local(LocalBucketConfig {
        bucket: Some("scratch".into()),
    });
    let backend = factory::build("scratch", &cfg, &providers, Some(&local_ctx))
        .await
        .expect("backend build");

    let key = "e2e/local/test.bin";
    let body = b"hello-local".to_vec();
    backend
        .put(PutReq {
            key: key.into(),
            body: body.clone(),
            content_type: "application/octet-stream".into(),
            cache_control: None,
            metadata: HashMap::new(),
        })
        .await
        .expect("put");
    let got = backend
        .get(GetReq {
            key: key.into(),
            ..Default::default()
        })
        .await
        .expect("get");
    assert_eq!(got.body, body);
    backend
        .delete(storage::backend::DeleteReq {
            key: key.into(),
            version_id: None,
        })
        .await
        .expect("delete");

    spawn::shutdown(handle).await;
}
