//! End-to-end test (docs/sops/binary-worker.md §9 pattern A): spawn the `iii` engine
//! and the `storage` worker as subprocesses, drive the worker through
//! `iii-sdk` as a client, then tear both down. **Self-skips** when neither
//! `III_ENGINE_BIN` nor an `iii` binary on `PATH` is available.
//!
//! What we exercise: a `storage::putObject` round-trip against the native
//! local backend (no cloud credentials or sidecar needed), then a `getObject`
//! to confirm the bytes round-trip.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};
use iii_sdk::protocol::TriggerRequest;
use iii_sdk::{register_worker, InitOptions};
use serde_json::json;
use tokio::time::{sleep, timeout};

struct Harness {
    iii: Child,
    worker: Child,
    url: String,
    _directory: tempfile::TempDir,
}

impl Drop for Harness {
    fn drop(&mut self) {
        let _ = self.worker.kill();
        let _ = self.worker.wait();
        let _ = self.iii.kill();
        let _ = self.iii.wait();
    }
}

async fn boot() -> Option<Harness> {
    let iii_bin = std::env::var_os("III_ENGINE_BIN")
        .map(PathBuf::from)
        .or_else(|| which::which("iii").ok())?;
    let port = TcpListener::bind("127.0.0.1:0")
        .ok()?
        .local_addr()
        .ok()?
        .port();
    let directory = tempfile::tempdir().ok()?;
    let engine_config_path = directory.path().join("engine.yaml");
    std::fs::write(
        &engine_config_path,
        format!("workers:\n  - name: iii-worker-manager\n    config:\n      port: {port}\n"),
    )
    .ok()?;
    let url = format!("ws://127.0.0.1:{port}");

    let mut iii = Command::new(&iii_bin)
        .arg("--no-update-check")
        .arg("--config")
        .arg(&engine_config_path)
        .current_dir(directory.path())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    sleep(Duration::from_millis(800)).await;

    let worker_bin = env!("CARGO_BIN_EXE_storage");
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let config_path = format!("{manifest_dir}/config.yaml");

    // If the worker fails to spawn, kill the engine before returning so we
    // don't leak the iii subprocess into the next test (or out of the
    // process entirely). Without this, a transient spawn failure could
    // leave a stray engine bound to the test port.
    let worker = match Command::new(worker_bin)
        .args(["--url", &url, "--config", &config_path])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(w) => w,
        Err(_) => {
            let _ = iii.kill();
            let _ = iii.wait();
            return None;
        }
    };

    Some(Harness {
        iii,
        worker,
        url,
        _directory: directory,
    })
}

/// Poll `storage::putObject` until the worker has registered, or give up.
/// Returns the last response value (or panics with the last error). The worker
/// initializes its local data directory before registering functions, so
/// there can still be a short readiness window.
async fn put_when_ready(
    client: &iii_sdk::IIIClient,
    payload: serde_json::Value,
) -> serde_json::Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    let mut last_err: Option<String> = None;
    while std::time::Instant::now() < deadline {
        match timeout(
            Duration::from_secs(5),
            client.trigger(TriggerRequest {
                function_id: "storage::putObject".into(),
                payload: payload.clone(),
                action: None,
                timeout_ms: Some(4_000),
            }),
        )
        .await
        {
            Ok(Ok(v)) => return v,
            Ok(Err(e)) => {
                let msg = format!("{e:?}");
                if !msg.contains("function_not_found") && !msg.contains("not found") {
                    panic!("putObject failed: {msg}");
                }
                last_err = Some(msg);
            }
            Err(_) => last_err = Some("trigger timed out".into()),
        }
        sleep(Duration::from_millis(500)).await;
    }
    panic!(
        "worker never registered storage::putObject within 30s; last error: {}",
        last_err.unwrap_or_else(|| "<none>".into())
    );
}

#[tokio::test]
async fn put_then_get_round_trips_via_native_local_storage() {
    let Some(harness) = boot().await else {
        eprintln!("skipping: iii engine is unavailable");
        return;
    };

    let client = register_worker(&harness.url, InitOptions::default());
    sleep(Duration::from_millis(500)).await;

    let body = b"hello from integration.rs";
    let body_b64 = B64.encode(body);

    let put = put_when_ready(
        &client,
        json!({
            "bucket": "scratch",
            "key": "integration/hello.txt",
            "body": body_b64,
            "content_type": "text/plain"
        }),
    )
    .await;

    assert!(put.is_object(), "putObject should return an object");

    let get: serde_json::Value = timeout(
        Duration::from_secs(10),
        client.trigger(TriggerRequest {
            function_id: "storage::getObject".into(),
            payload: json!({
                "bucket": "scratch",
                "key": "integration/hello.txt"
            }),
            action: None,
            timeout_ms: Some(8_000),
        }),
    )
    .await
    .expect("getObject timed out")
    .expect("getObject failed");

    let got_b64 = get["body_base64"]
        .as_str()
        .expect("getObject response should have base64 `body_base64`");
    let got = B64.decode(got_b64).expect("response body is valid base64");
    assert_eq!(got, body, "round-tripped body must match");

    client.shutdown_async().await;
}

