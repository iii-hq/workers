//! Real R2 round-trip. Gated on R2_ACCOUNT_ID + R2_ACCESS_KEY_ID + R2_SECRET_ACCESS_KEY + R2_TEST_BUCKET.

use storage::backend::{factory, GetReq, PutReq};
use storage::config::{BucketConfig, ProvidersConfig, R2BucketConfig};
use std::collections::HashMap;

#[tokio::test]
#[ignore = "opt-in: set R2_ACCOUNT_ID, R2_ACCESS_KEY_ID, R2_SECRET_ACCESS_KEY, R2_TEST_BUCKET"]
async fn r2_round_trip() {
    let (Ok(account), Ok(ak), Ok(sk), Ok(bucket)) = (
        std::env::var("R2_ACCOUNT_ID"),
        std::env::var("R2_ACCESS_KEY_ID"),
        std::env::var("R2_SECRET_ACCESS_KEY"),
        std::env::var("R2_TEST_BUCKET"),
    ) else {
        eprintln!("e2e:r2 — skip: R2 env vars not all set");
        return;
    };
    let cfg = BucketConfig::R2(R2BucketConfig {
        bucket: Some(bucket),
        account_id: account,
        access_key_id: ak,
        secret_access_key: sk,
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
    let body = b"hello-r2".to_vec();
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
