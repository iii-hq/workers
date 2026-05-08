//! Real GCS round-trip. Gated on GOOGLE_APPLICATION_CREDENTIALS + GCS_TEST_BUCKET.

use std::collections::HashMap;
use storage::backend::{factory, GetReq, PutReq};
use storage::config::{BucketConfig, GcsBucketConfig, ProvidersConfig};

#[tokio::test]
#[ignore = "opt-in: set GOOGLE_APPLICATION_CREDENTIALS and GCS_TEST_BUCKET"]
async fn gcs_round_trip() {
    let Ok(bucket) = std::env::var("GCS_TEST_BUCKET") else {
        eprintln!("e2e:gcs — skip: GCS_TEST_BUCKET not set");
        return;
    };
    if std::env::var("GOOGLE_APPLICATION_CREDENTIALS").is_err() {
        eprintln!("e2e:gcs — skip: GOOGLE_APPLICATION_CREDENTIALS not set");
        return;
    }
    let cfg = BucketConfig::Gcs(GcsBucketConfig {
        bucket: Some(bucket.clone()),
        credentials_file: None,
        endpoint_url: None,
        notifications: None,
    });
    let backend = factory::build("e2e", &cfg, &ProvidersConfig::default(), None)
        .await
        .expect("backend build");
    let key = format!(
        "e2e/storage/{}.bin",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)
    );
    let body = b"hello-gcs".to_vec();
    backend
        .put(PutReq {
            key: key.clone(),
            body: body.clone(),
            content_type: "application/octet-stream".into(),
            cache_control: None,
            metadata: HashMap::new(),
        })
        .await
        .expect("put");
    let got = backend
        .get(GetReq {
            key: key.clone(),
            ..Default::default()
        })
        .await
        .expect("get");
    assert_eq!(got.body, body);
    backend
        .delete(storage::backend::DeleteReq {
            key,
            version_id: None,
        })
        .await
        .expect("delete");
}