#[tokio::test]
#[ignore = "explicit isolated S3 fixture required"]
async fn multipart_controls_contract() {
    use sha2::{Digest, Sha256};
    use std::{collections::HashMap, sync::Arc};
    use storage::backend::{
        s3::{S3Backend, S3BackendOpts, S3StaticCreds},
        Backend,
    };
    use storage::handlers::{head_object, multipart, AppState};
    let endpoint = std::env::var("S3_TEST_ENDPOINT").unwrap();
    let bucket = std::env::var("S3_TEST_BUCKET").unwrap();
    let region = std::env::var("S3_TEST_REGION").unwrap();
    let backend = S3Backend::build(
        S3BackendOpts {
            region: &region,
            credentials: Some(S3StaticCreds {
                access_key_id: std::env::var("AWS_ACCESS_KEY_ID").unwrap(),
                secret_access_key: std::env::var("AWS_SECRET_ACCESS_KEY").unwrap(),
                session_token: std::env::var("AWS_SESSION_TOKEN").ok(),
            }),
            endpoint_url: Some(endpoint),
            force_path_style: true,
            provider_tag: "s3",
            fast_fail: true,
        },
        bucket,
    )
    .await
    .unwrap();
    let mut backends: HashMap<String, Arc<dyn Backend>> = HashMap::new();
    backends.insert("test".into(), Arc::new(backend));
    let state = AppState::new(backends);
    let key = format!("qa/control/{}", hex::encode(rand::random::<[u8; 16]>()));
    let create = multipart::create(
        &state,
        multipart::CreateReq {
            bucket: "test".into(),
            key: key.clone(),
            content_type: "video/mp4".into(),
            metadata: HashMap::from([("token".into(), "example".into())]),
        },
    )
    .await
    .unwrap();
    let id = create.upload_id;
    let data = b"multipart-control";
    let checksum = B64.encode(Sha256::digest(data));
    let signed = multipart::presign(
        &state,
        multipart::PresignReq {
            bucket: "test".into(),
            key: key.clone(),
            upload_id: id.clone(),
            part_number: 1,
            size: data.len() as u64,
            checksum_sha256: checksum.clone(),
            expires_in_seconds: 60,
        },
    )
    .await
    .unwrap();
    let mut request = reqwest::Client::new().put(&signed.url).body(data.to_vec());
    for (name, value) in &signed.headers {
        request = request.header(name, value);
    }
    let response = request
        .send()
        .await
        .unwrap_or_else(|_| panic!("signed upload request failed"));
    assert!(
        response.status().is_success(),
        "signed upload failed with {}",
        response.status()
    );
    let identity = || multipart::Identity {
        bucket: "test".into(),
        key: key.clone(),
        upload_id: id.clone(),
    };
    let listed = multipart::list(&state, identity()).await.unwrap();
    assert_eq!(listed.parts.len(), 1);
    assert_eq!(listed.parts[0].checksum_sha256, checksum);
    assert_eq!(listed.parts[0].size, data.len() as u64);
    let mut wrong = listed.parts.clone();
    wrong[0].etag = "\"wrong\"".into();
    let error = multipart::complete(
        &state,
        multipart::CompleteReq {
            bucket: "test".into(),
            key: key.clone(),
            upload_id: id.clone(),
            parts: wrong,
        },
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("InvalidPart"),
        "unexpected completion error: {error}"
    );
    let completed = multipart::complete(
        &state,
        multipart::CompleteReq {
            bucket: "test".into(),
            key: key.clone(),
            upload_id: id.clone(),
            parts: listed.parts,
        },
    )
    .await;
    let head = head_object::handle(
        &state,
        head_object::HeadReq {
            bucket: "test".into(),
            key: key.clone(),
            version_id: None,
            checksum_mode: true,
        },
    )
    .await
    .unwrap();
    eprintln!(
        "completion ok: {}; HEAD checksum present: {}; HEAD checksum type: {:?}",
        completed.is_ok(),
        head.checksum_sha256.is_some(),
        head.checksum_type
    );
    state
        .backend("test")
        .await
        .unwrap()
        .delete(storage::backend::DeleteReq {
            key: key.clone(),
            version_id: None,
        })
        .await
        .unwrap();
    let aborted_key = format!("{}/aborted", key);
    let aborted = multipart::create(
        &state,
        multipart::CreateReq {
            bucket: "test".into(),
            key: aborted_key.clone(),
            content_type: "video/mp4".into(),
            metadata: HashMap::new(),
        },
    )
    .await
    .unwrap();
    let abort_identity = || multipart::Identity {
        bucket: "test".into(),
        key: aborted_key.clone(),
        upload_id: aborted.upload_id.clone(),
    };
    assert!(
        multipart::abort(&state, abort_identity())
            .await
            .unwrap()
            .aborted
    );
    assert!(
        multipart::abort(&state, abort_identity())
            .await
            .unwrap()
            .aborted
    );
    let error = multipart::list(&state, abort_identity()).await.unwrap_err();
    assert!(
        error.contains("NoSuchUpload"),
        "unexpected list error: {error}"
    );
    let error = multipart::complete(
        &state,
        multipart::CompleteReq {
            bucket: "test".into(),
            key: aborted_key,
            upload_id: aborted.upload_id,
            parts: vec![multipart::Part {
                part_number: 1,
                size: data.len() as u64,
                etag: "\"opaque\"".into(),
                checksum_sha256: checksum.clone(),
            }],
        },
    )
    .await
    .unwrap_err();
    assert!(
        error.contains("NoSuchUpload"),
        "unexpected completion error: {error}"
    );
    let completed = completed.unwrap();
    assert_eq!(completed.checksum_type, "COMPOSITE");
    assert_eq!(head.metadata.unwrap().get("token").unwrap(), "example");
    assert_eq!(
        head.checksum_sha256.as_deref(),
        Some(completed.checksum_sha256.as_str())
    );
    assert_eq!(head.checksum_type.as_deref(), Some("COMPOSITE"));
}
