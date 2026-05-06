//! Real AWS S3 round-trip. Gated on AWS_ACCESS_KEY_ID + S3_TEST_BUCKET.

use storage::backend::{factory, GetReq, PutReq};
use storage::config::{BucketConfig, ProvidersConfig, S3BucketConfig};
use std::collections::HashMap;

fn require_env(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.is_empty() => Some(v),
        _ => {
            eprintln!("e2e:s3 — skip: {name} not set");
            None
        }
    }
}

#[tokio::test]
#[ignore = "opt-in: set AWS_ACCESS_KEY_ID, AWS_SECRET_ACCESS_KEY, S3_TEST_BUCKET, S3_TEST_REGION"]
async fn s3_round_trip() {
    let Some(bucket) = require_env("S3_TEST_BUCKET") else {
        return;
    };
    let region = std::env::var("S3_TEST_REGION").unwrap_or_else(|_| "us-east-1".into());
    let cfg = BucketConfig::S3(S3BucketConfig {
        bucket: Some(bucket.clone()),
        region,
        access_key_id: None,
        secret_access_key: None,
        session_token: None,
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
    let body = b"hello-s3".to_vec();
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
